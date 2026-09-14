// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use super::{
    checked_path, durable_json, lifecycle, storage_error, sync_directory, HybridStorage,
    Projection, StorageDescriptor,
};
use crate::DatabaseManager;
use screenpipe_config::DbConfig;
use serde::{Deserialize, Serialize};
use sqlx::Connection;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompactJournal {
    source: StorageDescriptor,
    candidate: StorageDescriptor,
}

async fn open(
    root: &Path,
    descriptor: &StorageDescriptor,
    config: DbConfig,
) -> Result<DatabaseManager, sqlx::Error> {
    DatabaseManager::new_with_storage(
        checked_path(root, &descriptor.index)?
            .to_str()
            .ok_or_else(|| storage_error("non-UTF8 database path"))?,
        config,
        Some(HybridStorage::new(root.to_path_buf(), descriptor.clone())?),
        false,
        false,
    )
    .await
}

/// Rebuild only an inactive candidate, streaming the authoritative projections.
async fn rebuild_fts(db: &DatabaseManager) -> Result<(), sqlx::Error> {
    db.execute_raw_sql_write(&format!(
        "DROP TABLE frames_fts; {};",
        super::schema::hybrid_fts_schema()
    ))
    .await?;
    let mut after = i64::MIN;
    loop {
        let ids: Vec<i64> =
            sqlx::query_scalar("SELECT id FROM frames WHERE id>? ORDER BY id LIMIT 128")
                .bind(after)
                .fetch_all(&db.pool)
                .await?;
        let Some(last) = ids.last() else {
            break;
        };
        after = *last;
        let payloads = db.frame_payloads(&ids, Projection::Search).await?;
        let mut tx = db.begin_immediate_with_retry().await?;
        for payload in payloads.values() {
            if let Some(text) = payload.full_text.as_deref().filter(|s| !s.is_empty()) {
                sqlx::query("INSERT INTO frames_fts(rowid,full_text,app_name,window_name,browser_url) SELECT id,?,COALESCE(app_name,''),COALESCE(window_name,''),COALESCE(browser_url,'') FROM frames WHERE id=?")
                    .bind(text).bind(payload.id).execute(&mut **tx.conn()).await?;
            }
        }
        tx.commit().await?;
    }
    Ok(())
}

async fn update_descriptor(
    index: &Path,
    descriptor: &StorageDescriptor,
) -> Result<(), sqlx::Error> {
    let mut conn = sqlx::SqliteConnection::connect(&format!("sqlite:{}", index.display())).await?;
    sqlx::query("PRAGMA synchronous=FULL")
        .execute(&mut conn)
        .await?;
    sqlx::query("UPDATE storage_metadata SET descriptor=?")
        .bind(serde_json::to_string(descriptor).map_err(storage_error)?)
        .execute(&mut conn)
        .await?;
    conn.close().await?;
    super::sync_file(index)?;
    Ok(())
}

