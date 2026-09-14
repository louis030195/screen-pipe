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
    #[serde(default)]
    pub source_identity: Option<RetainedSourceIdentity>,
    #[serde(default)]
    pub allocated_before_bytes: Option<u64>,
    #[serde(default)]
    pub allocated_after_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetainedSourceIdentity {
    bytes: u64,
    modified: std::time::SystemTime,
    #[serde(default)]
    file_id: Option<(u64, u64)>,
}

fn source_identity(root: &Path) -> Result<Option<RetainedSourceIdentity>, sqlx::Error> {
    let path = checked_path(root, Path::new("db.sqlite"))?;
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(Some(RetainedSourceIdentity {
            bytes: metadata.len(),
            modified: metadata.modified()?,
            file_id: {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    Some((metadata.dev(), metadata.ino()))
                }
                #[cfg(not(unix))]
                {
                    None
                }
            },
        })),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Ok(_) => Err(storage_error("original database is not a regular file")),
        Err(error) => Err(error.into()),
    }
}

pub fn migration_report(root: &Path) -> Result<Option<MigrationReport>, sqlx::Error> {
    let path = checked_path(root, Path::new("storage-migration-complete.json"))?;
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(storage_error),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Building,
    Ready,
    Paused,
    Active,
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Journal {
    format: u32,
    phase: Phase,
    descriptor: StorageDescriptor,
    source: Vec<TableParity>,
    #[serde(default)]
    snapshot: Option<RetainedSourceIdentity>,
    report: Option<MigrationReport>,
    #[serde(default)]
    allocated_before_bytes: Option<u64>,
    #[serde(default)]
    search_receipts: Vec<(String, Vec<i64>)>,
}

fn read_journal(root: &Path) -> Result<Journal, sqlx::Error> {
    let path = checked_path(root, Path::new("storage-migration.json"))?;
    let journal: Journal = serde_json::from_slice(&std::fs::read(path)?).map_err(storage_error)?;
    if !matches!(journal.format, 1 | 2) {
        return Err(storage_error("unsupported migration journal"));
    }
    journal.descriptor.validate(root)?;
    Ok(journal)
}

pub(super) fn migration_is_paused(root: &Path) -> Result<bool, sqlx::Error> {
    let journal = read_journal(root)?;
    Ok(journal.phase == Phase::Paused && checked_path(root, Path::new("db.sqlite"))?.is_file())
}

pub fn migration_requires_resume(root: &Path) -> Result<bool, sqlx::Error> {
    if !root.join("storage-migration.json").exists() {
        return Ok(false);
    }
    Ok(read_journal(root)?.format == 2)
}

/// A paused in-place migration has a complete writable resident schema, but
/// retains its journal so archival can only continue after explicit user action.
pub(super) fn migration_recording_ready(root: &Path) -> Result<bool, sqlx::Error> {
    if !root.join("storage-migration.json").exists() {
        return Ok(false);
    }
    let journal = read_journal(root)?;
    if journal.format != 2 || journal.phase != Phase::Paused {
        return Ok(false);
    }
    let active = StorageDescriptor::read(root)?;
    Ok(active.as_ref() == Some(&journal.descriptor)
        || (active.is_none()
            && root.join("db.sqlite").is_file()
            && !root.join(&journal.descriptor.index).exists()))
}

/// Restore recording after an interrupted conversion without retrying archival.
/// Committed payload files are retained byte-for-byte; remaining rows stay in
/// SQLite. Recovery and subsequent recording use the existing single writer.
pub async fn recover_interrupted_migration(
    root: &Path,
    config: DbConfig,
) -> Result<(), sqlx::Error> {
    if !root.join("storage-migration.json").exists() {
        return Ok(());
    }
    if read_journal(root)?.format == 1 {
        return pause_interrupted_migration(root);
    }
    if migration_recording_ready(root)? {
        return Ok(());
    }
    let root = root.canonicalize()?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(".storage.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .map_err(|_| storage_error("storage lifecycle is already owned"))?;
    let mut journal = read_journal(&root)?;
    let index = checked_path(&root, &journal.descriptor.index)?;
    if root.join("db.sqlite").is_file() {
        if index.exists() || StorageDescriptor::read(&root)?.is_some() {
            return Err(storage_error(
                "migration recovery found conflicting database generations",
            ));
        }
        // Nothing was renamed or converted. Keep the original writable, and
        // refresh its verification receipts only on a later explicit retry.
        journal.phase = Phase::Paused;
        durable_json(&root.join("storage-migration.json"), &journal)?;
        return Ok(());
    }
    if !index.is_file() {
        return Err(storage_error("migration index is missing"));
    }
    if StorageDescriptor::read(&root)?.is_some_and(|active| active != journal.descriptor) {
        return Err(storage_error(
            "active descriptor differs from migration index",
        ));
    }
    let storage = HybridStorage::new(root.clone(), journal.descriptor.clone())?;
    super::in_place::recover_recording(storage.clone()).await?;
    let db = DatabaseManager::new_with_storage(
        index.to_str().unwrap(),
        config,
        Some(storage),
        false,
        false,
    )
    .await?;
    let check = sqlx::query("SELECT id FROM frames LIMIT 1")
        .fetch_optional(&db.pool)
        .await;
    db.close().await;
    check?;
    // Journal first: interruption before descriptor publication remains closed;
    // recovery is idempotent and never starts a Parquet conversion batch.
    journal.phase = Phase::Paused;
    durable_json(&root.join("storage-migration.json"), &journal)?;
    super::faults::checkpoint("migration_recovery_ready");
    durable_json(&root.join("storage.json"), &journal.descriptor)?;
    super::faults::checkpoint("migration_recovery_activated");
    tracing::info!("storage recovered for recording; migration remains paused");
    Ok(())
}

/// Reopening the desktop app restores ordinary use of its last active storage.
/// An unpublished candidate is paused; resuming it takes a fresh source snapshot.
pub fn pause_interrupted_migration(root: &Path) -> Result<(), sqlx::Error> {
    if !root.join("storage-migration.json").exists() || root.join("storage.json").exists() {
        return Ok(());
    }
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
        return Ok(());
    }
    let mut journal = read_journal(&root)?;
    if journal.format == 2 {
        return Ok(());
    }
    if !matches!(
        journal.phase,
        Phase::Building | Phase::Ready | Phase::Paused
    ) || !checked_path(&root, Path::new("db.sqlite"))?.is_file()
    {
        return Err(storage_error(
            "interrupted migration has no usable original database",
        ));
    }
    if journal.phase != Phase::Paused {
        journal.phase = Phase::Paused;
        durable_json(&root.join("storage-migration.json"), &journal)?;
    }
    Ok(())
}

/// Offline conversion owns only the selected logical root. The caller shuts
/// down its recorder before invoking this operation. The existing SQLite file
/// becomes the active index, with committed batches recovered on interruption.
pub async fn migrate(
    root: &Path,
    config: DbConfig,
    options: MigrationOptions,
) -> Result<MigrationReport, sqlx::Error> {
    migrate_with_progress(root, config, options, |_| {}).await
}

/// Counts describe conversion only; verification and activation have no known denominator.
#[derive(Clone, Copy, Debug)]
pub struct MigrationProgress {
    pub message: &'static str,
    pub completed_records: Option<u64>,
    pub total_records: Option<u64>,
    pub bytes_saved: Option<u64>,
    pub available_bytes: Option<u64>,
}

impl MigrationProgress {
    fn phase(message: &'static str) -> Self {
        Self {
            message,
            completed_records: None,
            total_records: None,
            bytes_saved: None,
            available_bytes: None,
        }
    }
}

pub async fn migrate_with_progress(
    root: &Path,
    config: DbConfig,
    options: MigrationOptions,
    progress: impl Fn(MigrationProgress) + Send + Sync,
) -> Result<MigrationReport, sqlx::Error> {
    // Old unpublished conversions still own a complete original. Retire only
    // their verified disposable candidate before choosing the new strategy.
    if root.join("storage-migration.json").exists() && read_journal(root)?.format == 1 {
        if StorageDescriptor::read(root)?.is_some() {
            return resume_legacy_migration(root, config, progress).await;
        }
        cancel_migration(root, config.clone()).await?;
    }
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
    let mut journal = if journal_path.exists() {
        read_journal(&root)?
    } else {
        if StorageDescriptor::read(&root)?.is_some() {
            return Err(storage_error("root already has active storage"));
        }
        if !source_path.is_file() {
            return Err(storage_error("source database is missing"));
        }
        progress(MigrationProgress::phase(
            "checking storage before conversion",
        ));
        if fs2::available_space(&root)? < super::in_place::working_space(&options.budget) {
            return Err(storage_error(
                "insufficient migration working space; database unchanged",
            ));
        }
        super::reclaim::probe(&root)?;
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
        let original = async {
            let frozen = source.begin_immediate_with_retry().await?;
            verify_integrity(&source.pool).await?;
            let pages: i64 = sqlx::query_scalar("PRAGMA page_count").fetch_one(&source.pool).await?;
            if (pages as usize).div_ceil(8) > options.budget.decode_bytes / 2 {
                return Err(storage_error("freelist map exceeds migration memory budget; database unchanged"));
            }
            let receipts = table_receipts(&source, None).await?;
            let terms: Vec<String> = sqlx::query_scalar("SELECT full_text FROM frames WHERE full_text IS NOT NULL AND id IN ((SELECT min(id) FROM frames),(SELECT max(id) FROM frames))").fetch_all(&source.pool).await?;
            let mut searches = Vec::new();
            for text in terms {
                if let Some(term) = text.split_whitespace().find(|t| t.len()>2) {
                    let term = format!("\"{}\"",term.replace('"',"\"\""));
                    let ids = sqlx::query_scalar(MIGRATION_SEARCH).bind(&term).fetch_all(&source.pool).await?;
                    searches.push((term,ids));
                }
            }
            frozen.rollback().await?;
            Ok::<_,sqlx::Error>((receipts,searches))
        }.await;
        source.close().await;
        drop(source);
        let (receipts, searches) = original?;
        // Detect external SQLite readers before journaling or moving the file.
        let mut exclusive = sqlx::SqliteConnection::connect_with(
            &sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&source_path)
                .create_if_missing(false)
                .pragma("locking_mode", "EXCLUSIVE")
                .busy_timeout(std::time::Duration::from_secs(5)),
        )
        .await?;
        let check = async {
            sqlx::raw_sql("BEGIN EXCLUSIVE; COMMIT;")
                .execute(&mut exclusive)
                .await?;
            super::schema::construction_checkpoint(&mut exclusive).await
        }
        .await;
        exclusive.close().await?;
        check?;
        let generation = uuid::Uuid::new_v4().to_string();
        let directory = PathBuf::from("storage").join(&generation);
        let journal = Journal {
            format: 2,
            phase: Phase::Building,
            descriptor: StorageDescriptor {
                format: 1,
                mode: StorageMode::HybridParquetV1,
                database_id: uuid::Uuid::new_v4().to_string(),
                generation,
                source_continuity: true,
                index: directory.join("index.sqlite"),
                payloads: directory.join("payloads"),
                capabilities: super::capabilities(),
                budget: options.budget,
                privacy: options.privacy,
            },
            source: receipts,
            snapshot: source_identity(&root)?,
            report: None,
            allocated_before_bytes: Some(super::reclaim::footprint(&root)?),
            search_receipts: searches,
        };
        durable_json(&journal_path, &journal)?;
        super::faults::checkpoint("migration_before_rename");
        journal
    };
    if journal.phase == Phase::Paused {
        // Recording may have appended, edited, or retained history since the
        // failure. The explicit retry verifies that current logical dataset,
        // keeping all already committed archive files and their generations.
        let active = StorageDescriptor::read(&root)?;
        let path = if active.is_some() {
            root.join(&journal.descriptor.index)
        } else {
            source_path.clone()
        };
        let storage = active
            .map(|d| HybridStorage::new(root.clone(), d))
            .transpose()?;
        let db = DatabaseManager::new_with_storage(
            path.to_str().unwrap(),
            config.clone(),
            storage,
            false,
            false,
        )
        .await?;
        let refreshed = async {
            let receipts = table_receipts(&db, Some(&journal.source)).await?;
            let mut searches = Vec::new();
            for (term, _) in &journal.search_receipts {
                let ids = sqlx::query_scalar(MIGRATION_SEARCH)
                    .bind(term)
                    .fetch_all(&db.pool)
                    .await?;
                searches.push((term.clone(), ids));
            }
            Ok::<_, sqlx::Error>((receipts, searches))
        }
        .await;
        db.close().await;
        let (receipts, searches) = refreshed?;
        journal.source = receipts;
        journal.search_receipts = searches;
        journal.snapshot = source_identity(&root)?;
        journal.report = None;
        journal.phase = Phase::Building;
        durable_json(&journal_path, &journal)?;
    }
    if let Some(active) = StorageDescriptor::read(&root)? {
        if active != journal.descriptor {
            return Err(storage_error(
                "active descriptor differs from migration index",
            ));
        }
    }
    let index = checked_path(&root, &journal.descriptor.index)?;
    if source_path.exists() {
        if index.exists() || source_identity(&root)? != journal.snapshot {
            return Err(storage_error(
                "migration source identity changed; both files have been kept",
            ));
        }
        let _source_owner =
            screenpipe_sqlite_coordinator::acquire_sqlite_manager_lease(&source_path)
                .map_err(storage_error)?;
        for suffix in ["-wal", "-journal"] {
            if root.join(format!("db.sqlite{suffix}")).exists() {
                return Err(storage_error(
                    "source has an active SQLite journal; migration cannot move it",
                ));
            }
        }
        std::fs::create_dir_all(index.parent().unwrap())?;
        std::fs::rename(&source_path, &index)?;
        sync_directory(&root)?;
        sync_directory(index.parent().unwrap())?;
        sync_directory(&root.join("storage"))?;
        super::faults::checkpoint("migration_after_rename");
    }
    if !index.is_file() {
        return Err(storage_error("migration index is missing"));
    }
    if journal.phase == Phase::Building {
        let storage = HybridStorage::new(root.clone(), journal.descriptor.clone())?;
        super::in_place::convert(
            storage,
            journal.allocated_before_bytes.unwrap_or(0),
            journal
                .source
                .iter()
                .filter(|t| t.table == "frames" || super::bulk::is_bulk_table(&t.table))
                .map(|t| t.rows)
                .sum(),
            &progress,
        )
        .await?;
        journal.phase = Phase::Ready;
        durable_json(&journal_path, &journal)?;
    }
    if journal.phase == Phase::Ready {
        progress(MigrationProgress::phase("checking storage and search"));
        let db = DatabaseManager::new_with_storage(
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
            verify_integrity(&db.pool).await?;
            db.verify_storage().await?;
            if table_receipts(&db, Some(&journal.source)).await? != journal.source {
                return Err(storage_error(
                    "migration logical data differs; converted data has been kept for diagnosis",
                ));
            }
            for (term, expected) in &journal.search_receipts {
                let actual: Vec<i64> = sqlx::query_scalar(MIGRATION_SEARCH)
                    .bind(term)
                    .fetch_all(&db.pool)
                    .await?;
                if &actual != expected {
                    return Err(storage_error("migration indexed-search parity failed"));
                }
            }
            Ok::<_, sqlx::Error>(())
        }
        .await;
        db.close().await;
        verification?;
        journal.report = Some(MigrationReport {
            database_id: journal.descriptor.database_id.clone(),
            generation: journal.descriptor.generation.clone(),
            frames: journal
                .source
                .iter()
                .find(|t| t.table == "frames")
                .map_or(0, |t| t.rows),
            tables: journal.source.clone(),
            source_bytes: journal.snapshot.as_ref().map_or(0, |s| s.bytes),
            index_bytes: std::fs::metadata(&index)?.len(),
            payload_bytes: directory_bytes(&root.join(&journal.descriptor.payloads))?,
            source_identity: None,
            allocated_before_bytes: journal.allocated_before_bytes,
            allocated_after_bytes: Some(super::reclaim::footprint(&root)?),
        });
        durable_json(&journal_path, &journal)?;
        super::faults::checkpoint("migration_ready");
        durable_json(&root.join("storage.json"), &journal.descriptor)?;
        super::faults::checkpoint("migration_activated");
        journal.phase = Phase::Active;
        durable_json(&journal_path, &journal)?;
    }
    let reopened = DatabaseManager::new_with_storage(
        index.to_str().unwrap(),
        config,
        Some(HybridStorage::new(
            root.clone(),
            journal.descriptor.clone(),
        )?),
        false,
        false,
    )
    .await?;
    let check = sqlx::query("SELECT id FROM frames LIMIT 1")
        .fetch_optional(&reopened.pool)
        .await;
    reopened.close().await;
    check?;
    let report = journal
        .report
        .ok_or_else(|| storage_error("migration report is missing"))?;
    durable_json(&root.join("storage-migration-complete.json"), &report)?;
    super::faults::checkpoint("migration_completed");
    std::fs::remove_file(journal_path)?;
    sync_directory(&root)?;
    Ok(report)
}

