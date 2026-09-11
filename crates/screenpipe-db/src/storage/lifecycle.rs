// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use super::{
    checked_path, durable_json, storage_error, sync_directory, HybridStorage, PrivacyPolicy,
    Projection, StorageBudget, StorageDescriptor, StorageMode,
};
use crate::DatabaseManager;
use screenpipe_config::DbConfig;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool, TypeInfo, ValueRef};
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parity_includes_domain_payload_columns_and_minimum_rowids() {
        let root = tempfile::tempdir().unwrap();
        let db = DatabaseManager::new(
            root.path().join("db.sqlite").to_str().unwrap(),
            Default::default(),
        )
        .await
        .unwrap();
        db.execute_raw_sql_write("CREATE TABLE receipt_probe(id INTEGER PRIMARY KEY,payload_json TEXT); INSERT INTO receipt_probe VALUES(-9223372036854775808,'before')").await.unwrap();
        let before = table_receipts(&db, None).await.unwrap();
        let before = before.iter().find(|r| r.table == "receipt_probe").unwrap();
        assert_eq!(before.rows, 1);
        db.execute_raw_sql_write("UPDATE receipt_probe SET payload_json='after'")
            .await
            .unwrap();
        let after = table_receipts(&db, None).await.unwrap();
        assert_ne!(
            before.sha256,
            after
                .iter()
                .find(|r| r.table == "receipt_probe")
                .unwrap()
                .sha256
        );
        db.close().await;
    }
}

