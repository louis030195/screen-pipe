// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use super::{
    checked_path, codec, durable_json, lifecycle, storage_error, sync_directory, StorageDescriptor,
};
use crate::DatabaseManager;
use serde::{Deserialize, Serialize};
use sqlx::{Connection, Row};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
struct Entry {
    path: PathBuf,
    bytes: u64,
    sha256: String,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    format: u32,
    descriptor: StorageDescriptor,
    files: Vec<Entry>,
}

impl DatabaseManager {
    pub(crate) async fn backup_hybrid(&self, destination: &Path) -> Result<(), sqlx::Error> {
        let started = std::time::Instant::now();
        let storage = self
            .storage
            .as_ref()
            .ok_or_else(|| storage_error("hybrid backup requires hybrid storage"))?;
        if destination.exists() {
            return Err(storage_error("backup destination already exists"));
        }
        let parent = destination
            .parent()
            .ok_or_else(|| storage_error("backup parent is missing"))?;
        let temporary = tempfile::Builder::new()
            .prefix(".screenpipe-backup-")
            .tempdir_in(parent)?;
        let directory = temporary.path();
        // Coarse generation pin covers precisely whichever committed files
        // the backup's fixed SQLite snapshot ends up referencing.
        let _pin = self.storage_read_token().await?;
        let index = directory.join(&storage.descriptor.index);
        std::fs::create_dir_all(index.parent().unwrap())?;
        // A sparse in-place index can have a large logical length. The page
        // backup API materializes its holes; VACUUM INTO writes only live pages
        // directly into the requested backup, without an intermediate copy.
        {
            let writer = self.coordinated_writer();
            let permit = writer.lock().await?;
            sqlx::query("VACUUM INTO ?")
                .bind(
                    index
                        .to_str()
                        .ok_or_else(|| storage_error("non-UTF8 backup path"))?,
                )
                .execute(permit.pool())
                .await?;
        }
        std::fs::File::open(&index)?.sync_all()?;
        let mut conn =
            sqlx::SqliteConnection::connect(&format!("sqlite:{}?mode=ro", index.display())).await?;
        let files=sqlx::query("SELECT DISTINCT pf.search_path,pf.detail_path FROM payload_files pf JOIN frame_payloads p ON p.file_id=pf.id ORDER BY pf.id").fetch_all(&mut conn).await?;
        let mut payload_paths = Vec::new();
        if storage.has_bulk() {
            let paths: Vec<String> = sqlx::query_scalar(
                "SELECT path FROM _bulk_files WHERE state IN ('published','dirty')",
            )
            .fetch_all(&mut conn)
            .await?;
            payload_paths.extend(paths.into_iter().map(PathBuf::from));
        }
        conn.close().await?;
        for row in files {
            for column in ["search_path", "detail_path"] {
                payload_paths.push(PathBuf::from(row.try_get::<String, _>(column)?));
            }
        }
        let storage = std::sync::Arc::clone(storage);
        let destination = destination.to_path_buf();
        let parent = parent.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let _pin = _pin;
            let directory = temporary.path();
            let check_deadline = || {
                if started.elapsed().as_secs() >= storage.descriptor.budget.lifecycle_timeout_secs {
                    return Err(storage_error("backup deadline exceeded"));
                }
                Ok(())
            };
            let mut paths = vec![storage.descriptor.index.clone()];
            for relative in payload_paths {
                check_deadline()?;
                let source = storage.payload_path(&relative)?;
                let target = checked_path(directory, &relative)?;
                std::fs::create_dir_all(target.parent().unwrap())?;
                std::fs::copy(source, &target)?;
                std::fs::File::open(&target)?.sync_all()?;
                sync_directory(target.parent().unwrap())?;
                paths.push(relative);
            }
            let mut entries = Vec::new();
            for relative in paths {
                check_deadline()?;
                let path = checked_path(directory, &relative)?;
                entries.push(Entry {
                    path: relative,
                    bytes: std::fs::metadata(&path)?.len(),
                    sha256: codec::checksum(&path)?,
                });
            }
            check_deadline()?;
            durable_json(&directory.join("storage.json"), &storage.descriptor)?;
            durable_json(
                &directory.join("manifest.json"),
                &Manifest {
                    format: 1,
                    descriptor: storage.descriptor.clone(),
                    files: entries,
                },
            )?;
            sync_tree_directories(directory)?;
            std::fs::rename(directory, &destination)?;
            sync_directory(&parent)?;
            Ok(())
        })
        .await
        .map_err(storage_error)?
    }
}