/// Exclusive offline index reclamation preserves payload/media paths. The
/// durable descriptor replacement is the only activation point.
pub async fn compact(root: &Path, config: DbConfig) -> Result<(), sqlx::Error> {
    let root = root.canonicalize()?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(".storage.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .map_err(|_| storage_error("storage lifecycle is already owned"))?;
    let journal_path = root.join("storage-maintenance.json");
    let active = StorageDescriptor::read(&root)?
        .ok_or_else(|| storage_error("compact requires hybrid storage"))?;
    let journal: CompactJournal = if journal_path.exists() {
        serde_json::from_slice(&std::fs::read(&journal_path)?).map_err(storage_error)?
    } else {
        let mut candidate = active.clone();
        candidate.generation = uuid::Uuid::new_v4().to_string();
        candidate.index = PathBuf::from("storage")
            .join(&candidate.generation)
            .join("index.sqlite");
        let journal = CompactJournal {
            source: active.clone(),
            candidate,
        };
        durable_json(&journal_path, &journal)?;
        journal
    };
    journal.source.validate(&root)?;
    journal.candidate.validate(&root)?;
    if active != journal.source && active != journal.candidate {
        return Err(storage_error("maintenance descriptor mismatch"));
    }
    if active == journal.source {
        let source = open(&root, &journal.source, config.clone()).await?;
        let build = async {
            let index = checked_path(&root, &journal.candidate.index)?;
            if fs2::available_space(&root)?
                < std::fs::metadata(root.join(&journal.source.index))?
                    .len()
                    .saturating_mul(2)
                    + journal.source.budget.disk_reserve_bytes
            {
                return Err(storage_error("insufficient compact disk reserve"));
            }
            std::fs::create_dir_all(index.parent().unwrap())?;
            remove_index(&index)?;
            lifecycle::copy_sqlite(
                source.pool.clone(),
                index.clone(),
                journal.source.budget.lifecycle_timeout_secs,
            )
            .await?;
            update_descriptor(&index, &journal.candidate).await?;
            let candidate = open(&root, &journal.candidate, config.clone()).await?;
            let verification = async {
                rebuild_fts(&candidate).await?;
                candidate.verify_storage().await?;
                let mut expected = lifecycle::table_receipts(&source, None).await?;
                expected.retain(|table| table.table != "storage_metadata");
                if lifecycle::table_receipts(&candidate, Some(&expected)).await? != expected {
                    return Err(storage_error("compact logical parity failed"));
                }
                Ok::<_, sqlx::Error>(())
            }
            .await;
            candidate.close().await;
            verification?;
            lifecycle::compact_candidate(&index, candidate.storage.as_ref().unwrap()).await?;
            sync_directory(index.parent().unwrap())?;
            sync_directory(&root.join("storage"))?;
            super::faults::checkpoint("compact_ready");
            durable_json(&root.join("storage.json"), &journal.candidate)?;
            super::faults::checkpoint("compact_activated");
            Ok::<_, sqlx::Error>(())
        }
        .await;
        source.close().await;
        build?;
    }
    let candidate = open(&root, &journal.candidate, config).await?;
    let verified = candidate.verify_storage().await;
    candidate.close().await;
    verified?;
    remove_index(&checked_path(&root, &journal.source.index)?)?;
    sync_directory(root.join(&journal.source.index).parent().unwrap())?;
    durable_json(&root.join("storage-reclamation-complete.json"), &journal)?;
    std::fs::remove_file(&journal_path)?;
    sync_directory(&root)
}

fn remove_index(index: &Path) -> Result<(), sqlx::Error> {
    for suffix in ["", "-wal", "-shm"] {
        match std::fs::remove_file(PathBuf::from(format!("{}{suffix}", index.display()))) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

/// Materialize a separate, standalone SQLite database for legacy readers.
pub async fn export_sqlite(
    root: &Path,
    destination: &Path,
    config: DbConfig,
) -> Result<(), sqlx::Error> {
    if destination.exists() {
        return Err(storage_error("export destination already exists"));
    }
    let root = root.canonicalize()?;
    let descriptor = StorageDescriptor::read(&root)?
        .ok_or_else(|| storage_error("legacy export requires hybrid storage"))?;
    let source = open(&root, &descriptor, config.clone()).await?;
    let result=async {
        let temporary=tempfile::Builder::new().prefix(".screenpipe-export-").tempdir_in(destination.parent().ok_or_else(||storage_error("export parent missing"))?)?;
        let index=temporary.path().join("export.sqlite");
        lifecycle::copy_sqlite(source.pool.clone(),index.clone(),descriptor.budget.lifecycle_timeout_secs).await?;
        let mut output=sqlx::SqliteConnection::connect(&format!("sqlite:{}",index.display())).await?;
        let triggers:Vec<String>=sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='trigger' AND name LIKE 'hybrid_%'").fetch_all(&mut output).await?;
        for name in triggers {sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP TRIGGER \"{}\"",name.replace('"',"\"\"")))).execute(&mut output).await?;}
        sqlx::raw_sql("DROP VIEW IF EXISTS _bulk_element_search; DROP TABLE IF EXISTS _bulk_element_lookup; DROP TABLE IF EXISTS _storage_revocation;").execute(&mut output).await?;
        super::bulk::export(&source,&mut output).await?;
        sqlx::raw_sql("DROP TABLE frames_fts; DROP TABLE frame_payloads; DROP TABLE payload_files; DROP TABLE upload_bindings; DROP TABLE storage_metadata; DROP TABLE _hybrid_migrations;").execute(&mut output).await?;
        // A standalone export starts a new conversion if imported again. Its
        // old completion checkpoints must not skip the new catalogs/backfills.
        sqlx::raw_sql("DROP TABLE IF EXISTS _storage_conversion_steps; DROP TABLE IF EXISTS _storage_conversion_triggers; DROP TABLE IF EXISTS _storage_conversion_schema;").execute(&mut output).await?;
        // Hybrid construction adds these derived columns to the legacy schema.
        // Remove them from the private export before restoring bulky payloads,
        // so a later migration can construct its own catalog without collisions.
        sqlx::raw_sql(
            "ALTER TABLE frames DROP COLUMN payload_full_text_length;
             ALTER TABLE frames DROP COLUMN payload_accessibility_length;
             ALTER TABLE frames DROP COLUMN payload_full_text_present;
             ALTER TABLE frames DROP COLUMN payload_accessibility_present;
             ALTER TABLE frames DROP COLUMN payload_detail_present;",
        )
        .execute(&mut output)
        .await?;
        let mut after=i64::MIN;
        loop {
            let ids:Vec<i64>=sqlx::query_scalar("SELECT id FROM frames WHERE id>? ORDER BY id LIMIT 128").bind(after).fetch_all(&source.pool).await?;
            let Some(last)=ids.last() else {break;}; after=*last;
            let payloads=source.frame_payloads(&ids,Projection::All).await?;
            let mut tx=output.begin().await?;
            for payload in payloads.values() {
                sqlx::query("UPDATE frames SET full_text=?,accessibility_text=?,accessibility_tree_json=?,text_json=? WHERE id=?")
                    .bind(&payload.full_text).bind(&payload.accessibility_text).bind(&payload.accessibility_tree_json).bind(&payload.text_json).bind(payload.id)
                    .execute(&mut *tx).await?;
            }
            tx.commit().await?;
        }
        sqlx::raw_sql(super::schema::LEGACY_FTS).execute(&mut output).await?;
        sqlx::query("VACUUM").execute(&mut output).await?;
        output.close().await?;
        let exported=DatabaseManager::new(index.to_str().unwrap(),config).await?;
        let verified=exported.verify_storage().await;
        exported.close().await; verified?;
        super::sync_file(&index)?;
        std::fs::rename(&index,destination)?;
        sync_directory(destination.parent().unwrap())
    }.await;
    source.close().await;
    result
}
