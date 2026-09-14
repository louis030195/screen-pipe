// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

//! Versioned local frame payloads. SQLite owns visibility; immutable files own
//! sealed bytes. All mutations join the database's existing writer capability.

mod backup;
mod bindings;
pub(crate) mod bulk;
mod codec;
mod command;
mod faults;
mod import;
mod inventory;
mod lifecycle;
mod maintenance;
pub(crate) mod read_schema;
mod reader;
pub(crate) mod schema;
mod sealing;
pub(crate) mod snapshot;
pub(crate) mod sql;
mod validation;

pub use backup::restore;
pub use command::run_command;
pub use inventory::{artifact_bytes, inventory};
pub use lifecycle::{
    cancel_migration, migrate, migrate_with_progress, migration_report,
    pause_interrupted_migration, MigrationOptions, MigrationProgress, MigrationReport,
};
pub use maintenance::{compact, export_sqlite};
pub use reader::StorageReadToken;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock, Semaphore};
pub use validation::compare;

pub(crate) fn storage_error(error: impl std::fmt::Display) -> sqlx::Error {
    sqlx::Error::Protocol(format!("frame storage: {error}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StorageMode {
    Sqlite,
    HybridParquetV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StorageDescriptor {
    pub format: u32,
    pub mode: StorageMode,
    pub database_id: String,
    pub generation: String,
    /// A converted legacy root retains its existing consumer wire namespace.
    pub source_continuity: bool,
    pub index: PathBuf,
    pub payloads: PathBuf,
    pub capabilities: Vec<String>,
    pub budget: StorageBudget,
    pub privacy: PrivacyPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StorageBudget {
    pub row_group_rows: usize,
    pub file_rows: usize,
    pub file_bytes: usize,
    pub record_bytes: usize,
    pub decode_bytes: usize,
    pub response_bytes: usize,
    pub concurrent_decodes: usize,
    pub staging_bytes: u64,
    pub disk_reserve_bytes: u64,
    pub operation_timeout_secs: u64,
    pub lifecycle_timeout_secs: u64,
    pub read_attempts: u8,
}

impl Default for StorageBudget {
    fn default() -> Self {
        Self {
            row_group_rows: 128,
            file_rows: 1024,
            file_bytes: 16 * 1024 * 1024,
            record_bytes: 32 * 1024 * 1024,
            decode_bytes: 128 * 1024 * 1024,
            response_bytes: 128 * 1024 * 1024,
            concurrent_decodes: 2,
            staging_bytes: 512 * 1024 * 1024,
            disk_reserve_bytes: 2 * 1024 * 1024 * 1024,
            operation_timeout_secs: 60,
            lifecycle_timeout_secs: 3600,
            read_attempts: 3,
        }
    }
}

impl StorageBudget {
    pub fn validate(&self) -> Result<(), sqlx::Error> {
        if self.row_group_rows == 0
            || self.file_rows < self.row_group_rows
            || self.file_bytes == 0
            || self.record_bytes == 0
            || self.record_bytes > 128 * 1024 * 1024
            || self.file_bytes > 128 * 1024 * 1024
            || self.decode_bytes < self.file_bytes.max(self.record_bytes).saturating_mul(2)
            || self.decode_bytes > 512 * 1024 * 1024
            || self.response_bytes > 512 * 1024 * 1024
            || self.concurrent_decodes > 8
            || self.staging_bytes > i64::MAX as u64
            || self.response_bytes < self.record_bytes
            || self.concurrent_decodes == 0
            || self.staging_bytes < self.record_bytes as u64
            || self.operation_timeout_secs == 0
            || self.lifecycle_timeout_secs == 0
            || self.read_attempts == 0
            || self.read_attempts > 8
        {
            return Err(storage_error("invalid storage budget"));
        }
        Ok(())
    }
}

/// Each bit is one independently completed configured frame surface.
/// Bit order: full text, accessibility text, tree JSON, OCR JSON, window, URL.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PrivacyPolicy {
    pub identity: String,
    pub required_surfaces: u8,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FramePayload {
    pub id: i64,
    pub generation: i64,
    pub full_text: Option<String>,
    pub accessibility_text: Option<String>,
    pub accessibility_tree_json: Option<String>,
    pub text_json: Option<String>,
}

impl FramePayload {
    pub fn text(&self) -> &str {
        self.full_text
            .as_deref()
            .or(self.accessibility_text.as_deref())
            .unwrap_or("")
    }

    pub fn bytes(&self) -> usize {
        [
            &self.full_text,
            &self.accessibility_text,
            &self.accessibility_tree_json,
            &self.text_json,
        ]
        .into_iter()
        .flatten()
        .map(String::len)
        .sum()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection {
    Search,
    Detail,
    All,
}

pub(crate) struct HybridStorage {
    pub root: PathBuf,
    pub descriptor: StorageDescriptor,
    pub gate: Arc<Mutex<()>>,
    pub leases: Arc<RwLock<()>>,
    pub decoder: Arc<Semaphore>,
    pub file_job: Mutex<()>,
    pub read_lanes: std::sync::OnceLock<Arc<Semaphore>>,
    pub closing: tokio_util::sync::CancellationToken,
    pub privacy_ready: std::sync::atomic::AtomicBool,
    pub bulk: bulk::Runtime,
}

const FRAME_CAPABILITIES: [&str; 3] =
    ["parquet-frame-v1", "contentless-delete-fts5", "durable-wal"];
pub(super) fn capabilities() -> Vec<String> {
    FRAME_CAPABILITIES
        .into_iter()
        .chain([bulk::CAPABILITY])
        .map(String::from)
        .collect()
}

impl StorageDescriptor {
    pub fn read(root: &Path) -> Result<Option<Self>, sqlx::Error> {
        let path = root.join("storage.json");
        match std::fs::read(&path) {
            Ok(bytes) => {
                let descriptor: Self = serde_json::from_slice(&bytes).map_err(storage_error)?;
                descriptor.validate(root)?;
                Ok(Some(descriptor))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn validate(&self, root: &Path) -> Result<(), sqlx::Error> {
        if self.format != 1
            || self.mode != StorageMode::HybridParquetV1
            || (self.capabilities != FRAME_CAPABILITIES.map(String::from)
                && self.capabilities != capabilities())
        {
            return Err(storage_error("unsupported storage format or capabilities"));
        }
        uuid::Uuid::parse_str(&self.database_id).map_err(storage_error)?;
        uuid::Uuid::parse_str(&self.generation).map_err(storage_error)?;
        if self.index
            != PathBuf::from("storage")
                .join(&self.generation)
                .join("index.sqlite")
        {
            return Err(storage_error(
                "index path does not identify its physical generation",
            ));
        }
        let payload_parts: Vec<_> = self.payloads.components().collect();
        if payload_parts.len() != 3
            || payload_parts[0].as_os_str() != "storage"
            || payload_parts[2].as_os_str() != "payloads"
        {
            return Err(storage_error("invalid payload generation root"));
        }
        uuid::Uuid::parse_str(
            payload_parts[1]
                .as_os_str()
                .to_str()
                .ok_or_else(|| storage_error("invalid payload generation"))?,
        )
        .map_err(storage_error)?;
        self.budget.validate()?;
        if self.privacy.required_surfaces & !63 != 0
            || (self.privacy.required_surfaces != 0 && self.privacy.identity.is_empty())
        {
            return Err(storage_error("invalid privacy policy"));
        }
        checked_path(root, &self.index)?;
        checked_path(root, &self.payloads)?;
        Ok(())
    }
}

/// Resolve persisted mode before a caller starts mode-specific recovery.
pub fn resolve_database_path(database: &Path) -> Result<PathBuf, sqlx::Error> {
    let root = database
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if database.file_name().is_some_and(|name| name == "db.sqlite") {
        if root.join("storage-maintenance.json").exists() {
            return Err(storage_error(
                "offline index maintenance pending; resume compact before opening",
            ));
        }
        if !root.join("storage.json").exists()
            && (root.join("storage-init.json").exists()
                || (root.join("storage-migration.json").exists()
                    && !lifecycle::migration_is_paused(root)?))
        {
            return Err(storage_error(
                "offline migration pending; resume or cancel it before opening",
            ));
        }
        if let Some(descriptor) = StorageDescriptor::read(root)? {
            let path = checked_path(root, &descriptor.index)?;
            if !path.is_file() {
                return Err(storage_error("active index is missing"));
            }
            return Ok(path);
        }
    }
    Ok(database.to_path_buf())
}

pub(crate) fn checked_path(root: &Path, relative: &Path) -> Result<PathBuf, sqlx::Error> {
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(storage_error(
            "storage path must be relative and confined to its root",
        ));
    }
    let root = root.canonicalize()?;
    let path = root.join(relative);
    let mut component_path = root.clone();
    for component in relative.components() {
        component_path.push(component);
        if std::fs::symlink_metadata(&component_path)
            .is_ok_and(|meta| meta.file_type().is_symlink())
        {
            return Err(storage_error("storage paths cannot contain symbolic links"));
        }
    }
    let mut existing = path.as_path();
    while !existing.exists() {
        existing = existing
            .parent()
            .ok_or_else(|| storage_error("invalid storage path"))?;
    }
    if !existing.canonicalize()?.starts_with(&root) {
        return Err(storage_error("storage path escapes its root"));
    }
    Ok(path)
}

pub(crate) fn sync_directory(path: &Path) -> Result<(), sqlx::Error> {
    #[cfg(unix)]
    std::fs::File::open(path)?.sync_all()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(0x02000000)
            .open(path)?
            .sync_all()?;
    }
    Ok(())
}

pub(crate) fn durable_json<T: Serialize>(path: &Path, value: &T) -> Result<(), sqlx::Error> {
    use std::io::Write;
    let parent = path
        .parent()
        .ok_or_else(|| storage_error("missing descriptor parent"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(&mut temporary, value).map_err(storage_error)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|e| sqlx::Error::Io(e.error))?;
    sync_directory(parent)
}

impl HybridStorage {
    pub fn payload_path(&self, relative: &Path) -> Result<PathBuf, sqlx::Error> {
        if !relative.starts_with(&self.descriptor.payloads) {
            return Err(storage_error("file is outside its payload root"));
        }
        checked_path(&self.root, relative)
    }
    pub async fn verify_catalog(&self, pool: &sqlx::SqlitePool) -> Result<(), sqlx::Error> {
        use sqlx::Row;
        if self.has_bulk() {
            let files: Vec<(String, String, String)> = sqlx::query_as(
                "SELECT table_name,path,checksum FROM _bulk_files WHERE state IN ('published','dirty')",
            ).fetch_all(pool).await?;
            for (table, path, checksum) in files {
                if !bulk::is_bulk_table(&table)
                    || checksum.len() != 64
                    || !self.payload_path(Path::new(&path))?.is_file()
                {
                    return Err(storage_error("bulk payload catalog is incomplete"));
                }
            }
        }
        let missing:i64=sqlx::query_scalar("SELECT count(*) FROM frames f LEFT JOIN frame_payloads p ON p.frame_id=f.id WHERE p.frame_id IS NULL").fetch_one(pool).await?;
        if missing != 0 {
            return Err(storage_error("frame payload catalog is incomplete"));
        }
        let files=sqlx::query("SELECT DISTINCT pf.search_path,pf.detail_path,pf.state,pf.schema_version FROM frame_payloads p LEFT JOIN payload_files pf ON p.file_id=pf.id WHERE p.state='sealed'").fetch_all(pool).await?;
        for file in files {
            if !matches!(file.try_get::<&str, _>("state")?, "published" | "dirty")
                || file.try_get::<i64, _>("schema_version")? != codec::SCHEMA_VERSION as i64
            {
                return Err(storage_error(
                    "unsupported or incomplete payload publication",
                ));
            }
            for column in ["search_path", "detail_path"] {
                let path = self.payload_path(Path::new(file.try_get::<&str, _>(column)?))?;
                if !path.is_file() {
                    return Err(storage_error("committed payload file is missing"));
                }
            }
        }
        Ok(())
    }

    pub fn for_index(path: &Path) -> Result<Option<Arc<Self>>, sqlx::Error> {
        if path.file_name().is_none_or(|name| name != "index.sqlite") {
            return Ok(None);
        }
        if path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .is_none_or(|name| name != "storage")
        {
            return Ok(None);
        }
        let Some(root) = path.parent().and_then(Path::parent).and_then(Path::parent) else {
            return Ok(None);
        };
        let Some(descriptor) = StorageDescriptor::read(root)? else {
            return Err(storage_error(
                "hybrid index requires an active storage descriptor",
            ));
        };
        if checked_path(root, &descriptor.index)? != path.canonicalize()? {
            return Err(storage_error("index is not the active database generation"));
        }
        Self::new(root.to_path_buf(), descriptor).map(Some)
    }

    pub fn new(root: PathBuf, descriptor: StorageDescriptor) -> Result<Arc<Self>, sqlx::Error> {
        descriptor.validate(&root)?;
        Ok(Arc::new(Self {
            root,
            decoder: Arc::new(Semaphore::new(descriptor.budget.concurrent_decodes)),
            descriptor,
            gate: Arc::new(Mutex::new(())),
            leases: Arc::new(RwLock::new(())),
            file_job: Mutex::new(()),
            read_lanes: std::sync::OnceLock::new(),
            closing: tokio_util::sync::CancellationToken::new(),
            privacy_ready: std::sync::atomic::AtomicBool::new(false),
            bulk: bulk::Runtime::default(),
        }))
    }
}