#[derive(Debug, Clone, Default)]
pub struct MigrationOptions {
    pub budget: StorageBudget,
    pub privacy: PrivacyPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TableParity {
    pub table: String,
    pub rows: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationReport {
    pub database_id: String,
    pub generation: String,
    pub frames: u64,
    pub tables: Vec<TableParity>,
    pub source_bytes: u64,
    pub index_bytes: u64,
    pub payload_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Building,
    Ready,
    Active,
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Journal {
    format: u32,
    phase: Phase,
    descriptor: StorageDescriptor,
    copied: bool,
    source: Vec<TableParity>,
    report: Option<MigrationReport>,
}

/// Offline conversion owns only the selected logical root. The caller shuts
/// down its recorder before invoking this operation. Source deletion follows
/// a verified reopen of the descriptor-selected candidate.
pub async fn migrate(
    root: &Path,
    config: DbConfig,
    options: MigrationOptions,
) -> Result<MigrationReport, sqlx::Error> {
    let root = root.canonicalize()?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(".storage.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .map_err(|_| storage_error("storage lifecycle is already owned"))?;
    options.budget.validate()?;
    super::inventory::verify_protection(&root)?;
    let journal_path = root.join("storage-migration.json");
    let source_path = root.join("db.sqlite");
    let mut journal: Journal = if journal_path.exists() {
        let journal: Journal =
            serde_json::from_slice(&std::fs::read(&journal_path)?).map_err(storage_error)?;
        if journal.format != 1 {
            return Err(storage_error("unsupported migration journal"));
        }
        journal.descriptor.validate(&root)?;
        journal
    } else {
        if StorageDescriptor::read(&root)?.is_some() {
            return Err(storage_error(
                "root already has an active storage descriptor",
            ));
        }
        if !source_path.is_file() {
            return Err(storage_error("source database is missing"));
        }
        let generation = uuid::Uuid::new_v4().to_string();
        let directory = PathBuf::from("storage").join(&generation);
        let descriptor = StorageDescriptor {
            format: 1,
            mode: StorageMode::HybridParquetV1,
            database_id: uuid::Uuid::new_v4().to_string(),
            generation,
            source_continuity: true,
            index: directory.join("index.sqlite"),
            payloads: directory.join("payloads"),
            capabilities: ["parquet-frame-v1", "contentless-delete-fts5", "durable-wal"]
                .map(String::from)
                .to_vec(),
            budget: options.budget,
            privacy: options.privacy,
        };
        Journal {
            format: 1,
            phase: Phase::Building,
            descriptor,
            copied: false,
            source: Vec::new(),
            report: None,
        }
    };
    if let Some(active) = StorageDescriptor::read(&root)? {
        if active != journal.descriptor {
            return Err(storage_error(
                "active descriptor differs from migration candidate",
            ));
        }
        journal.phase = Phase::Active;
    }
    let index = checked_path(&root, &journal.descriptor.index)?;
    if journal.phase == Phase::Building || journal.phase == Phase::Ready {
        // This explicit physical opener is the migration's source owner. It
        // neither follows nor overwrites the candidate descriptor.
        let source = DatabaseManager::new_with_storage(
            source_path
                .to_str()
                .ok_or_else(|| storage_error("non-UTF8 database path"))?,
            config.clone(),
            None,
            false,
            false,
        )
        .await?;
        let result = async {
            let frozen = source.begin_immediate_with_retry().await?;
            durable_json(&journal_path, &journal)?;
            tracing::info!(phase = "source_integrity", "storage migration");
            verify_integrity(&source.pool).await?;
            tracing::info!(phase = "source_parity_receipts", "storage migration");
            let source_receipts = table_receipts(&source, None).await?;
            if !journal.source.is_empty() && journal.source != source_receipts {
                return Err(storage_error(
                    "frozen source changed; candidate verification requires a fresh migration",
                ));
            }
            journal.source = source_receipts;
            durable_json(&journal_path, &journal)?;
            if !journal.copied {
                let needed = std::fs::metadata(&source_path)?.len()
                    + journal.descriptor.budget.disk_reserve_bytes;
                if fs2::available_space(&root)? < needed {
                    return Err(storage_error("insufficient migration disk reserve"));
                }
                std::fs::create_dir_all(index.parent().unwrap())?;
                // This path belongs to an incomplete backup recorded by this journal.
                for path in [
                    &index,
                    &PathBuf::from(format!("{}-wal", index.display())),
                    &PathBuf::from(format!("{}-shm", index.display())),
                ] {
                    if path.exists() {
                        std::fs::remove_file(path)?;
                    }
                }
                tracing::info!(phase = "sqlite_copy", "storage migration");
                copy_sqlite(
                    source.pool.clone(),
                    index.clone(),
                    journal.descriptor.budget.lifecycle_timeout_secs,
                )
                .await?;
                super::faults::checkpoint("migration_copied");
                journal.copied = true;
                durable_json(&journal_path, &journal)?;
            }
            let storage = HybridStorage::new(root.clone(), journal.descriptor.clone())?;
            let catalog_exists = {
                use sqlx::Connection;
                let mut conn =
                    sqlx::SqliteConnection::connect(&format!("sqlite:{}?mode=ro", index.display()))
                        .await?;
                let n: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM sqlite_master WHERE name='storage_metadata'",
                )
                .fetch_one(&mut conn)
                .await?;
                conn.close().await?;
                n != 0
            };
            let candidate = DatabaseManager::new_with_storage(
                index.to_str().unwrap(),
                config.clone(),
                Some(storage),
                !catalog_exists,
                false,
            )
            .await?;
            let candidate_result = async {
                tracing::info!(phase = "sealing", "storage migration");
                loop {
                    if candidate.seal_frame_payloads().await? == 0 {
                        break;
                    }
                }
                let pending: i64 =
                    sqlx::query_scalar("SELECT count(*) FROM frame_payloads WHERE state='staged'")
                        .fetch_one(&candidate.pool)
                        .await?;
                if pending != 0 {
                    return Err(storage_error(
                        "migration awaits required PII completion before sealing",
                    ));
                }
                verify_integrity(&candidate.pool).await?;
                tracing::info!(phase = "candidate_parity_receipts", "storage migration");
                let tables = table_receipts(&candidate, Some(&journal.source)).await?;
                if tables != journal.source {
                    return Err(storage_error("migration logical-record parity failed"));
                }
                verify_search_parity(&source, &candidate).await?;
                super::validation::verify_typed_parity(&source, &candidate).await?;
                Ok::<_, sqlx::Error>(tables)
            }
            .await;
            candidate.close().await;
            let tables = candidate_result?;
            compact_candidate(&index, &journal.descriptor.budget).await?;
            // The regular opener checks identity, schema, and catalog before ready.
            let reopened = DatabaseManager::new_with_storage(
                index.to_str().unwrap(),
                config.clone(),
                Some(HybridStorage::new(
                    root.clone(),
                    journal.descriptor.clone(),
                )?),
                false,
                false,
            )
            .await?;
            let verification = async {
                verify_integrity(&reopened.pool).await?;
                let ids: Vec<i64> =
                    sqlx::query_scalar("SELECT id FROM frames ORDER BY id DESC LIMIT 32")
                        .fetch_all(&reopened.pool)
                        .await?;
                reopened.frame_payloads(&ids, Projection::All).await?;
                Ok::<_, sqlx::Error>(())
            }
            .await;
            reopened.close().await;
            verification?;
            let frames = tables
                .iter()
                .find(|t| t.table == "frames")
                .map(|t| t.rows)
                .unwrap_or(0);
            journal.report = Some(MigrationReport {
                database_id: journal.descriptor.database_id.clone(),
                generation: journal.descriptor.generation.clone(),
                frames,
                tables,
                source_bytes: std::fs::metadata(&source_path)?.len(),
                index_bytes: std::fs::metadata(&index)?.len(),
                payload_bytes: directory_bytes(&root.join(&journal.descriptor.payloads))?,
            });
            journal.phase = Phase::Ready;
            durable_json(&journal_path, &journal)?;
            super::faults::checkpoint("migration_ready");
            durable_json(&root.join("storage.json"), &journal.descriptor)?;
            super::faults::checkpoint("migration_activated");
            journal.phase = Phase::Active;
            durable_json(&journal_path, &journal)?;
            frozen.rollback().await?;
            Ok::<_, sqlx::Error>(())
        }
        .await;
        source.close().await;
        result?;
    }
    let reopened = DatabaseManager::new(root.join("db.sqlite").to_str().unwrap(), config).await?;
    let verified = verify_integrity(&reopened.pool).await;
    reopened.close().await;
    verified?;
    for suffix in ["", "-wal", "-shm"] {
        let path = PathBuf::from(format!("{}{suffix}", source_path.display()));
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    sync_directory(&root)?;
    super::faults::checkpoint("migration_reclaimed");
    journal.phase = Phase::Complete;
    durable_json(&journal_path, &journal)?;
    let report = journal
        .report
        .ok_or_else(|| storage_error("migration verification receipt is missing"))?;
    durable_json(&root.join("storage-migration-complete.json"), &report)?;
    std::fs::remove_file(&journal_path)?;
    sync_directory(&root)?;
    Ok(report)
}

/// Cancel an unactivated conversion while retaining the complete source.
pub async fn cancel_migration(root: &Path, config: DbConfig) -> Result<(), sqlx::Error> {
    let root = root.canonicalize()?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(".storage.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .map_err(|_| storage_error("storage lifecycle is already owned"))?;
    if StorageDescriptor::read(&root)?.is_some() {
        return Err(storage_error("activated storage remains authoritative"));
    }
    let journal_path = root.join("storage-migration.json");
    let journal: Journal =
        serde_json::from_slice(&std::fs::read(&journal_path)?).map_err(storage_error)?;
    if journal.format != 1 || !matches!(journal.phase, Phase::Building | Phase::Ready) {
        return Err(storage_error("migration cannot be cancelled in this phase"));
    }
    journal.descriptor.validate(&root)?;
    let source_path = root.join("db.sqlite");
    if !source_path.is_file() {
        return Err(storage_error("migration source is missing"));
    }
    let source = DatabaseManager::new_with_storage(
        source_path
            .to_str()
            .ok_or_else(|| storage_error("non-UTF8 source path"))?,
        config,
        None,
        false,
        false,
    )
    .await?;
    let verified = verify_integrity(&source.pool).await;
    source.close().await;
    verified?;
    let generation = checked_path(&root, &journal.descriptor.index)?
        .parent()
        .unwrap()
        .to_path_buf();
    if generation.exists() {
        std::fs::remove_dir_all(&generation)?;
    }
    if root.join("storage").exists() {
        sync_directory(&root.join("storage"))?;
    }
    std::fs::remove_file(journal_path)?;
    sync_directory(&root)
}

pub(super) async fn verify_integrity(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let checks: Vec<String> = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_all(pool)
        .await?;
    if checks != ["ok"] {
        return Err(storage_error("SQLite integrity verification failed"));
    }
    if !sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(pool)
        .await?
        .is_empty()
    {
        return Err(storage_error("SQLite foreign-key verification failed"));
    }
    Ok(())
}

/// Compact a closed, inactive candidate with one destination-sized scratch
/// file. Closing its manager first releases the bootstrap WAL allocation.
pub(super) async fn compact_candidate(
    index: &Path,
    budget: &super::StorageBudget,
) -> Result<(), sqlx::Error> {
    tracing::info!(phase = "compact_candidate", "storage migration");
    let compact = index.with_extension("compacting.sqlite");
    if compact.exists() {
        std::fs::remove_file(&compact)?;
    }
    let mut conn =
        sqlx::SqliteConnection::connect(&format!("sqlite:{}?mode=ro", index.display())).await?;
    let pages: i64 = sqlx::query_scalar("PRAGMA page_count")
        .fetch_one(&mut conn)
        .await?;
    let free: i64 = sqlx::query_scalar("PRAGMA freelist_count")
        .fetch_one(&mut conn)
        .await?;
    let page_size: i64 = sqlx::query_scalar("PRAGMA page_size")
        .fetch_one(&mut conn)
        .await?;
    let needed = ((pages - free) as u64)
        .saturating_mul(page_size as u64)
        .saturating_add(budget.disk_reserve_bytes);
    if fs2::available_space(index.parent().unwrap())? < needed {
        conn.close().await?;
        return Err(storage_error("insufficient compact scratch reserve"));
    }
    let vacuum = sqlx::query("VACUUM INTO ?")
        .bind(
            compact
                .to_str()
                .ok_or_else(|| storage_error("non-UTF8 compact path"))?,
        )
        .execute(&mut conn)
        .await;
    conn.close().await?;
    vacuum?;
    std::fs::File::open(&compact)?.sync_all()?;
    super::faults::checkpoint("candidate_compacted");
    std::fs::rename(&compact, index)?;
    sync_directory(index.parent().unwrap())
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn hash_value(
    hash: &mut Sha256,
    row: &sqlx::sqlite::SqliteRow,
    index: usize,
    replacement: Option<&Option<String>>,
) -> Result<(), sqlx::Error> {
    if let Some(value) = replacement {
        if let Some(value) = value {
            hash.update(b"T");
            hash.update((value.len() as u64).to_le_bytes());
            hash.update(value.as_bytes());
        } else {
            hash.update(b"N");
        }
        return Ok(());
    }
    let raw = row.try_get_raw(index)?;
    if raw.is_null() {
        hash.update(b"N");
        return Ok(());
    }
    match raw.type_info().name() {
        "INTEGER" => {
            hash.update(b"I");
            hash.update(row.try_get::<i64, _>(index)?.to_le_bytes());
        }
        "REAL" => {
            hash.update(b"R");
            hash.update(row.try_get::<f64, _>(index)?.to_bits().to_le_bytes());
        }
        "TEXT" => {
            let bytes: Vec<u8> = row.try_get(index)?;
            hash.update(b"T");
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        }
        _ => {
            let bytes: Vec<u8> = row.try_get(index)?;
            hash.update(b"B");
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        }
    }
    Ok(())
}

pub(super) async fn table_receipts(
    db: &DatabaseManager,
    source: Option<&[TableParity]>,
) -> Result<Vec<TableParity>, sqlx::Error> {
    let tables: Vec<String> = if let Some(source) = source {
        source.iter().map(|t| t.table.clone()).collect()
    } else {
        sqlx::query_scalar("SELECT m.name FROM sqlite_master m WHERE m.type='table' AND (m.name NOT LIKE 'sqlite_%' OR m.name='sqlite_sequence') AND m.name NOT LIKE '%_fts%' AND m.name NOT LIKE '%_fts5%' AND m.sql NOT LIKE 'CREATE VIRTUAL TABLE%' ORDER BY m.name")
            .fetch_all(&db.pool).await?
    };
    let mut result = Vec::new();
    for table in tables {
        let mut columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info(?) ORDER BY cid")
                .bind(&table)
                .fetch_all(&db.pool)
                .await?;
        if table == "frames" && db.storage.is_some() {
            columns.retain(|name| {
                !matches!(
                    name.as_str(),
                    "payload_full_text_length"
                        | "payload_accessibility_length"
                        | "payload_full_text_present"
                        | "payload_accessibility_present"
                        | "payload_detail_present"
                )
            });
        }
        let mut hash = Sha256::new();
        let mut count = 0;
        let mut last = i64::MIN;
        let mut first = true;
        loop {
            let sql = format!(
                "SELECT rowid AS __storage_rowid,{} FROM {} WHERE rowid{}? ORDER BY rowid LIMIT 128",
                columns
                    .iter()
                    .map(|c| quote(c))
                    .collect::<Vec<_>>()
                    .join(","),
                quote(&table),
                if first {">="} else {">"}
            );
            let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
                .bind(last)
                .fetch_all(&db.pool)
                .await?;
            if rows.is_empty() {
                break;
            }
            first = false;
            let payloads = if table == "frames" && db.storage.is_some() {
                let ids: Vec<i64> = rows.iter().map(|r| r.get("id")).collect();
                db.frame_payloads(&ids, Projection::All).await?
            } else {
                Default::default()
            };
            for row in rows {
                last = row.try_get("__storage_rowid")?;
                let payload = payloads.get(&last);
                for (index, column) in columns.iter().enumerate() {
                    let value = payload.and_then(|p| match column.as_str() {
                        "full_text" => Some(&p.full_text),
                        "accessibility_text" => Some(&p.accessibility_text),
                        "accessibility_tree_json" => Some(&p.accessibility_tree_json),
                        "text_json" => Some(&p.text_json),
                        _ => None,
                    });
                    hash_value(&mut hash, &row, index + 1, value)?;
                }
                hash.update(b"E");
                count += 1;
            }
        }
        tracing::info!(table=%table, rows=count, "storage parity receipt complete");
        result.push(TableParity {
            table,
            rows: count,
            sha256: format!("{:x}", hash.finalize()),
        });
    }
    Ok(result)
}

async fn verify_search_parity(
    source: &DatabaseManager,
    candidate: &DatabaseManager,
) -> Result<(), sqlx::Error> {
    // FTS vocabulary is derived from the current indexed source, including
    // punctuation and Unicode. Compare exact ordered IDs for each sampled term.
    let texts: Vec<String> = sqlx::query_scalar(
        "SELECT full_text FROM frames WHERE full_text IS NOT NULL ORDER BY id LIMIT 32",
    )
    .fetch_all(&source.pool)
    .await?;
    for term in texts
        .iter()
        .flat_map(|t| t.split_whitespace().take(2))
        .filter(|t| t.len() > 2)
    {
        let query = format!("\"{}\"", term.replace('"', "\"\""));
        let sql="SELECT frames.id FROM frames JOIN frames_fts ON frames_fts.rowid=frames.id WHERE frames_fts MATCH ? ORDER BY frames.timestamp DESC,frames.id DESC";
        let a: Vec<i64> = sqlx::query_scalar(sql)
            .bind(&query)
            .fetch_all(&source.pool)
            .await?;
        let b: Vec<i64> = sqlx::query_scalar(sql)
            .bind(&query)
            .fetch_all(&candidate.pool)
            .await?;
        if a != b {
            return Err(storage_error("migration indexed-search parity failed"));
        }
    }
    Ok(())
}

fn directory_bytes(path: &Path) -> Result<u64, sqlx::Error> {
    if !path.exists() {
        return Ok(0);
    }
    let mut size = 0;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        size += if meta.is_dir() {
            directory_bytes(&entry.path())?
        } else {
            meta.len()
        };
    }
    Ok(size)
}

/// Owns the source connection and snapshot on the blocking thread, including
/// when its async caller is cancelled. No borrowed SQLite handle outlives it.
pub(super) async fn copy_sqlite(
    pool: SqlitePool,
    destination: PathBuf,
    timeout_secs: u64,
) -> Result<(), sqlx::Error> {
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(async move {
            let mut conn = pool.acquire().await?;
            let mut tx = conn.begin().await?;
            sqlx::query("SELECT count(*) FROM sqlite_master")
                .fetch_one(&mut *tx)
                .await?;
            let mut locked = tx.lock_handle().await?;
            let name = std::ffi::CString::new(destination.to_string_lossy().as_bytes())
                .map_err(storage_error)?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
            // SAFETY: the source is exclusively held by LockedSqliteHandle;
            // this thread owns the destination and backup until both close.
            let result = unsafe {
                let mut dest = std::ptr::null_mut();
                let rc = libsqlite3_sys::sqlite3_open_v2(
                    name.as_ptr(),
                    &mut dest,
                    libsqlite3_sys::SQLITE_OPEN_READWRITE | libsqlite3_sys::SQLITE_OPEN_CREATE,
                    std::ptr::null(),
                );
                if rc != libsqlite3_sys::SQLITE_OK {
                    if !dest.is_null() {
                        libsqlite3_sys::sqlite3_close(dest);
                    }
                    return Err(storage_error("cannot open backup destination"));
                }
                let backup = libsqlite3_sys::sqlite3_backup_init(
                    dest,
                    c"main".as_ptr(),
                    locked.as_raw_handle().as_ptr(),
                    c"main".as_ptr(),
                );
                if backup.is_null() {
                    libsqlite3_sys::sqlite3_close(dest);
                    return Err(storage_error("cannot initialize SQLite backup"));
                }
                let rc = loop {
                    let rc = libsqlite3_sys::sqlite3_backup_step(backup, 256);
                    if rc != libsqlite3_sys::SQLITE_OK {
                        break rc;
                    }
                    if std::time::Instant::now() >= deadline {
                        break libsqlite3_sys::SQLITE_INTERRUPT;
                    }
                    std::thread::yield_now();
                };
                let finish = libsqlite3_sys::sqlite3_backup_finish(backup);
                let close = libsqlite3_sys::sqlite3_close(dest);
                if rc == libsqlite3_sys::SQLITE_DONE && finish == 0 && close == 0 {
                    Ok(())
                } else {
                    Err(storage_error(format!(
                        "SQLite backup failed ({rc}/{finish}/{close})"
                    )))
                }
            };
            drop(locked);
            tx.commit().await?;
            result?;
            std::fs::File::open(&destination)?.sync_all()?;
            sync_directory(destination.parent().unwrap())?;
            Ok(())
        })
    })
    .await
    .map_err(storage_error)?
}

use sqlx::Connection;

impl DatabaseManager {
    pub async fn verify_storage(&self) -> Result<(), sqlx::Error> {
        verify_integrity(&self.pool).await?;
        if let Some(storage) = &self.storage {
            storage.verify_catalog(&self.pool).await?;
            let mut after = i64::MIN;
            loop {
                let ids: Vec<i64> =
                    sqlx::query_scalar("SELECT id FROM frames WHERE id>? ORDER BY id LIMIT 128")
                        .bind(after)
                        .fetch_all(&self.pool)
                        .await?;
                let Some(last) = ids.last() else {
                    break;
                };
                after = *last;
                self.frame_payloads(&ids, Projection::All).await?;
            }
        }
        Ok(())
    }

    /// Explicit opt-in for an independent empty logical database root.
    pub async fn new_hybrid(
        root: &Path,
        config: DbConfig,
        options: MigrationOptions,
    ) -> Result<Self, sqlx::Error> {
        std::fs::create_dir_all(root)?;
        let root = root.canonicalize()?;
        super::inventory::verify_protection(&root)?;
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join(".storage.lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .map_err(|_| storage_error("storage lifecycle is already owned"))?;
        let initialization = root.join("storage-init.json");
        if let Some(active) = StorageDescriptor::read(&root)? {
            let expected: StorageDescriptor = serde_json::from_slice(
                &std::fs::read(&initialization)
                    .map_err(|_| storage_error("root already has an active storage descriptor"))?,
            )
            .map_err(storage_error)?;
            if active != expected {
                return Err(storage_error("initialization descriptor mismatch"));
            }
            std::fs::remove_file(&initialization)?;
            sync_directory(&root)?;
            return Self::new(root.join("db.sqlite").to_str().unwrap(), config).await;
        }
        let descriptor = if initialization.exists() {
            serde_json::from_slice(&std::fs::read(&initialization)?).map_err(storage_error)?
        } else {
            if root.join("db.sqlite").exists() || root.join("storage-migration.json").exists() {
                return Err(storage_error(
                    "fresh hybrid initialization requires an empty database root",
                ));
            }
            let generation = uuid::Uuid::new_v4().to_string();
            let directory = PathBuf::from("storage").join(&generation);
            let descriptor = StorageDescriptor {
                format: 1,
                mode: StorageMode::HybridParquetV1,
                database_id: uuid::Uuid::new_v4().to_string(),
                generation,
                source_continuity: false,
                index: directory.join("index.sqlite"),
                payloads: directory.join("payloads"),
                capabilities: ["parquet-frame-v1", "contentless-delete-fts5", "durable-wal"]
                    .map(String::from)
                    .to_vec(),
                budget: options.budget,
                privacy: options.privacy,
            };
            durable_json(&initialization, &descriptor)?;
            descriptor
        };
        descriptor.validate(&root)?;
        std::fs::create_dir_all(root.join(&descriptor.payloads))?;
        let index = root.join(&descriptor.index);
        let catalog_exists = if index.exists() {
            let mut conn =
                sqlx::SqliteConnection::connect(&format!("sqlite:{}?mode=ro", index.display()))
                    .await?;
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM sqlite_master WHERE name='storage_metadata'",
            )
            .fetch_one(&mut conn)
            .await?;
            conn.close().await?;
            count != 0
        } else {
            false
        };
        let candidate = Self::new_with_storage(
            index.to_str().unwrap(),
            config.clone(),
            Some(HybridStorage::new(root.clone(), descriptor.clone())?),
            !catalog_exists,
            false,
        )
        .await?;
        let verified = verify_integrity(&candidate.pool).await;
        candidate.close().await;
        verified?;
        sync_directory(index.parent().unwrap())?;
        sync_directory(&root.join("storage"))?;
        super::faults::checkpoint("initialization_ready");
        durable_json(&root.join("storage.json"), &descriptor)?;
        super::faults::checkpoint("initialization_activated");
        std::fs::remove_file(&initialization)?;
        sync_directory(&root)?;
        Self::new(root.join("db.sqlite").to_str().unwrap(), config).await
    }
}