const MIGRATION_SEARCH: &str = "SELECT frames.id FROM frames JOIN frames_fts ON frames_fts.rowid=frames.id WHERE frames_fts MATCH ? ORDER BY frames.timestamp DESC,frames.id DESC LIMIT 32";

async fn resume_legacy_migration(
    root: &Path,
    config: DbConfig,
    progress: impl Fn(MigrationProgress) + Send + Sync,
) -> Result<MigrationReport, sqlx::Error> {
    progress(MigrationProgress::phase("preparing migration"));
    let root = root.canonicalize()?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(".storage.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .map_err(|_| storage_error("storage lifecycle is already owned"))?;
    super::inventory::verify_protection(&root)?;
    let journal_path = root.join("storage-migration.json");
    let source_path = root.join("db.sqlite");
    let mut journal = read_journal(&root)?;
    if journal.format != 1 || StorageDescriptor::read(&root)?.as_ref() != Some(&journal.descriptor)
    {
        return Err(storage_error("legacy migration is not activated"));
    }
    if source_path.exists() {
        progress(MigrationProgress::phase(
            "verifying the retained original database",
        ));
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
            let unchanged = match &journal.snapshot {
                Some(snapshot) => source_identity(&root)?.as_ref() == Some(snapshot),
                // Journals from the earlier migration have logical receipts only.
                None => table_receipts(&source, None).await? == journal.source,
            };
            if !unchanged {
                return Err(storage_error(
                    "original database changed after activation; it has been kept",
                ));
            }
            frozen.rollback().await?;
            Ok::<_, sqlx::Error>(())
        }
        .await;
        source.close().await;
        result?;
    }
    let index = checked_path(&root, &journal.descriptor.index)?;
    let reopened = DatabaseManager::new_with_storage(
        index.to_str().unwrap(),
        config,
        Some(HybridStorage::new(
            root.clone(),
            journal.descriptor.clone(),
        )?),
        false,
        false,
    )
    .await?;
    let query = sqlx::query("SELECT id FROM frames LIMIT 1")
        .fetch_optional(&reopened.pool)
        .await;
    reopened.close().await;
    query?;
    journal.phase = Phase::Complete;
    durable_json(&journal_path, &journal)?;
    let mut report = journal
        .report
        .ok_or_else(|| storage_error("migration verification receipt is missing"))?;
    report.source_identity = match migration_report(&root)? {
        Some(completed)
            if completed.database_id == report.database_id
                && completed.generation == report.generation =>
        {
            completed.source_identity
        }
        _ => source_identity(&root)?,
    };
    durable_json(&root.join("storage-migration-complete.json"), &report)?;
    super::faults::checkpoint("migration_completed");
    std::fs::remove_file(&journal_path)?;
    sync_directory(&root)?;
    Ok(report)
}