fn sync_tree_directories(path: &Path) -> Result<(), sqlx::Error> {
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            sync_tree_directories(&entry.path())?;
        }
    }
    sync_directory(path)
}

pub async fn restore(
    bundle: &Path,
    destination: &Path,
    config: screenpipe_config::DbConfig,
) -> Result<(), sqlx::Error> {
    if destination.exists() {
        return Err(storage_error("restore destination already exists"));
    }
    let manifest: Manifest = serde_json::from_slice(&std::fs::read(bundle.join("manifest.json"))?)
        .map_err(storage_error)?;
    if manifest.format != 1 {
        return Err(storage_error("unsupported backup manifest"));
    }
    manifest.descriptor.validate(bundle)?;
    if StorageDescriptor::read(bundle)?.as_ref() != Some(&manifest.descriptor) {
        return Err(storage_error("backup descriptor mismatch"));
    }
    if !manifest
        .files
        .iter()
        .any(|e| e.path == manifest.descriptor.index)
    {
        return Err(storage_error("backup index is missing from manifest"));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| storage_error("restore parent is missing"))?;
    super::inventory::verify_protection(parent)?;
    let temp = tempfile::Builder::new()
        .prefix(".screenpipe-restore-")
        .tempdir_in(parent)?;
    let mut seen = std::collections::HashSet::new();
    for entry in &manifest.files {
        if !seen.insert(&entry.path) {
            return Err(storage_error("duplicate backup path"));
        }
        let source = checked_path(bundle, &entry.path)?;
        if std::fs::metadata(&source)?.len() != entry.bytes
            || codec::checksum(&source)? != entry.sha256
        {
            return Err(storage_error("backup checksum mismatch"));
        }
        let target = checked_path(temp.path(), &entry.path)?;
        std::fs::create_dir_all(target.parent().unwrap())?;
        std::fs::copy(source, &target)?;
        std::fs::File::open(&target)?.sync_all()?;
    }
    let mut descriptor = manifest.descriptor.clone();
    descriptor.generation = uuid::Uuid::new_v4().to_string();
    descriptor.index = PathBuf::from("storage")
        .join(&descriptor.generation)
        .join("index.sqlite");
    let new_index = temp.path().join(&descriptor.index);
    std::fs::create_dir_all(new_index.parent().unwrap())?;
    std::fs::rename(temp.path().join(&manifest.descriptor.index), &new_index)?;
    let mut conn =
        sqlx::SqliteConnection::connect(&format!("sqlite:{}", new_index.display())).await?;
    sqlx::query("UPDATE storage_metadata SET descriptor=?")
        .bind(serde_json::to_string(&descriptor).map_err(storage_error)?)
        .execute(&mut conn)
        .await?;
    conn.close().await?;
    std::fs::File::open(&new_index)?.sync_all()?;
    durable_json(&temp.path().join("storage.json"), &descriptor)?;
    let db = DatabaseManager::new(temp.path().join("db.sqlite").to_str().unwrap(), config).await?;
    let verification = async {
        lifecycle::verify_integrity(&db.pool).await?;
        let ids: Vec<i64> =
            sqlx::query_scalar("SELECT frame_id FROM frame_payloads ORDER BY frame_id")
                .fetch_all(&db.pool)
                .await?;
        for batch in ids.chunks(128) {
            db.frame_payloads(batch, super::Projection::All).await?;
        }
        Ok::<_, sqlx::Error>(())
    }
    .await;
    db.close().await;
    verification?;
    sync_tree_directories(temp.path())?;
    std::fs::rename(temp.path(), destination)?;
    sync_directory(parent)
}