impl DatabaseManager {
    /// The descriptor of this open manager, rather than a desired on-disk mode.
    pub fn storage_descriptor(&self) -> Option<&StorageDescriptor> {
        self.storage.as_ref().map(|storage| &storage.descriptor)
    }

    /// Deletion is available only on the successfully opened migrated generation.
    pub fn retained_migration_source_bytes(&self) -> Result<Option<u64>, sqlx::Error> {
        let storage = self
            .storage
            .as_ref()
            .ok_or_else(|| storage_error("new storage is not running"))?;
        let root = &storage.root;
        if self.pool.is_closed()
            || root.join("storage-migration.json").exists()
            || StorageDescriptor::read(root)?.as_ref() != Some(&storage.descriptor)
        {
            return Err(storage_error(
                "migration has not finished switching storage",
            ));
        }
        let report = migration_report(root)?
            .ok_or_else(|| storage_error("migration verification receipt is missing"))?;
        if report.database_id != storage.descriptor.database_id
            || report.generation != storage.descriptor.generation
        {
            return Err(storage_error(
                "migration receipt does not match the running storage",
            ));
        }
        let Some(actual) = source_identity(root)? else {
            return Ok(None);
        };
        if report.source_identity.as_ref() != Some(&actual) {
            return Err(storage_error(
                "original database changed after migration; it has been kept",
            ));
        }
        // WAL and rollback journals can contain records outside the receipt.
        // An orphaned SHM is only a WAL index and carries no database content.
        for name in ["db.sqlite-wal", "db.sqlite-journal"] {
            if checked_path(root, Path::new(name))?.exists() {
                return Err(storage_error(
                    "original database has SQLite WAL or rollback journal files; deletion is blocked and the original has been kept",
                ));
            }
        }
        Ok(Some(actual.bytes))
    }

    pub async fn delete_migration_source(&self, generation: &str) -> Result<u64, sqlx::Error> {
        let storage = self
            .storage
            .as_ref()
            .ok_or_else(|| storage_error("new storage is not running"))?;
        if storage.descriptor.generation != generation {
            return Err(storage_error(
                "storage changed since deletion was requested",
            ));
        }
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(storage.root.join(".storage.lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .map_err(|_| storage_error("storage lifecycle is already owned"))?;
        let source_path = checked_path(&storage.root, Path::new("db.sqlite"))?;
        let _source_owner =
            screenpipe_sqlite_coordinator::acquire_sqlite_manager_lease(&source_path)
                .map_err(storage_error)?;
        let bytes = self
            .retained_migration_source_bytes()?
            .ok_or_else(|| storage_error("original database has already been deleted"))?;
        // Prove the live manager can serve a real query before the irreversible step.
        sqlx::query("SELECT id FROM frames LIMIT 1")
            .fetch_optional(&self.pool)
            .await?;
        self.retained_migration_source_bytes()?;
        std::fs::remove_file(source_path)?;
        sync_directory(&storage.root)?;
        Ok(bytes)
    }
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
    if journal.format != 1
        || !matches!(
            journal.phase,
            Phase::Building | Phase::Ready | Phase::Paused
        )
    {
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
    let verified = async {
        verify_integrity(&source.pool).await?;
        if !journal.source.is_empty() {
            if table_receipts(&source, Some(&journal.source)).await? != journal.source {
                return Err(storage_error("legacy source differs from its receipt; both files kept for diagnosis"));
            }
        } else if journal.snapshot.is_none() || source_identity(&root)? != journal.snapshot {
            return Err(storage_error("cannot verify complete legacy source after it changed; both files kept for diagnosis"));
        }
        Ok::<_, sqlx::Error>(())
    }.await;
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
    storage: &std::sync::Arc<HybridStorage>,
) -> Result<(), sqlx::Error> {
    let budget = &storage.descriptor.budget;
    tracing::info!(phase = "compact_candidate", "storage migration");
    let compact = index.with_extension("compacting.sqlite");
    if compact.exists() {
        std::fs::remove_file(&compact)?;
    }
    // The last manager connection can be read-only, leaving a valid WAL on
    // disk. Checkpoint and leave WAL mode before replacing this inactive file;
    // otherwise old WAL frames can be replayed onto the compacted page layout.
    let mut conn = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new()
            .filename(index)
            .pragma("locking_mode", "EXCLUSIVE")
            .pragma("journal_mode", "DELETE"),
    )
    .await?;
    super::bulk::register_hash(&mut conn).await?;
    if storage.has_bulk() {
        super::bulk::elements::register(&mut conn, storage.clone()).await?;
    }
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
    super::sync_file(&compact)?;
    super::faults::checkpoint("candidate_compacted");
    std::fs::rename(&compact, index)?;
    sync_directory(index.parent().unwrap())
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

pub(super) fn hash_value(
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

pub(super) async fn logical_tables(db: &DatabaseManager) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT m.name FROM sqlite_master m WHERE m.type='table' AND (m.name NOT LIKE 'sqlite_%' OR m.name='sqlite_sequence') AND m.name NOT LIKE '%_fts%' AND m.name NOT LIKE '%_fts5%' AND (m.sql NOT LIKE 'CREATE VIRTUAL TABLE%' OR m.name='elements') ORDER BY m.name")
        .fetch_all(&db.pool).await
}

pub(super) async fn table_receipts(
    db: &DatabaseManager,
    source: Option<&[TableParity]>,
) -> Result<Vec<TableParity>, sqlx::Error> {
    let tables: Vec<String> = if let Some(source) = source {
        source.iter().map(|t| t.table.clone()).collect()
    } else {
        logical_tables(db).await?
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
        columns.retain(|name| !name.starts_with("_archive_"));
        let key = if db.storage.as_ref().is_some_and(|s| s.has_bulk())
            && super::bulk::is_bulk_table(&table)
        {
            "id"
        } else {
            "rowid"
        };
        let mut hash = Sha256::new();
        let mut count = 0;
        let mut last = i64::MIN;
        let mut first = true;
        loop {
            let sql = format!(
                "SELECT {key} AS __storage_rowid,{} FROM {} WHERE {key}{}? ORDER BY {key} LIMIT 128",
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
            super::sync_file(&destination)?;
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
            storage.verify_bulk(&self.pool).await?;
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
                capabilities: super::capabilities(),
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
            let mut conn = sqlx::SqliteConnection::connect_with(
                &sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(&index)
                    .read_only(true),
            )
            .await?;
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM sqlite_master WHERE name='_hybrid_migrations'",
            )
            .fetch_one(&mut conn)
            .await?;
            let complete = if count != 0 {
                let version = if descriptor
                    .capabilities
                    .iter()
                    .any(|c| c == super::bulk::CAPABILITY)
                {
                    2
                } else {
                    1
                };
                sqlx::query_scalar::<_, i64>(
                    "SELECT count(*) FROM _hybrid_migrations WHERE version=?",
                )
                .bind(version)
                .fetch_one(&mut conn)
                .await?
                    != 0
            } else {
                false
            };
            conn.close().await?;
            complete
        } else {
            false
        };
        if !catalog_exists {
            // Initialization owns an empty unpublished generation. Its schema
            // completion receipt determines whether construction can be reused.
            for suffix in ["", "-wal", "-shm"] {
                let path = PathBuf::from(format!("{}{suffix}", index.display()));
                if path.exists() {
                    std::fs::remove_file(path)?;
                }
            }
        }
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
