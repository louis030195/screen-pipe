// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

//! `screenpipe db ...` — corruption recovery + storage cleanup.
//!
//! ## Safety model
//!
//! These commands rewrite or delete files inside `~/.screenpipe/`. A racing
//! desktop-app launch (or a second CLI invocation) would silently corrupt the
//! database again. The reliability story is built around **one PID lock file**
//! that every mutating path acquires:
//!
//!   * `~/.screenpipe/.db_recovery.lock` — JSON `{pid, host, started_at, op}`
//!   * Created with `O_CREAT|O_EXCL` (atomic) so two CLI runs can't both win.
//!   * Heartbeated every 30 s by a background thread so a long recovery
//!     (multi-GB DB) doesn't look stale.
//!   * Released on normal Drop, on SIGINT, and on SIGTERM.
//!   * The desktop app refuses to start while the lock is fresh
//!     (`apps/screenpipe-app-tauri/src-tauri/src/main.rs`); env var
//!     `SCREENPIPE_IGNORE_DB_LOCK=1` is the escape hatch.
//!
//! ## When the lock is "stale"
//!
//! 1. Foreign host (lock written from another machine over a shared `$HOME`):
//!    we **refuse to clear** automatically — print the path and tell the user.
//! 2. Same host, PID gone: clear and proceed.
//! 3. Same host, PID alive: refuse, point at `screenpipe db unlock`.
//! 4. Same host, PID unknown, mtime > 1 h: clear (heartbeat would have kept it
//!    fresh; older means the heartbeat thread is dead too).
//!
//! ## Recovery pre-flight
//!
//!  * SQLite's official recovery extension is compiled into Screenpipe
//!  * budget space for a candidate and private WAL/SHM; copy the main file
//!    only when the filesystem cannot hard-link it into the working directory
//!  * never write or checkpoint the quarantined generation; recover through a
//!    read-only main-file handle with private WAL/SHM at the working path
//!  * discard recovered external-content FTS shadow data and rebuild those
//!    derived indexes from authoritative application tables
//!  * require a new physical file identity, quick/full integrity, zero foreign
//!    key violations, FTS integrity, and durable write/close/reopen canaries
//!  * preserve the exact original DB/WAL/SHM in a recovery directory and clear
//!    durable quarantine only after the installed file passes verification

use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use sysinfo::{DiskExt, Pid, PidExt, System, SystemExt};

use super::DbCommand;

const LOCK_FILE: &str = ".db_recovery.lock";
const RECOVERY_MANIFEST_FILE: &str = "recovery-manifest.json";
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const STALE_AFTER: Duration = Duration::from_secs(3600); // 1 h

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum RecoveryPhase {
    Preparing,
    CandidateVerified,
    OriginalArchived,
    CandidateInstalled,
    InstalledVerified,
    Complete,
    RestoredAfterFailure,
}

impl RecoveryPhase {
    fn file_label(&self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::CandidateVerified => "candidate-verified",
            Self::OriginalArchived => "original-archived",
            Self::CandidateInstalled => "candidate-installed",
            Self::InstalledVerified => "installed-verified",
            Self::Complete => "complete",
            Self::RestoredAfterFailure => "restored-after-failure",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RecoveryManifest {
    schema_version: u32,
    phase: RecoveryPhase,
    live_path: PathBuf,
    original_identity: screenpipe_db::SqliteFileIdentity,
    candidate_identity: Option<screenpipe_db::SqliteFileIdentity>,
    started_at_unix_ms: u64,
}

// ── lock file payload ──────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug)]
struct LockPayload {
    pid: u32,
    host: String,
    /// Unix-epoch seconds when the lock was acquired.
    started_at: u64,
    /// Free-form: "recover", "cleanup", "unlock". Useful in error messages.
    op: String,
}

fn current_host() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── lock guard ─────────────────────────────────────────────────────────

struct DbLock {
    path: PathBuf,
    /// Set to true on Drop / signal — heartbeat thread observes and exits.
    stop: Arc<AtomicBool>,
}

#[derive(Debug)]
enum LockState {
    /// No lock file present — free to acquire.
    Free,
    /// Lock is held by `pid` on the same host and that PID is alive.
    HeldHere { pid: u32, op: String },
    /// Lock is held by another machine — never auto-clear.
    Foreign { host: String, pid: u32 },
    /// Lock file exists but the holder is gone (dead PID and/or mtime past
    /// STALE_AFTER). Safe to overwrite.
    Stale,
    /// Lock file exists but is unparseable — treat as stale after mtime check.
    Unreadable,
}

impl DbLock {
    fn acquire(data_dir: &Path, op: &str) -> Result<Self> {
        Self::acquire_inner(data_dir, op, true)
    }

    fn acquire_inner(data_dir: &Path, op: &str, install_signal_handlers: bool) -> Result<Self> {
        let path = data_dir.join(LOCK_FILE);
        fs::create_dir_all(data_dir).ok();

        match Self::inspect(&path) {
            LockState::Free => {}
            LockState::Stale => {
                let _ = fs::remove_file(&path);
            }
            LockState::Unreadable => {
                let mtime_ok = fs::metadata(&path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.elapsed().ok())
                    .map(|e| e > STALE_AFTER)
                    .unwrap_or(true);
                if !mtime_ok {
                    bail!(
                        "lock file at {} is corrupted but recent (<1 h). \
if you're sure no `screenpipe db ...` is running, run `screenpipe db unlock --force`.",
                        path.display(),
                    );
                }
                let _ = fs::remove_file(&path);
            }
            LockState::HeldHere { pid, op: other } => {
                bail!(
                    "another db op is running: {} (pid {}). wait for it, or `screenpipe db unlock --force` if you're sure it's stuck.",
                    other,
                    pid,
                );
            }
            LockState::Foreign { host, pid } => {
                bail!(
                    "lock file at {} is held by host {host} (pid {pid}). this looks like a shared $HOME. \
will not auto-clear cross-host locks. resolve manually if needed.",
                    path.display(),
                );
            }
        }

        // Atomic exclusive create — fails if another process raced us between
        // the inspect() above and now.
        let payload = LockPayload {
            pid: std::process::id(),
            host: current_host(),
            started_at: now_unix(),
            op: op.to_string(),
        };
        let body = serde_json::to_string(&payload).context("serializing lock payload")? + "\n";
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| {
                format!(
                    "couldn't acquire lock at {}. another `screenpipe db ...` may have raced us.",
                    path.display()
                )
            })?;
        file.write_all(body.as_bytes())
            .context("writing lock payload")?;
        drop(file);

        let stop = Arc::new(AtomicBool::new(false));
        Self::start_heartbeat(&path, stop.clone());
        if install_signal_handlers {
            Self::install_signal_handlers(&path, stop.clone());
        }

        Ok(Self { path, stop })
    }

    fn inspect(path: &Path) -> LockState {
        let raw = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => return LockState::Free,
        };
        let payload: LockPayload = match serde_json::from_str(raw.trim()) {
            Ok(p) => p,
            Err(_) => return LockState::Unreadable,
        };
        if payload.host != current_host() {
            return LockState::Foreign {
                host: payload.host,
                pid: payload.pid,
            };
        }
        if pid_alive(payload.pid) {
            return LockState::HeldHere {
                pid: payload.pid,
                op: payload.op,
            };
        }
        // PID dead — might still be a fresh-but-orphaned lock. Heartbeat keeps
        // mtime current while a real op runs; if mtime > STALE_AFTER, treat as
        // stale regardless.
        let recently_touched = fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .map(|e| e <= STALE_AFTER)
            .unwrap_or(false);
        if recently_touched {
            // Edge case: PID died <1 h ago and heartbeat thread also died.
            // Conservative: still treat as stale since the holder is gone.
            // We err on the side of letting the user proceed; they can always
            // re-acquire after.
            LockState::Stale
        } else {
            LockState::Stale
        }
    }

    fn start_heartbeat(path: &Path, stop: Arc<AtomicBool>) {
        let path = path.to_path_buf();
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                // Touch the mtime by rewriting the same content.
                if let Ok(content) = fs::read_to_string(&path) {
                    let _ = OpenOptions::new()
                        .write(true)
                        .truncate(true)
                        .open(&path)
                        .and_then(|mut f| f.write_all(content.as_bytes()));
                }
                std::thread::sleep(HEARTBEAT_INTERVAL);
            }
        });
    }

    fn install_signal_handlers(path: &Path, stop: Arc<AtomicBool>) {
        let path = path.to_path_buf();
        // Single best-effort cleanup task. tokio::signal works for both
        // SIGINT (ctrl-c) and SIGTERM (kill default) on unix; Windows ctrl-c
        // also flows through this.
        tokio::spawn(async move {
            #[cfg(unix)]
            {
                use tokio::signal::unix::{signal, SignalKind};
                let mut term = match signal(SignalKind::terminate()) {
                    Ok(s) => s,
                    Err(_) => return,
                };
                let mut int_ = match signal(SignalKind::interrupt()) {
                    Ok(s) => s,
                    Err(_) => return,
                };
                tokio::select! {
                    _ = term.recv() => {}
                    _ = int_.recv() => {}
                }
            }
            #[cfg(not(unix))]
            {
                let _ = tokio::signal::ctrl_c().await;
            }
            stop.store(true, Ordering::Relaxed);
            let _ = fs::remove_file(&path);
            eprintln!("\nscreenpipe: caught signal, lock released. exiting.");
            std::process::exit(130);
        });
    }
}

impl Drop for DbLock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = fs::remove_file(&self.path);
    }
}

fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // This lock also protects desktop startup on Windows, where an external
    // `kill` executable is not available. Check the owner on every platform.
    let mut system = System::new();
    system.refresh_process(Pid::from_u32(pid))
}

// ── runtime checks ─────────────────────────────────────────────────────

fn server_is_running() -> bool {
    use std::net::TcpStream;
    TcpStream::connect_timeout(
        &"127.0.0.1:3030".parse().unwrap(),
        Duration::from_millis(250),
    )
    .is_ok()
}

fn ensure_app_quit(force: bool) -> Result<()> {
    if !force && server_is_running() {
        bail!(
            "screenpipe is currently running (port 3030 is open). quit the app from the menubar first, or pass --force to override (only safe if you know nothing is writing to the db)."
        );
    }
    Ok(())
}

fn ensure_disk_space(data_dir: &Path, needed: u64) -> Result<()> {
    let available = available_disk_bytes(data_dir);
    if let Some(avail) = available {
        if avail < needed {
            bail!(
                "not enough disk space: need ~{} MB free, have {} MB. free space outside the active database recovery evidence; `screenpipe db cleanup` can reclaim old artifacts only when quarantine is not active.",
                needed / 1_048_576,
                avail / 1_048_576,
            );
        }
    }
    Ok(())
}

fn available_disk_bytes(path: &Path) -> Option<u64> {
    let path = fs::canonicalize(path).ok()?;
    let mut system = System::new();
    system.refresh_disks_list();
    system.refresh_disks();
    system
        .disks()
        .iter()
        .filter(|disk| path.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().components().count())
        .map(|disk| disk.available_space())
}

// ── command entry ──────────────────────────────────────────────────────

pub async fn handle_db_command(command: &DbCommand) -> Result<()> {
    let data_dir = screenpipe_core::paths::default_screenpipe_data_dir();
    if data_dir.join("storage.json").exists() {
        if matches!(command, DbCommand::Check) {
            let db = screenpipe_db::DatabaseManager::new(
                data_dir
                    .join("db.sqlite")
                    .to_str()
                    .context("invalid database path")?,
                Default::default(),
            )
            .await?;
            let result = db.verify_storage().await;
            db.close().await;
            result?;
            return Ok(());
        }
        bail!("hybrid storage lifecycle uses screenpipe-storage verify, restore, compact, or resume migration");
    }
    match command {
        DbCommand::Check => integrity_check(&data_dir.join("db.sqlite")),
        DbCommand::Recover { force, resume } => recover(&data_dir, *force, *resume).await,
        DbCommand::Cleanup { apply, force } => cleanup(&data_dir, *apply, *force).await,
        DbCommand::Unlock { force } => unlock(&data_dir, *force),
    }
}

// ── check ──────────────────────────────────────────────────────────────

fn integrity_check(db_path: &Path) -> Result<()> {
    if !db_path.exists() {
        bail!("no database at {}", db_path.display());
    }
    if screenpipe_db::sqlite_quarantine_exists(db_path) {
        bail!(
            "database is durably quarantined; refusing to open or checkpoint its DB/WAL/SHM generation. quit screenpipe and run `screenpipe db recover`"
        );
    }
    println!("running PRAGMA quick_check on {} …", db_path.display());
    let connection = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("opening {} read-only", db_path.display()))?;
    let mut statement = connection.prepare("PRAGMA quick_check")?;
    let results = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if results.as_slice() == ["ok"] {
        println!("  ✓ database is healthy");
        Ok(())
    } else {
        bail!(
            "corruption detected:\n{}\n\nnext step: quit the screenpipe app, then run `screenpipe db recover`",
            results.join("\n")
        )
    }
}

// ── recover ────────────────────────────────────────────────────────────

fn sqlite_sidecar(database_path: &Path, suffix: &str) -> PathBuf {
    let mut name = database_path
        .file_name()
        .expect("database path must have a filename")
        .to_os_string();
    name.push(suffix);
    database_path.with_file_name(name)
}

fn sync_file(path: &Path) -> Result<()> {
    // FlushFileBuffers rejects Windows handles opened without write access,
    // even when the file itself was just created by fs::copy.
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("opening {} for durability sync", path.display()))?
        .sync_all()
        .with_context(|| format!("syncing {}", path.display()))
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    fs::File::open(path)
        .with_context(|| format!("opening directory {} for sync", path.display()))?
        .sync_all()
        .with_context(|| format!("syncing directory {}", path.display()))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

fn atomic_write_manifest(path: &Path, manifest: &RecoveryManifest) -> Result<()> {
    let temp = path.with_extension(format!("json.tmp-{}", std::process::id()));
    let body = serde_json::to_vec_pretty(manifest).context("serializing recovery manifest")?;
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .with_context(|| format!("creating recovery manifest temp {}", temp.display()))?;
        file.write_all(&body)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temp, path).with_context(|| {
            format!(
                "installing recovery manifest {} at {}",
                temp.display(),
                path.display()
            )
        })?;
        sync_directory(path.parent().expect("manifest has parent"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn update_manifest(
    path: &Path,
    manifest: &mut RecoveryManifest,
    phase: RecoveryPhase,
) -> Result<()> {
    manifest.phase = phase;
    let phase_path = path.with_file_name(format!(
        "recovery-manifest-{}.json",
        manifest.phase.file_label()
    ));
    atomic_write_manifest(&phase_path, manifest)
}

// Phase records are immutable; the original manifest remains Preparing. Read
// the latest durable phase rather than mistaking a completed swap for its start.
fn read_latest_recovery_manifest(directory: &Path) -> Result<Option<RecoveryManifest>> {
    let path = directory.join(RECOVERY_MANIFEST_FILE);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("reading recovery manifest"),
    };
    let initial: RecoveryManifest = serde_json::from_slice(&bytes)
        .with_context(|| format!("reading recovery manifest {}", path.display()))?;
    for phase in [
        RecoveryPhase::RestoredAfterFailure,
        RecoveryPhase::Complete,
        RecoveryPhase::InstalledVerified,
        RecoveryPhase::CandidateInstalled,
        RecoveryPhase::OriginalArchived,
        RecoveryPhase::CandidateVerified,
    ] {
        let phase_path = directory.join(format!("recovery-manifest-{}.json", phase.file_label()));
        let bytes = match fs::read(&phase_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).context("reading recovery phase manifest"),
        };
        let latest: RecoveryManifest = serde_json::from_slice(&bytes)
            .with_context(|| format!("reading recovery manifest {}", phase_path.display()))?;
        if latest.phase != phase
            || latest.schema_version != initial.schema_version
            || latest.live_path != initial.live_path
            || latest.original_identity != initial.original_identity
            || latest.started_at_unix_ms != initial.started_at_unix_ms
        {
            bail!(
                "recovery phase does not identify its original manifest: {}",
                phase_path.display()
            );
        }
        return Ok(Some(latest));
    }
    Ok(Some(initial))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GenerationComponentFingerprint {
    name: String,
    identity: screenpipe_db::SqliteFileIdentity,
    length: u64,
    modified_unix_nanos: Option<u128>,
}

fn generation_fingerprint(live: &Path) -> Result<Vec<GenerationComponentFingerprint>> {
    let mut fingerprint = Vec::new();
    for path in [
        live.to_path_buf(),
        sqlite_sidecar(live, "-wal"),
        sqlite_sidecar(live, "-shm"),
    ] {
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).with_context(|| format!("stating {}", path.display())),
        };
        fingerprint.push(GenerationComponentFingerprint {
            name: path
                .file_name()
                .expect("database component has filename")
                .to_string_lossy()
                .into_owned(),
            identity: screenpipe_db::sqlite_file_identity(&path)
                .with_context(|| format!("identifying {}", path.display()))?,
            length: metadata.len(),
            modified_unix_nanos: metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos()),
        });
    }
    if !fingerprint
        .iter()
        .any(|component| component.name == "db.sqlite")
    {
        bail!("database generation has no db.sqlite component");
    }
    Ok(fingerprint)
}

/// A quarantined app shell can cause SQLite to create a brand-new zero-byte
/// WAL while rendering the recovery UI. That file contains no committed data
/// and does not make the base database a newer generation. SHM is a transient
/// WAL index, not committed data: a timestamp-only change is also harmless.
/// Keep rejecting DB/WAL changes and SHM identity, size, or presence changes.
fn recovery_source_is_current(
    source: &[GenerationComponentFingerprint],
    current: &[GenerationComponentFingerprint],
) -> bool {
    let current_without_new_empty_wal = current.iter().filter(|component| {
        let newly_created_empty_wal = component.name == "db.sqlite-wal"
            && component.length == 0
            && !source
                .iter()
                .any(|source_component| source_component.name == component.name);
        !newly_created_empty_wal
    });

    let current = current_without_new_empty_wal.collect::<Vec<_>>();
    source.len() == current.len()
        && source.iter().zip(current).all(|(before, after)| {
            before == after
                || (before.name == "db.sqlite-shm"
                    && before.name == after.name
                    && before.identity == after.identity
                    && before.length == after.length)
        })
}

fn prepare_readonly_generation(
    live: &Path,
    work: &Path,
) -> Result<Vec<GenerationComponentFingerprint>> {
    let before = generation_fingerprint(live)?;
    // Only the main file is linked. Every SQLite input connection below uses
    // READONLY, so it cannot checkpoint or write through this link. WAL/SHM
    // remain private: SQLite may rebuild the working WAL index on open.
    // A hard link shares storage, not a mutable copy. The offline gate and
    // source fingerprint checks remain mandatory before installing a candidate.
    // Hard-linking a symlink has platform-dependent semantics; preserve the
    // old copy behavior for user configurations that symlink their database.
    let link_result = if fs::symlink_metadata(live)?.file_type().is_file() {
        fs::hard_link(live, work)
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "source is not a regular file",
        ))
    };
    let linked = match link_result {
        Ok(()) => true,
        Err(error) => {
            if fs::symlink_metadata(work).is_ok() {
                return Err(error).context("recovery working path already exists");
            }
            eprintln!("read-only source link unavailable ({error}); using a private database copy");
            false
        }
    };
    let required = recovery_space_required(&before, linked);
    if let Err(error) = ensure_disk_space(work.parent().expect("working copy has parent"), required)
    {
        if linked {
            // This is the link just created above, not the original database.
            let _ = fs::remove_file(work);
        }
        return Err(error);
    }
    if linked {
        println!("  main database linked read-only; no full-size database copy needed");
    } else {
        fs::copy(live, work).with_context(|| {
            format!(
                "copying quarantined database {} to working copy {}",
                live.display(),
                work.display()
            )
        })?;
        sync_file(work)?;
    }
    for suffix in ["-wal", "-shm"] {
        let source = sqlite_sidecar(live, suffix);
        if !source.exists() {
            continue;
        }
        let destination = sqlite_sidecar(work, suffix);
        fs::copy(&source, &destination).with_context(|| {
            format!(
                "copying quarantined sidecar {} to {}",
                source.display(),
                destination.display()
            )
        })?;
        sync_file(&destination)?;
    }
    sync_directory(work.parent().expect("working copy has parent"))?;
    let after = generation_fingerprint(live)?;
    if !recovery_source_is_current(&before, &after) {
        bail!(
            "live DB/WAL/SHM changed while it was being copied; refusing recovery because the source was not offline"
        );
    }
    Ok(before)
}

fn recovery_space_required(source: &[GenerationComponentFingerprint], linked: bool) -> u64 {
    let source_size = source.iter().fold(0u64, |size, component| {
        size.saturating_add(component.length)
    });
    let private_copy_size = source
        .iter()
        .filter(|component| !linked || component.name != "db.sqlite")
        .fold(0u64, |size, component| {
            size.saturating_add(component.length)
        });
    // Retain the existing candidate allowance, subtract only the main-file copy
    // we actually avoided, and leave 1 GiB for manifests/journals and the OS.
    source_size
        .saturating_add(private_copy_size)
        .saturating_add(1 << 30)
}

fn move_generation(live: &Path, destination_dir: &Path) -> Result<Vec<(PathBuf, PathBuf)>> {
    fs::create_dir_all(destination_dir)?;
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    for source in [
        live.to_path_buf(),
        sqlite_sidecar(live, "-wal"),
        sqlite_sidecar(live, "-shm"),
    ] {
        if !source.exists() {
            continue;
        }
        let destination = destination_dir.join(
            source
                .file_name()
                .expect("database generation component has filename"),
        );
        if destination.exists() {
            for (rollback_source, rollback_destination) in moved.iter().rev() {
                let _ = fs::rename(rollback_destination, rollback_source);
            }
            bail!(
                "recovery archive destination already exists: {}",
                destination.display()
            );
        }
        if let Err(error) = fs::rename(&source, &destination) {
            for (rollback_source, rollback_destination) in moved.iter().rev() {
                let _ = fs::rename(rollback_destination, rollback_source);
            }
            return Err(error).with_context(|| {
                format!(
                    "archiving database generation component {} to {}",
                    source.display(),
                    destination.display()
                )
            });
        }
        moved.push((source, destination));
    }
    if !moved.iter().any(|(source, _)| source == live) {
        bail!("live database disappeared before the generation swap");
    }
    sync_directory(live.parent().expect("live database has parent"))?;
    sync_directory(destination_dir)?;
    Ok(moved)
}

fn restore_generation(moved: &[(PathBuf, PathBuf)]) -> Result<()> {
    for (live, archived) in moved.iter().rev() {
        if archived.exists() {
            fs::rename(archived, live).with_context(|| {
                format!(
                    "restoring original database component {} to {}",
                    archived.display(),
                    live.display()
                )
            })?;
        }
    }
    if let Some((live, _)) = moved.first() {
        sync_directory(live.parent().expect("live path has parent"))?;
    }
    Ok(())
}

fn rollback_failed_install(
    live: &Path,
    recovery_dir: &Path,
    moved: &[(PathBuf, PathBuf)],
) -> Result<()> {
    let failed_dir = recovery_dir.join("failed-installed-generation");
    move_generation(live, &failed_dir).context(
        "archiving the failed installed DB/WAL/SHM before restoring the original generation",
    )?;
    restore_generation(moved)
}

fn newest_recovery_directories(data_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut directories = fs::read_dir(data_dir)
        .with_context(|| format!("reading {}", data_dir.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("db-recovery-"))
        })
        .collect::<Vec<_>>();
    directories.sort();
    directories.reverse();
    Ok(directories)
}

/// Exclude offline reconciliation until the caller has bound its HTTP listener.
/// Runtime maintenance still requires the caller to stop the recording process.
pub struct DatabaseStartupGuard {
    _lock: DbLock,
}

/// Reconcile interrupted replacement before ordinary SQLite startup can create
/// a missing DB or verify a main file whose committed WAL is still archived.
/// Unlike CLI maintenance, this guard never installs process-exit handlers.
pub async fn prepare_database_startup(data_dir: &Path) -> Result<DatabaseStartupGuard> {
    let lock = DbLock::acquire_inner(data_dir, "database startup", false)?;
    let legacy = data_dir.join("db.sqlite");
    let live = screenpipe_db::storage::resolve_database_path(&legacy)?;
    if live == legacy {
        reconcile_interrupted_recovery(data_dir, &live).await?;
    }
    Ok(DatabaseStartupGuard { _lock: lock })
}

async fn reconcile_interrupted_recovery(data_dir: &Path, live: &Path) -> Result<()> {
    for directory in newest_recovery_directories(data_dir)? {
        let manifest_path = directory.join(RECOVERY_MANIFEST_FILE);
        let mut manifest = match read_latest_recovery_manifest(&directory)? {
            Some(manifest) => manifest,
            None => {
                // A crash before preparing the manifest cannot have moved the
                // source. Unexplained archived components require diagnosis.
                if directory.join("source-generation").exists() {
                    bail!(
                        "interrupted recovery has archived data but no readable manifest: {}",
                        directory.display()
                    );
                }
                continue;
            }
        };
        if manifest.schema_version != 1 || manifest.live_path != live {
            bail!(
                "recovery manifest does not identify this database: {}",
                manifest_path.display()
            );
        }
        let identity = screenpipe_db::sqlite_file_identity(live).ok();
        if matches!(
            manifest.phase,
            RecoveryPhase::Complete | RecoveryPhase::RestoredAfterFailure
        ) {
            // A later completed repair supersedes evidence from earlier jobs.
            if identity.as_ref() == Some(&manifest.original_identity)
                || identity
                    .as_ref()
                    .is_some_and(|id| Some(id) == manifest.candidate_identity.as_ref())
            {
                return Ok(());
            }
            continue;
        }
        let source = directory.join("source-generation");
        if identity.as_ref() == Some(&manifest.original_identity) || !live.exists() {
            restore_interrupted_swap(data_dir, live)?;
            if screenpipe_db::sqlite_file_identity(live)? != manifest.original_identity {
                bail!("interrupted recovery did not restore the original database generation");
            }
            update_manifest(
                &manifest_path,
                &mut manifest,
                RecoveryPhase::RestoredAfterFailure,
            )?;
            return Ok(());
        }
        if identity
            .as_ref()
            .is_some_and(|id| Some(id) == manifest.candidate_identity.as_ref())
        {
            // CandidateInstalled can mean the process died before its final
            // durability check. Apply the existing complete verification battery
            // before resolving the original quarantine or allowing startup.
            let verified = screenpipe_db::verify_fresh_sqlite_recovery_candidate(
                live,
                &[manifest.original_identity.clone()],
            )
            .await
            .context("verifying interrupted installed recovery candidate")?;
            if Some(&verified.file_identity) != manifest.candidate_identity.as_ref() {
                bail!("installed recovery candidate changed during verification");
            }
            screenpipe_db::archive_resolved_sqlite_quarantine(
                live,
                source.join("resolved-quarantine.json"),
                &verified.file_identity,
                &[manifest.original_identity.clone()],
            )?;
            update_manifest(&manifest_path, &mut manifest, RecoveryPhase::Complete)?;
            return Ok(());
        }
        bail!(
            "interrupted recovery does not identify the installed generation: {}",
            directory.display()
        );
    }
    Ok(())
}

/// Repair only the interrupted archive half of a previous swap. The durable
/// quarantine marker remains active; this merely restores the exact old
/// generation so a fresh recovery attempt has a coherent source.
fn restore_interrupted_swap(data_dir: &Path, live: &Path) -> Result<()> {
    for directory in newest_recovery_directories(data_dir)? {
        let manifest = match read_latest_recovery_manifest(&directory)? {
            Some(manifest) => manifest,
            None => continue,
        };
        if matches!(
            manifest.phase,
            RecoveryPhase::Complete | RecoveryPhase::RestoredAfterFailure
        ) {
            continue;
        }
        let source_dir = directory.join("source-generation");
        let archived_db = source_dir.join("db.sqlite");
        let live_identity = screenpipe_db::sqlite_file_identity(live).ok();
        let live_is_original = live_identity
            .as_ref()
            .is_some_and(|identity| identity == &manifest.original_identity);

        if !live.exists() && archived_db.exists() {
            let components = [
                (archived_db, live.to_path_buf()),
                (
                    source_dir.join("db.sqlite-wal"),
                    sqlite_sidecar(live, "-wal"),
                ),
                (
                    source_dir.join("db.sqlite-shm"),
                    sqlite_sidecar(live, "-shm"),
                ),
            ];
            for (archived, destination) in &components {
                if archived.exists() && destination.exists() {
                    bail!(
                        "interrupted recovery component conflicts with installed data: {}",
                        destination.display()
                    );
                }
            }
            for (archived, destination) in components {
                if archived.exists() {
                    if destination.exists() {
                        bail!(
                            "interrupted recovery component conflicts with installed data: {}",
                            destination.display()
                        );
                    }
                    fs::rename(&archived, &destination).with_context(|| {
                        format!(
                            "restoring interrupted recovery component {} to {}",
                            archived.display(),
                            destination.display()
                        )
                    })?;
                }
            }
            sync_directory(data_dir)?;
            println!(
                "restored the original DB/WAL/SHM from interrupted recovery {}",
                directory.display()
            );
            return Ok(());
        }

        if live_is_original {
            let mut restored_sidecar = false;
            for suffix in ["-wal", "-shm"] {
                let destination = sqlite_sidecar(live, suffix);
                let archived = source_dir.join(
                    destination
                        .file_name()
                        .expect("database sidecar has filename"),
                );
                if archived.exists() && destination.exists() {
                    bail!(
                        "interrupted recovery has conflicting live and archived sidecars: {}",
                        destination.display()
                    );
                }
                if !destination.exists() && archived.exists() {
                    fs::rename(&archived, &destination)?;
                    restored_sidecar = true;
                }
            }
            if restored_sidecar {
                sync_directory(data_dir)?;
                println!(
                    "restored WAL/SHM from interrupted recovery {}",
                    directory.display()
                );
                return Ok(());
            }
        }
    }
    Ok(())
}

async fn recover(data_dir: &Path, _force: bool, resume: bool) -> Result<()> {
    if data_dir.join("storage.json").exists() || data_dir.join("storage-migration.json").exists() {
        bail!("hybrid recovery preserves its descriptor and payload bundle; use storage verification or restore");
    }

    // A recovery that races even one live SQLite connection cannot promise an
    // exact source generation. `--force` is retained for CLI compatibility but
    // deliberately cannot override this architectural boundary.
    ensure_app_quit(false)?;

    if resume {
        resume_recovery_offline(data_dir).await
    } else {
        recover_offline(data_dir).await
    }
}

/// Compare only the small WAL copies; never read/copy the entire main database.
fn same_file_bytes(left: &Path, right: &Path) -> Result<bool> {
    if fs::metadata(left)?.len() != fs::metadata(right)?.len() {
        return Ok(false);
    }
    let mut left = fs::File::open(left)?;
    let mut right = fs::File::open(right)?;
    let mut a = [0u8; 64 * 1024];
    let mut b = [0u8; 64 * 1024];
    loop {
        let count = left.read(&mut a)?;
        if count == 0 {
            return Ok(right.read(&mut b[..1])? == 0);
        }
        right.read_exact(&mut b[..count])?;
        if a[..count] != b[..count] {
            return Ok(false);
        }
    }
}

fn validate_resume_source(
    live: &Path,
    recovery_dir: &Path,
    manifest: &RecoveryManifest,
) -> Result<Vec<GenerationComponentFingerprint>> {
    let work = recovery_dir.join("working-copy/db.sqlite");
    let candidate = recovery_dir.join("candidate.sqlite");
    if manifest.schema_version != 1
        || !matches!(manifest.phase, RecoveryPhase::CandidateVerified)
        || fs::canonicalize(&manifest.live_path)? != fs::canonicalize(live)?
        || recovery_dir.join("source-generation").exists()
    {
        bail!(
            "cannot resume: candidate does not describe an uninstalled recovery for this database"
        );
    }
    for path in [live, work.as_path(), candidate.as_path()] {
        if !fs::symlink_metadata(path)?.file_type().is_file() {
            bail!(
                "cannot resume: expected a regular file at {}",
                path.display()
            );
        }
    }
    let before = generation_fingerprint(live)?;
    // v0.4.48 did not persist source fingerprints. Its original identity,
    // surviving hard link, and pre-recovery DB/WAL write times establish that
    // this is still its input. Fail closed if those facts cannot be established.
    if before[0].identity != manifest.original_identity
        || screenpipe_db::sqlite_file_identity(&work)? != manifest.original_identity
        || manifest.started_at_unix_ms == 0
        || before
            .iter()
            .filter(|part| part.name != "db.sqlite-shm")
            .any(|part| {
                !part.modified_unix_nanos.is_some_and(|modified| {
                    modified < u128::from(manifest.started_at_unix_ms) * 1_000_000
                })
            })
    {
        bail!("cannot resume: original DB/WAL changed since this recovery started, or its read-only input link is missing");
    }
    let live_wal = sqlite_sidecar(live, "-wal");
    let work_wal = sqlite_sidecar(&work, "-wal");
    if live_wal.exists() != work_wal.exists()
        || (live_wal.exists() && !same_file_bytes(&live_wal, &work_wal)?)
    {
        bail!("cannot resume: original WAL differs from the saved recovery input");
    }
    if manifest.candidate_identity.as_ref()
        != Some(&screenpipe_db::sqlite_file_identity(&candidate)?)
    {
        bail!("cannot resume: candidate file identity changed since verification");
    }
    if !recovery_source_is_current(&before, &generation_fingerprint(live)?) {
        bail!("cannot resume: source changed while checking the saved recovery input");
    }
    Ok(before)
}

async fn resume_recovery_offline(data_dir: &Path) -> Result<()> {
    let _lock = DbLock::acquire(data_dir, "recover --resume")?;
    let live = data_dir.join("db.sqlite");
    if !screenpipe_db::sqlite_quarantine_exists(&live) {
        bail!("cannot resume: no active database quarantine; no files were replaced");
    }
    for recovery_dir in newest_recovery_directories(data_dir)? {
        let verified = recovery_dir.join("recovery-manifest-candidate-verified.json");
        if !verified.exists() || !recovery_dir.join("candidate.sqlite").exists() {
            continue;
        }
        let manifest: RecoveryManifest = serde_json::from_slice(&fs::read(&verified)?)
            .context("reading verified recovery manifest")?;
        let fingerprint = validate_resume_source(&live, &recovery_dir, &manifest)?;
        println!("resuming verified candidate at {}", recovery_dir.display());
        println!(
            "rechecking the existing candidate; no database copy or page-level recovery needed"
        );
        return verify_and_install_candidate(data_dir, &recovery_dir, manifest, fingerprint).await;
    }
    bail!("no previously verified candidate available to resume; no new recovery was started and no database files were replaced")
}

/// Recover a database that was durably quarantined by the running app.
///
/// The desktop app calls this only on a fail-closed launch, before it starts
/// the server or capture. Requiring the durable marker keeps an accidental UI
/// invocation from turning an ordinary healthy database into a recovery job.
pub async fn recover_quarantined_database(data_dir: &Path) -> Result<()> {
    if data_dir.join("storage.json").exists() || data_dir.join("storage-migration.json").exists() {
        bail!("hybrid recovery preserves its descriptor and payload bundle; use storage verification or restore");
    }

    let live = data_dir.join("db.sqlite");
    if !screenpipe_db::sqlite_quarantine_exists(&live) {
        bail!(
            "database recovery is not required: no durable quarantine marker exists for {}",
            live.display()
        );
    }
    ensure_app_quit(false)?;
    recover_offline(data_dir).await
}

async fn recover_offline(data_dir: &Path) -> Result<()> {
    let live = data_dir.join("db.sqlite");
    fs::create_dir_all(data_dir)?;
    let _lock = DbLock::acquire(data_dir, "recover")?;
    reconcile_interrupted_recovery(data_dir, &live).await?;
    if !live.exists() {
        bail!("no database at {}", live.display());
    }
    // An explicit recovery request or an old marker is not proof that salvage
    // is needed. Diagnose before allocating a copy or requiring free space.
    let verified_damage = screenpipe_db::sqlite_confirmed_corruption_exists(&live);
    if !verified_damage {
        let token = screenpipe_db::begin_sqlite_verification(&live)?;
        match screenpipe_db::inspect_database_health(&live).await {
            Ok(_) => {
                screenpipe_db::admit_verified_sqlite_generation(token)?;
                println!("database verified healthy; no rebuild needed");
                return Ok(());
            }
            Err(screenpipe_db::DatabaseHealthError::Unavailable(error)) => {
                return Err(error).context("database access is unavailable; retry when storage is available, no rebuild started");
            }
            Err(screenpipe_db::DatabaseHealthError::Corrupt(detail)) => {
                screenpipe_db::quarantine_verified_sqlite_generation(token, Some(11), detail)?;
            }
        }
    }
    let original_identity = screenpipe_db::sqlite_file_identity(&live)
        .with_context(|| format!("identifying quarantined database {}", live.display()))?;
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let recovery_dir = data_dir.join(format!("db-recovery-{ts}-{}", std::process::id()));
    let work_dir = recovery_dir.join("working-copy");
    fs::create_dir_all(&work_dir)?;
    let work = work_dir.join("db.sqlite");
    let candidate = recovery_dir.join("candidate.sqlite");
    let manifest_path = recovery_dir.join(RECOVERY_MANIFEST_FILE);
    let manifest = RecoveryManifest {
        schema_version: 1,
        phase: RecoveryPhase::Preparing,
        live_path: live.clone(),
        original_identity: original_identity.clone(),
        candidate_identity: None,
        started_at_unix_ms: now_unix().saturating_mul(1000),
    };
    atomic_write_manifest(&manifest_path, &manifest)?;

    println!("preserving the quarantined DB/WAL/SHM and preparing a read-only recovery input");
    let source_fingerprint = prepare_readonly_generation(&live, &work)?;

    let source_counts = best_effort_counts(&work);
    let source_table_count = table_count(&work).unwrap_or(0);
    println!(
        "  source: {} tables, rows {{{}}}",
        source_table_count,
        format_counts(&source_counts)
    );

    println!("running SQLite page-level recovery against the working copy");
    screenpipe_sqlite_recovery::recover_database(&work, &candidate)
        .context("embedded page-level recovery failed; quarantined DB/WAL/SHM remain untouched")?;

    println!("rebuilding derived external-content FTS5 indexes from authoritative tables");
    let rebuilt_fts = screenpipe_db::rebuild_recovered_fts5_indexes(&candidate)
        .await
        .context("rebuilding recovered FTS5 indexes; live generation remains untouched")?;
    if !rebuilt_fts.is_empty() {
        println!("  rebuilt FTS5 indexes: {}", rebuilt_fts.join(", "));
    }

    verify_and_install_candidate(data_dir, &recovery_dir, manifest, source_fingerprint).await
}

async fn verify_and_install_candidate(
    data_dir: &Path,
    recovery_dir: &Path,
    mut manifest: RecoveryManifest,
    source_fingerprint: Vec<GenerationComponentFingerprint>,
) -> Result<()> {
    let live = data_dir.join("db.sqlite");
    let work_dir = recovery_dir.join("working-copy");
    let source_dir = recovery_dir.join("source-generation");
    let candidate = recovery_dir.join("candidate.sqlite");
    let manifest_path = recovery_dir.join(RECOVERY_MANIFEST_FILE);
    let source_table_count = table_count(&work_dir.join("db.sqlite")).unwrap_or(0);
    let marker = match screenpipe_db::read_sqlite_quarantine(&live) {
        Ok(marker) => marker,
        Err(error) => {
            eprintln!(
                "warning: active quarantine marker is unreadable ({error}); keeping it fail-closed until recovery completes"
            );
            None
        }
    };
    let mut forbidden_identities = vec![manifest.original_identity.clone()];
    if let Some(marker_identity) = marker.and_then(|marker| marker.file_identity) {
        if !forbidden_identities.contains(&marker_identity) {
            forbidden_identities.push(marker_identity);
        }
    }
    println!("verifying fresh file identity, integrity, foreign keys, and durable write canary");
    let candidate_verification =
        screenpipe_db::verify_fresh_sqlite_recovery_candidate(&candidate, &forbidden_identities)
            .await
            .context(
                "recovered candidate failed verification; live generation remains untouched",
            )?;

    let recovered_table_count = table_count(&candidate).unwrap_or(0);
    let recovered_counts = best_effort_counts(&candidate);
    println!(
        "  recovered: {} tables, rows {{{}}}",
        recovered_table_count,
        format_counts(&recovered_counts)
    );

    // Schema parity. FTS5 shadow tables can be rebuilt, but a >5% drop in
    // total table count indicates the recovered DB is materially worse.
    if (source_table_count as i64) - (recovered_table_count as i64)
        > (source_table_count as i64 / 20).max(2)
    {
        bail!(
            "recovered db is missing too many tables ({} → {}); refusing to swap. artifacts kept at {}.",
            source_table_count,
            recovered_table_count,
            recovery_dir.display(),
        );
    }

    if matches!(manifest.phase, RecoveryPhase::CandidateVerified) {
        if manifest.candidate_identity.as_ref() != Some(&candidate_verification.file_identity) {
            bail!("candidate identity changed during resumed verification; original remains untouched");
        }
    } else {
        manifest.candidate_identity = Some(candidate_verification.file_identity.clone());
        update_manifest(
            &manifest_path,
            &mut manifest,
            RecoveryPhase::CandidateVerified,
        )?;
    }

    let current_fingerprint = generation_fingerprint(&live)?;
    if !recovery_source_is_current(&source_fingerprint, &current_fingerprint) {
        bail!(
            "live DB/WAL/SHM changed while recovery was running; refusing to replace a newer generation \
             (source={source_fingerprint:?}, current={current_fingerprint:?})"
        );
    }

    println!(
        "archiving the exact original generation at {}",
        source_dir.display()
    );
    let moved = move_generation(&live, &source_dir)?;
    update_manifest(
        &manifest_path,
        &mut manifest,
        RecoveryPhase::OriginalArchived,
    )?;

    let install_result = (|| -> Result<()> {
        fs::rename(&candidate, &live).with_context(|| {
            format!(
                "installing verified candidate {} at {}",
                candidate.display(),
                live.display()
            )
        })?;
        sync_directory(data_dir)
    })();
    if let Err(error) = install_result {
        restore_generation(&moved)?;
        return Err(error).context("candidate install failed; original generation restored");
    }
    update_manifest(
        &manifest_path,
        &mut manifest,
        RecoveryPhase::CandidateInstalled,
    )?;

    let installed_verification =
        screenpipe_db::verify_fresh_sqlite_recovery_candidate(&live, &forbidden_identities).await;
    let installed_verification = match installed_verification {
        Ok(verification) if verification.file_identity == candidate_verification.file_identity => {
            verification
        }
        Ok(_) => {
            let error = anyhow::anyhow!(
                "installed database identity differs from the verified candidate identity"
            );
            rollback_failed_install(&live, &recovery_dir, &moved)?;
            update_manifest(
                &manifest_path,
                &mut manifest,
                RecoveryPhase::RestoredAfterFailure,
            )?;
            return Err(error).context("post-install verification failed; original restored");
        }
        Err(error) => {
            rollback_failed_install(&live, &recovery_dir, &moved)?;
            update_manifest(
                &manifest_path,
                &mut manifest,
                RecoveryPhase::RestoredAfterFailure,
            )?;
            return Err(error).context("post-install verification failed; original restored");
        }
    };
    debug_assert_eq!(
        installed_verification.file_identity,
        candidate_verification.file_identity
    );
    update_manifest(
        &manifest_path,
        &mut manifest,
        RecoveryPhase::InstalledVerified,
    )?;

    let resolved_marker = source_dir.join("resolved-quarantine.json");
    screenpipe_db::archive_resolved_sqlite_quarantine(
        &live,
        &resolved_marker,
        &installed_verification.file_identity,
        &forbidden_identities,
    )
    .context("installed DB verified, but durable quarantine could not be resolved")?;
    if let Err(error) = update_manifest(&manifest_path, &mut manifest, RecoveryPhase::Complete) {
        eprintln!(
            "warning: recovery completed and quarantine resolved, but the final audit manifest could not be written: {error}"
        );
    }

    // All input connections have closed and the verified replacement is live.
    // The exact original remains in source-generation; this directory contains
    // only our working link/copy and disposable sidecars.
    if let Err(error) = fs::remove_dir_all(&work_dir) {
        eprintln!("warning: recovery succeeded, but working files could not be removed: {error}");
    }

    println!();
    println!("recovery complete");
    println!("    preserved original: {}", source_dir.display());
    println!(
        "    fresh identity: {:?}",
        installed_verification.file_identity
    );
    println!();
    println!("    next: start screenpipe — it will rebuild WAL/SHM on first open.");
    println!("    once you've confirmed everything works, run:");
    println!("        screenpipe db cleanup --apply");
    println!("    to reclaim disk from the snapshot + pre-recover archives.");
    Ok(())
}

fn best_effort_counts(db_path: &Path) -> Vec<(&'static str, u64)> {
    let tables = ["frames", "audio_transcriptions", "ui_events"];
    let connection = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok();
    let mut out = Vec::new();
    for t in tables {
        let n = connection
            .as_ref()
            .and_then(|connection| {
                connection
                    .query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .ok()
            })
            .and_then(|count| u64::try_from(count).ok())
            .unwrap_or(0);
        out.push((t, n));
    }
    out
}

fn format_counts(counts: &[(&'static str, u64)]) -> String {
    counts
        .iter()
        .map(|(t, n)| format!("{t}={n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn table_count(db_path: &Path) -> Option<u64> {
    let connection = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let count = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .ok()?;
    u64::try_from(count).ok()
}

// ── cleanup ────────────────────────────────────────────────────────────

const FILE_PREFIXES: &[&str] = &[
    "db.sqlite.corrupt-",
    "db.sqlite.pre-recover-",
    "db.sqlite-wal.corrupt-",
    "db.sqlite-shm.corrupt-",
    "db.sqlite-wal.pre-recover-",
    "db.sqlite-shm.pre-recover-",
];
const FILE_EXACT: &[&str] = &["db_corrupted.sqlite", "db.sqlite.backup"];
const DIR_PREFIXES: &[&str] = &["db-recovery-", "db-hotfix-"];
/// Files we MUST NEVER delete even if a pattern accidentally matches.
const NEVER_DELETE: &[&str] = &[
    "db.sqlite",
    "db.sqlite-wal",
    "db.sqlite-shm",
    "db.sqlite.quarantine.json",
    "db.sqlite.quarantine.reserve.json",
    "store.bin",
    "auth.json",
    "connections.json",
    "settings.json",
];

async fn cleanup(data_dir: &Path, apply: bool, force: bool) -> Result<()> {
    ensure_app_quit(force)?;
    let _lock = DbLock::acquire(data_dir, "cleanup")?;
    let live = data_dir.join("db.sqlite");
    if apply && screenpipe_db::sqlite_quarantine_exists(&live) {
        bail!(
            "database quarantine is still active; refusing to delete recovery artifacts before a verified fresh generation resolves it"
        );
    }

    let mut targets: Vec<(PathBuf, u64, bool)> = Vec::new(); // (path, size, is_dir)
    let entries =
        fs::read_dir(data_dir).with_context(|| format!("failed to read {}", data_dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if NEVER_DELETE.contains(&name.as_str()) {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let is_file = metadata.is_file();
        let is_dir = metadata.is_dir();
        let matched = (is_file
            && (FILE_EXACT.contains(&name.as_str())
                || FILE_PREFIXES.iter().any(|p| name.starts_with(p))))
            || (is_dir && DIR_PREFIXES.iter().any(|p| name.starts_with(p)));
        if !matched {
            continue;
        }
        let size = if is_file {
            metadata.len()
        } else {
            dir_size(&path).unwrap_or(0)
        };
        targets.push((path, size, is_dir));
    }

    if targets.is_empty() {
        println!("nothing to clean up — no stale recovery/backup artifacts found.");
        return Ok(());
    }

    targets.sort_by(|a, b| b.1.cmp(&a.1));
    let total: u64 = targets.iter().map(|(_, n, _)| *n).sum();
    println!(
        "{} {} candidate(s), total {} MB:",
        if apply { "deleting" } else { "would delete" },
        targets.len(),
        total / 1_048_576,
    );
    for (path, size, _) in &targets {
        println!("  {:>8} MB  {}", size / 1_048_576, path.display());
    }

    if !apply {
        println!("\n(dry-run by default — pass `--apply` to actually delete)");
        return Ok(());
    }

    let audit = data_dir.join(".db_cleanup.log");
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&audit)
        .ok();
    let ts = chrono::Local::now().to_rfc3339();
    let mut failures = 0usize;
    let mut freed = 0u64;
    for (path, size, is_dir) in &targets {
        let result = if *is_dir {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        };
        match result {
            Ok(()) => {
                freed += size;
                if let Some(l) = log.as_mut() {
                    let _ = writeln!(l, "{ts}\tdeleted\t{}\t{} bytes", path.display(), size);
                }
            }
            Err(e) => {
                failures += 1;
                eprintln!("  failed to delete {}: {e}", path.display());
                if let Some(l) = log.as_mut() {
                    let _ = writeln!(l, "{ts}\tfailed\t{}\t{e}", path.display());
                }
            }
        }
    }
    println!(
        "\n✓ reclaimed {} MB ({} ok, {} failed)",
        freed / 1_048_576,
        targets.len() - failures,
        failures
    );
    if failures > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn dir_size(path: &Path) -> Result<u64> {
    let mut total = 0u64;
    for entry in fs::read_dir(path)?.flatten() {
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            total += dir_size(&entry.path()).unwrap_or(0);
        } else {
            total += metadata.len();
        }
    }
    Ok(total)
}

// ── unlock ─────────────────────────────────────────────────────────────

fn unlock(data_dir: &Path, force: bool) -> Result<()> {
    let path = data_dir.join(LOCK_FILE);
    if !path.exists() {
        println!("no lock file present at {}.", path.display());
        return Ok(());
    }
    let state = DbLock::inspect(&path);
    println!("lock file: {}", path.display());
    println!("state: {:?}", state);
    let safe = matches!(state, LockState::Stale | LockState::Unreadable);
    if !safe && !force {
        bail!(
            "lock looks live ({state:?}). pass --force to remove anyway. you should only do this if you're certain no `screenpipe db ...` is actually running."
        );
    }
    fs::remove_file(&path).context("failed to delete lock file")?;
    println!("✓ lock file removed.");
    Ok(())
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use std::io::{Seek, SeekFrom};

    #[test]
    fn recovery_cli_accepts_explicit_resume_without_force() {
        use clap::Parser;
        let cli = super::super::Cli::try_parse_from(["screenpipe", "db", "recover", "--resume"])
            .expect("customer resume command must parse");
        assert!(matches!(
            cli.command,
            super::super::Command::Db {
                subcommand: DbCommand::Recover {
                    resume: true,
                    force: false
                }
            }
        ));
    }

    fn write_generation(live: &Path) {
        fs::write(live, b"database-bytes").expect("write db");
        fs::write(sqlite_sidecar(live, "-wal"), b"wal-bytes").expect("write wal");
        fs::write(sqlite_sidecar(live, "-shm"), b"shm-bytes").expect("write shm");
    }

    fn test_manifest(live: &Path) -> RecoveryManifest {
        RecoveryManifest {
            schema_version: 1,
            phase: RecoveryPhase::Preparing,
            live_path: live.to_path_buf(),
            original_identity: screenpipe_db::sqlite_file_identity(live)
                .expect("original identity"),
            candidate_identity: None,
            started_at_unix_ms: 1,
        }
    }

    #[test]
    fn working_copy_preserves_the_exact_db_wal_shm_bytes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let live = dir.path().join("db.sqlite");
        let work_dir = dir.path().join("work");
        fs::create_dir(&work_dir).expect("work dir");
        let work = work_dir.join("db.sqlite");
        write_generation(&live);

        prepare_readonly_generation(&live, &work).expect("prepare read-only generation");
        assert_eq!(
            screenpipe_db::sqlite_file_identity(&live).unwrap(),
            screenpipe_db::sqlite_file_identity(&work).unwrap(),
            "main file must share storage, not allocate another database"
        );
        for suffix in ["", "-wal", "-shm"] {
            let source = if suffix.is_empty() {
                live.clone()
            } else {
                sqlite_sidecar(&live, suffix)
            };
            let copy = if suffix.is_empty() {
                work.clone()
            } else {
                sqlite_sidecar(&work, suffix)
            };
            assert_eq!(
                fs::read(&source).expect("source"),
                fs::read(&copy).expect("copy")
            );
            if !suffix.is_empty() {
                assert_ne!(
                    screenpipe_db::sqlite_file_identity(&source).unwrap(),
                    screenpipe_db::sqlite_file_identity(&copy).unwrap(),
                    "sidecar must be private"
                );
            }
        }
    }

    #[test]
    fn recovery_space_budget_counts_only_allocated_copies() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("db.sqlite");
        write_generation(&live);
        let mut source = generation_fingerprint(&live).unwrap();
        let gib = 1u64 << 30;
        for component in &mut source {
            component.length = match component.name.as_str() {
                "db.sqlite" => 122 * gib,
                "db.sqlite-wal" => 2 * gib,
                _ => gib,
            };
        }
        // Candidate: 125 GiB. Private WAL+SHM: 3 GiB. Reserve: 1 GiB.
        assert_eq!(recovery_space_required(&source, true), 129 * gib);
        assert_eq!(recovery_space_required(&source, false), 251 * gib);
    }

    #[test]
    fn preparation_does_not_overwrite_existing_working_path() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("db.sqlite");
        let work = dir.path().join("working.sqlite");
        write_generation(&live);
        fs::write(&work, b"existing-working-file").unwrap();
        assert!(prepare_readonly_generation(&live, &work).is_err());
        assert_eq!(fs::read(&work).unwrap(), b"existing-working-file");
        assert_eq!(fs::read(&live).unwrap(), b"database-bytes");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_main_database_uses_private_copy_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let actual = dir.path().join("actual.sqlite");
        let live = dir.path().join("db.sqlite");
        let work = dir.path().join("working.sqlite");
        fs::write(&actual, b"original-db").unwrap();
        std::os::unix::fs::symlink(&actual, &live).unwrap();
        prepare_readonly_generation(&live, &work).expect("copy fallback");
        assert_eq!(fs::read(&work).unwrap(), b"original-db");
        assert_ne!(
            screenpipe_db::sqlite_file_identity(&actual).unwrap(),
            screenpipe_db::sqlite_file_identity(&work).unwrap()
        );
        fs::write(&work, b"private-work").unwrap();
        assert_eq!(fs::read(&actual).unwrap(), b"original-db");
    }

    #[test]
    fn recovery_tolerates_only_a_new_zero_byte_wal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let live = dir.path().join("db.sqlite");
        fs::write(&live, b"database-bytes").expect("write db");
        let source = generation_fingerprint(&live).expect("source fingerprint");

        fs::write(sqlite_sidecar(&live, "-wal"), b"").expect("empty wal");
        let empty_wal = generation_fingerprint(&live).expect("empty wal fingerprint");
        assert!(recovery_source_is_current(&source, &empty_wal));

        fs::write(sqlite_sidecar(&live, "-wal"), b"new-write").expect("nonempty wal");
        let changed = generation_fingerprint(&live).expect("changed fingerprint");
        assert!(!recovery_source_is_current(&source, &changed));
    }

    #[test]
    fn recovery_accepts_shm_timestamp_only_but_rejects_generation_changes() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("db.sqlite");
        write_generation(&live);
        let source = generation_fingerprint(&live).unwrap();
        for index in 0..3 {
            let mut current = source.clone();
            current[index].modified_unix_nanos = Some(42);
            assert_eq!(recovery_source_is_current(&source, &current), index == 2);
            current = source.clone();
            current[index].length += 1;
            assert!(!recovery_source_is_current(&source, &current));
            current = source.clone();
            current[index].identity = source[(index + 1) % 3].identity.clone();
            assert!(!recovery_source_is_current(&source, &current));
            current = source.clone();
            current.remove(index);
            assert!(!recovery_source_is_current(&source, &current));
        }
    }

    fn set_modified(path: &Path, time: SystemTime) {
        OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(time)
            .unwrap();
    }

    /// Create the exact v0.4.48 on-disk state after candidate verification and
    /// before installation, including a row present only in the copied WAL.
    async fn resumable_fixture(data_dir: &Path) -> (PathBuf, RecoveryManifest) {
        fs::create_dir_all(data_dir).unwrap();
        let seed = data_dir.join("seed.sqlite");
        let writer = Connection::open(&seed).unwrap();
        writer
            .execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; \
             CREATE TABLE records (id INTEGER PRIMARY KEY, value TEXT); \
             PRAGMA wal_checkpoint(TRUNCATE); \
             INSERT INTO records VALUES (1, 'only-in-wal');",
            )
            .unwrap();
        let live = data_dir.join("db.sqlite");
        for suffix in ["", "-wal", "-shm"] {
            let destination = data_dir.join(format!("db.sqlite{suffix}"));
            fs::copy(data_dir.join(format!("seed.sqlite{suffix}")), &destination).unwrap();
            set_modified(
                &destination,
                UNIX_EPOCH + Duration::from_secs(now_unix() - 60),
            );
        }
        drop(writer);
        let directory = data_dir.join("db-recovery-20260906-000000-48");
        fs::create_dir_all(directory.join("working-copy")).unwrap();
        let work = directory.join("working-copy/db.sqlite");
        let candidate = directory.join("candidate.sqlite");
        let mut manifest = test_manifest(&live);
        manifest.started_at_unix_ms = now_unix() * 1000;
        atomic_write_manifest(&directory.join(RECOVERY_MANIFEST_FILE), &manifest).unwrap();
        prepare_readonly_generation(&live, &work).unwrap();
        screenpipe_sqlite_recovery::recover_database(&work, &candidate).unwrap();
        let verification = screenpipe_db::verify_fresh_sqlite_recovery_candidate(
            &candidate,
            &[manifest.original_identity.clone()],
        )
        .await
        .unwrap();
        manifest.candidate_identity = Some(verification.file_identity);
        update_manifest(
            &directory.join(RECOVERY_MANIFEST_FILE),
            &mut manifest,
            RecoveryPhase::CandidateVerified,
        )
        .unwrap();
        screenpipe_db::persist_sqlite_quarantine(&live, Some(11), "resume fixture").unwrap();
        (directory, manifest)
    }

    fn triplet_bytes(live: &Path) -> Vec<Vec<u8>> {
        [
            live.to_path_buf(),
            sqlite_sidecar(live, "-wal"),
            sqlite_sidecar(live, "-shm"),
        ]
        .iter()
        .map(|path| fs::read(path).unwrap())
        .collect()
    }

    #[tokio::test]
    async fn resume_048_candidate_after_shm_timestamp_change_without_recovering_again() {
        let dir = tempfile::tempdir().unwrap();
        let (recovery_dir, manifest) = resumable_fixture(dir.path()).await;
        let live = dir.path().join("db.sqlite");
        let originals = triplet_bytes(&live);
        set_modified(&sqlite_sidecar(&live, "-shm"), SystemTime::now());
        resume_recovery_offline(dir.path())
            .await
            .expect("resume verified 0.4.48 candidate");
        assert_eq!(
            Some(screenpipe_db::sqlite_file_identity(&live).unwrap()),
            manifest.candidate_identity
        );
        assert_eq!(
            newest_recovery_directories(dir.path()).unwrap(),
            vec![recovery_dir.clone()]
        );
        assert!(!screenpipe_db::sqlite_quarantine_exists(&live));
        assert_eq!(
            triplet_bytes(&recovery_dir.join("source-generation/db.sqlite")),
            originals
        );
        let connection =
            Connection::open_with_flags(&live, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT value FROM records WHERE id=1", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "only-in-wal"
        );
    }

    #[tokio::test]
    async fn resume_rejects_changed_db_or_wal_and_preserves_originals() {
        for suffix in ["", "-wal"] {
            let dir = tempfile::tempdir().unwrap();
            let (recovery_dir, _) = resumable_fixture(dir.path()).await;
            let live = dir.path().join("db.sqlite");
            set_modified(
                &dir.path().join(format!("db.sqlite{suffix}")),
                SystemTime::now(),
            );
            let originals = triplet_bytes(&live);
            let error = resume_recovery_offline(dir.path()).await.unwrap_err();
            assert!(error.to_string().contains("changed since"), "{error:#}");
            assert_eq!(triplet_bytes(&live), originals);
            assert!(recovery_dir.join("candidate.sqlite").exists());
            assert!(screenpipe_db::sqlite_quarantine_exists(&live));
        }
    }

    #[tokio::test]
    async fn resume_checks_wal_bytes_even_with_unchanged_size_and_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let (_, manifest) = resumable_fixture(dir.path()).await;
        let live = dir.path().join("db.sqlite");
        let wal = sqlite_sidecar(&live, "-wal");
        let modified = fs::metadata(&wal).unwrap().modified().unwrap();
        let mut bytes = fs::read(&wal).unwrap();
        bytes[40] ^= 0xff;
        fs::write(&wal, bytes).unwrap();
        set_modified(&wal, modified);
        let originals = triplet_bytes(&live);
        let error = resume_recovery_offline(dir.path()).await.unwrap_err();
        assert!(error.to_string().contains("WAL differs"), "{error:#}");
        assert_eq!(triplet_bytes(&live), originals);
        assert_eq!(
            screenpipe_db::sqlite_file_identity(&live).unwrap(),
            manifest.original_identity
        );
    }

    #[tokio::test]
    async fn resume_revalidates_candidate_and_never_swaps_a_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let (recovery_dir, _) = resumable_fixture(dir.path()).await;
        let live = dir.path().join("db.sqlite");
        let originals = triplet_bytes(&live);
        fs::write(recovery_dir.join("candidate.sqlite"), b"not a database").unwrap();
        let error = resume_recovery_offline(dir.path()).await.unwrap_err();
        assert!(
            error.to_string().contains("candidate failed verification"),
            "{error:#}"
        );
        assert_eq!(triplet_bytes(&live), originals);
        assert!(!recovery_dir.join("source-generation").exists());
        assert!(screenpipe_db::sqlite_quarantine_exists(&live));
    }

    #[tokio::test]
    async fn resume_rejects_replaced_candidate_or_missing_input_link() {
        for target in ["candidate.sqlite", "working-copy/db.sqlite"] {
            let dir = tempfile::tempdir().unwrap();
            let (recovery_dir, _) = resumable_fixture(dir.path()).await;
            let live = dir.path().join("db.sqlite");
            let originals = triplet_bytes(&live);
            let target = recovery_dir.join(target);
            let saved = recovery_dir.join("saved.sqlite");
            fs::rename(&target, &saved).unwrap();
            fs::copy(&saved, &target).unwrap();
            assert!(resume_recovery_offline(dir.path()).await.is_err());
            assert_eq!(triplet_bytes(&live), originals);
            assert!(screenpipe_db::sqlite_quarantine_exists(&live));
        }
    }

    #[tokio::test]
    async fn resume_without_verified_candidate_does_not_start_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("db.sqlite");
        write_generation(&live);
        screenpipe_db::persist_sqlite_quarantine(&live, Some(11), "resume fixture").unwrap();
        let originals = triplet_bytes(&live);
        let error = resume_recovery_offline(dir.path()).await.unwrap_err();
        assert!(error
            .to_string()
            .contains("no previously verified candidate"));
        assert!(newest_recovery_directories(dir.path()).unwrap().is_empty());
        assert_eq!(triplet_bytes(&live), originals);
    }

    #[test]
    fn archive_failure_rolls_every_component_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        let live = dir.path().join("db.sqlite");
        let archive = dir.path().join("archive");
        fs::create_dir(&archive).expect("archive dir");
        write_generation(&live);
        fs::write(archive.join("db.sqlite-wal"), b"collision").expect("collision");

        move_generation(&live, &archive).expect_err("collision must abort archive");
        assert_eq!(fs::read(&live).expect("restored db"), b"database-bytes");
        assert_eq!(
            fs::read(sqlite_sidecar(&live, "-wal")).expect("live wal"),
            b"wal-bytes"
        );
        assert_eq!(
            fs::read(archive.join("db.sqlite-wal")).expect("collision retained"),
            b"collision"
        );
    }

    #[test]
    fn restart_restores_a_generation_interrupted_after_archive() {
        let dir = tempfile::tempdir().expect("tempdir");
        let data_dir = dir.path();
        let live = data_dir.join("db.sqlite");
        write_generation(&live);
        let manifest = test_manifest(&live);
        let recovery = data_dir.join("db-recovery-20260801-1");
        let source = recovery.join("source-generation");
        fs::create_dir_all(&source).expect("source dir");
        atomic_write_manifest(&recovery.join(RECOVERY_MANIFEST_FILE), &manifest).expect("manifest");
        move_generation(&live, &source).expect("archive source");
        assert!(!live.exists());

        restore_interrupted_swap(data_dir, &live).expect("restart restoration");
        assert_eq!(fs::read(&live).expect("restored db"), b"database-bytes");
        assert_eq!(
            fs::read(sqlite_sidecar(&live, "-wal")).expect("restored wal"),
            b"wal-bytes"
        );
        assert_eq!(
            fs::read(sqlite_sidecar(&live, "-shm")).expect("restored shm"),
            b"shm-bytes"
        );
    }

    #[test]
    fn restart_repairs_a_swap_interrupted_between_sidecars() {
        let dir = tempfile::tempdir().expect("tempdir");
        let data_dir = dir.path();
        let live = data_dir.join("db.sqlite");
        write_generation(&live);
        let manifest = test_manifest(&live);
        let recovery = data_dir.join("db-recovery-20260801-2");
        let source = recovery.join("source-generation");
        fs::create_dir_all(&source).expect("source dir");
        atomic_write_manifest(&recovery.join(RECOVERY_MANIFEST_FILE), &manifest).expect("manifest");
        fs::rename(sqlite_sidecar(&live, "-wal"), source.join("db.sqlite-wal"))
            .expect("simulate interrupted wal move");

        restore_interrupted_swap(data_dir, &live).expect("restart restoration");
        assert_eq!(
            fs::read(sqlite_sidecar(&live, "-wal")).expect("restored wal"),
            b"wal-bytes"
        );
        assert!(live.exists());
    }

    #[tokio::test]
    async fn startup_restores_committed_sidecars_before_database_admission() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("db.sqlite");
        write_generation(&live);
        let manifest = test_manifest(&live);
        let recovery = dir.path().join("db-recovery-interrupted");
        let source = recovery.join("source-generation");
        fs::create_dir_all(&source).unwrap();
        atomic_write_manifest(&recovery.join(RECOVERY_MANIFEST_FILE), &manifest).unwrap();
        fs::rename(sqlite_sidecar(&live, "-wal"), source.join("db.sqlite-wal")).unwrap();

        let guard = prepare_database_startup(dir.path()).await.unwrap();
        assert_eq!(
            fs::read(sqlite_sidecar(&live, "-wal")).unwrap(),
            b"wal-bytes"
        );
        assert!(DbLock::acquire_inner(dir.path(), "competing repair", false).is_err());
        drop(guard);
        let _next_start = prepare_database_startup(dir.path()).await.unwrap();
    }

    #[tokio::test]
    async fn startup_rejects_conflicting_recovery_sidecars_without_overwriting_either() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("db.sqlite");
        write_generation(&live);
        let manifest = test_manifest(&live);
        let recovery = dir.path().join("db-recovery-conflict");
        let source = recovery.join("source-generation");
        fs::create_dir_all(&source).unwrap();
        atomic_write_manifest(&recovery.join(RECOVERY_MANIFEST_FILE), &manifest).unwrap();
        fs::write(source.join("db.sqlite-wal"), b"different-committed-data").unwrap();
        assert!(prepare_database_startup(dir.path()).await.is_err());
        assert_eq!(
            fs::read(sqlite_sidecar(&live, "-wal")).unwrap(),
            b"wal-bytes"
        );
        assert_eq!(
            fs::read(source.join("db.sqlite-wal")).unwrap(),
            b"different-committed-data"
        );
    }

    #[tokio::test]
    async fn startup_verifies_an_installed_candidate_before_completing_interrupted_repair() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("db.sqlite");
        {
            let connection = Connection::open(&live).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE records(id INTEGER PRIMARY KEY); INSERT INTO records VALUES(1);",
                )
                .unwrap();
        }
        let mut manifest = test_manifest(&live);
        screenpipe_db::persist_sqlite_quarantine(&live, Some(10), "legacy interrupted repair")
            .unwrap();
        let recovery = dir.path().join("db-recovery-installed");
        let source = recovery.join("source-generation");
        fs::create_dir_all(&source).unwrap();
        move_generation(&live, &source).unwrap();
        {
            let connection = Connection::open(&live).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE records(id INTEGER PRIMARY KEY); INSERT INTO records VALUES(1);",
                )
                .unwrap();
        }
        manifest.phase = RecoveryPhase::Preparing;
        atomic_write_manifest(&recovery.join(RECOVERY_MANIFEST_FILE), &manifest).unwrap();
        manifest.candidate_identity = Some(screenpipe_db::sqlite_file_identity(&live).unwrap());
        update_manifest(
            &recovery.join(RECOVERY_MANIFEST_FILE),
            &mut manifest,
            RecoveryPhase::CandidateInstalled,
        )
        .unwrap();

        let _guard = prepare_database_startup(dir.path()).await.unwrap();
        assert!(!screenpipe_db::sqlite_quarantine_exists(&live));
        let completed = read_latest_recovery_manifest(&recovery).unwrap().unwrap();
        assert!(matches!(completed.phase, RecoveryPhase::Complete));
        assert!(source.join("db.sqlite").exists());
        let connection = Connection::open(&live).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM records", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn startup_does_not_create_a_database_when_recovery_manifest_is_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let recovery = dir.path().join("db-recovery-unreadable");
        fs::create_dir_all(&recovery).unwrap();
        fs::write(recovery.join(RECOVERY_MANIFEST_FILE), b"interrupted json").unwrap();
        assert!(prepare_database_startup(dir.path()).await.is_err());
        assert!(!dir.path().join("db.sqlite").exists());
    }

    #[test]
    fn recovery_lock_recognizes_the_current_process_on_this_platform() {
        assert!(pid_alive(std::process::id()));
    }

    #[tokio::test]
    async fn end_to_end_recovery_salvages_corruption_installs_fresh_inode_and_archives_original() {
        let dir = tempfile::tempdir().expect("tempdir");
        let data_dir = dir.path();
        let live = data_dir.join("db.sqlite");
        let create = Connection::open(&live).expect("create source database");
        create
            .execute_batch(
                "PRAGMA page_size = 4096; \
                 CREATE TABLE records (id INTEGER PRIMARY KEY, value TEXT NOT NULL); \
                 WITH RECURSIVE n(value) AS ( \
                   SELECT 1 UNION ALL SELECT value + 1 FROM n WHERE value < 200 \
                 ) \
                 INSERT INTO records(value) SELECT printf('row-%04d', value) FROM n; \
                 CREATE INDEX records_value_idx ON records(value); \
                 CREATE TABLE frames (\
                    id INTEGER PRIMARY KEY, full_text TEXT, app_name TEXT, \
                    window_name TEXT, browser_url TEXT\
                 ); \
                 CREATE VIRTUAL TABLE frames_fts USING fts5(\
                    full_text, app_name, window_name, browser_url, \
                    content='frames', content_rowid='id', tokenize='unicode61'\
                 ); \
                 CREATE TRIGGER frames_ai AFTER INSERT ON frames \
                 WHEN NEW.full_text IS NOT NULL AND NEW.full_text != '' BEGIN \
                    INSERT INTO frames_fts(\
                        rowid, full_text, app_name, window_name, browser_url\
                    ) VALUES (\
                        NEW.id, NEW.full_text, COALESCE(NEW.app_name, ''), \
                        COALESCE(NEW.window_name, ''), COALESCE(NEW.browser_url, '')\
                    ); \
                 END; \
                 CREATE TRIGGER frames_au \
                 AFTER UPDATE OF full_text, app_name, window_name, browser_url ON frames BEGIN \
                    INSERT INTO frames_fts(\
                        frames_fts, rowid, full_text, app_name, window_name, browser_url\
                    ) SELECT 'delete', OLD.id, OLD.full_text, COALESCE(OLD.app_name, ''), \
                             COALESCE(OLD.window_name, ''), COALESCE(OLD.browser_url, '') \
                      WHERE OLD.full_text IS NOT NULL AND OLD.full_text != ''; \
                    INSERT INTO frames_fts(\
                        rowid, full_text, app_name, window_name, browser_url\
                    ) SELECT NEW.id, NEW.full_text, COALESCE(NEW.app_name, ''), \
                             COALESCE(NEW.window_name, ''), COALESCE(NEW.browser_url, '') \
                      WHERE NEW.full_text IS NOT NULL AND NEW.full_text != ''; \
                 END; \
                 INSERT INTO frames VALUES \
                    (1, 'recoverable frame text', 'test-app', '', ''), \
                    (2, '', 'metadata-only-app', '', '');",
            )
            .expect("seed source database");
        let index_root: i64 = create
            .query_row(
                "SELECT rootpage FROM sqlite_schema WHERE name = 'records_value_idx'",
                [],
                |row| row.get(0),
            )
            .expect("index root page");
        let fts_data_root: i64 = create
            .query_row(
                "SELECT rootpage FROM sqlite_schema WHERE name = 'frames_fts_data'",
                [],
                |row| row.get(0),
            )
            .expect("FTS data root page");
        drop(create);

        let mut source_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&live)
            .expect("open source database bytes");
        source_file
            .seek(SeekFrom::Start(((index_root - 1) * 4096 + 100) as u64))
            .expect("seek index page");
        source_file
            .write_all(&[0xff; 512])
            .expect("corrupt index page");
        source_file
            .seek(SeekFrom::Start(((fts_data_root - 1) * 4096 + 100) as u64))
            .expect("seek FTS shadow page");
        source_file
            .write_all(&[0xa5; 512])
            .expect("corrupt FTS shadow page");
        source_file.sync_all().expect("sync source corruption");
        drop(source_file);

        let original_bytes = fs::read(&live).expect("read original bytes");
        let original_identity = screenpipe_db::sqlite_file_identity(&live).expect("old identity");
        screenpipe_db::prepare_sqlite_quarantine_reserve(&live).expect("prepare marker reserve");
        screenpipe_db::persist_sqlite_quarantine(&live, Some(11), "test corruption")
            .expect("persist quarantine");

        // This isolated fixture has no live owners. Exercise offline recovery
        // without depending on whether the developer's unrelated app uses 3030.
        recover_offline(data_dir)
            .await
            .expect("end-to-end offline recovery");

        let replacement_identity =
            screenpipe_db::sqlite_file_identity(&live).expect("replacement identity");
        assert_ne!(replacement_identity, original_identity);
        assert!(!screenpipe_db::sqlite_quarantine_exists(&live));
        let recovered = Connection::open_with_flags(&live, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("open recovered database");
        let recovered_count: i64 = recovered
            .query_row("SELECT COUNT(*) FROM records", [], |row| row.get(0))
            .expect("count recovered rows");
        assert_eq!(recovered_count, 200);
        assert_eq!(
            recovered
                .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
                .expect("verify recovered integrity"),
            "ok"
        );
        let frame_hits: i64 = recovered
            .query_row(
                "SELECT COUNT(*) FROM frames_fts WHERE frames_fts MATCH 'recoverable'",
                [],
                |row| row.get(0),
            )
            .expect("query rebuilt FTS index");
        assert_eq!(frame_hits, 1);
        drop(recovered);

        let writable = Connection::open(&live).expect("reopen recovered database for canary");
        writable
            .execute(
                "INSERT INTO frames(id, full_text, app_name, window_name, browser_url) \
                 VALUES (3, 'postrecoverycanary', '', '', '')",
                [],
            )
            .expect("write through normal post-recovery FTS trigger");
        let post_recovery_hits: i64 = writable
            .query_row(
                "SELECT COUNT(*) FROM frames_fts \
                 WHERE frames_fts MATCH 'postrecoverycanary'",
                [],
                |row| row.get(0),
            )
            .expect("query post-recovery FTS trigger write");
        assert_eq!(post_recovery_hits, 1);
        writable
            .execute(
                "INSERT INTO frames_fts(frames_fts) VALUES('integrity-check')",
                [],
            )
            .expect("FTS stays valid after post-recovery write");
        drop(writable);

        let recovery_dir = newest_recovery_directories(data_dir)
            .expect("recovery dirs")
            .into_iter()
            .next()
            .expect("recovery directory");
        assert_eq!(
            fs::read(recovery_dir.join("source-generation/db.sqlite"))
                .expect("preserved source database"),
            original_bytes,
            "recovery must archive the exact original database bytes"
        );
        assert!(recovery_dir
            .join("source-generation/resolved-quarantine.json")
            .exists());
        assert!(recovery_dir
            .join("recovery-manifest-complete.json")
            .exists());
        assert!(!recovery_dir.join("working-copy").exists());
    }

    #[tokio::test]
    async fn app_recovery_refuses_a_database_without_a_quarantine_marker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let live = dir.path().join("db.sqlite");
        Connection::open(&live).expect("create healthy database");

        let error = recover_quarantined_database(dir.path())
            .await
            .expect_err("healthy unmarked database must not enter app recovery");

        assert!(error.to_string().contains("no durable quarantine marker"));
        assert!(live.exists());
        assert!(newest_recovery_directories(dir.path())
            .expect("list recovery directories")
            .is_empty());
    }

    #[tokio::test]
    async fn linked_recovery_installs_wal_rows_and_archives_unchanged_triplet() {
        let dir = tempfile::tempdir().unwrap();
        let seed = dir.path().join("seed.sqlite");
        let writer = Connection::open(&seed).unwrap();
        writer
            .execute_batch(
                "PRAGMA page_size=4096; PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; \
             CREATE TABLE records (id INTEGER PRIMARY KEY, value TEXT); \
             CREATE TABLE indexed_records (id INTEGER PRIMARY KEY, value TEXT); \
             CREATE INDEX damaged_index ON indexed_records(value); \
             WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<200) \
             INSERT INTO indexed_records SELECT x, printf('value-%04d',x) FROM n; \
             PRAGMA wal_checkpoint(TRUNCATE); \
             INSERT INTO records VALUES (1, 'only-in-wal');",
            )
            .unwrap();
        let index_root: i64 = writer
            .query_row(
                "SELECT rootpage FROM sqlite_schema WHERE name='damaged_index'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let data_dir = dir.path().join("data");
        fs::create_dir(&data_dir).unwrap();
        let live = data_dir.join("db.sqlite");
        let mut originals = Vec::new();
        for suffix in ["", "-wal", "-shm"] {
            let mut bytes = fs::read(dir.path().join(format!("seed.sqlite{suffix}"))).unwrap();
            if suffix.is_empty() {
                let start = ((index_root - 1) * 4096 + 100) as usize;
                bytes[start..start + 512].fill(0xff);
            }
            fs::write(data_dir.join(format!("db.sqlite{suffix}")), &bytes).unwrap();
        }
        drop(writer);
        let identity = screenpipe_db::sqlite_file_identity(&live).unwrap();
        // A legacy error code alone must not rebuild a healthy database. Prove
        // real index damage while the WAL-only recording row remains intact.
        let token = screenpipe_db::begin_sqlite_verification(&live).unwrap();
        let damage = match screenpipe_db::inspect_database_health(&live).await {
            Err(screenpipe_db::DatabaseHealthError::Corrupt(detail)) => detail,
            result => panic!("expected verified index damage, got {result:?}"),
        };
        screenpipe_db::quarantine_verified_sqlite_generation(token, Some(11), damage).unwrap();
        for suffix in ["", "-wal", "-shm"] {
            originals.push((
                suffix,
                fs::read(data_dir.join(format!("db.sqlite{suffix}"))).unwrap(),
            ));
        }
        recover_offline(&data_dir)
            .await
            .expect("recover and install WAL generation");
        assert!(!screenpipe_db::sqlite_quarantine_exists(&live));
        assert_ne!(
            screenpipe_db::sqlite_file_identity(&live).unwrap(),
            identity
        );
        let recovered =
            Connection::open_with_flags(&live, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let value: String = recovered
            .query_row("SELECT value FROM records WHERE id=1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(value, "only-in-wal");
        let recovery_dir = newest_recovery_directories(&data_dir).unwrap().remove(0);
        assert!(!recovery_dir.join("working-copy").exists());
        for (suffix, bytes) in originals {
            assert_eq!(
                fs::read(
                    recovery_dir
                        .join("source-generation")
                        .join(format!("db.sqlite{suffix}"))
                )
                .unwrap(),
                bytes
            );
        }
    }

    #[tokio::test]
    async fn cleanup_cannot_delete_evidence_while_quarantine_is_active() {
        let dir = tempfile::tempdir().expect("tempdir");
        let live = dir.path().join("db.sqlite");
        fs::write(&live, b"quarantined database").expect("write db");
        screenpipe_db::prepare_sqlite_quarantine_reserve(&live).expect("prepare reserve");
        screenpipe_db::persist_sqlite_quarantine(&live, Some(522), "test I/O fault")
            .expect("persist quarantine");
        let check_error =
            integrity_check(&live).expect_err("db check must not open the quarantined generation");
        assert!(check_error.to_string().contains("refusing to open"));
        let evidence = dir.path().join("db-recovery-evidence");
        fs::create_dir(&evidence).expect("evidence dir");
        fs::write(evidence.join("db.sqlite"), b"evidence").expect("evidence file");

        let error = cleanup(dir.path(), true, true)
            .await
            .expect_err("active quarantine must protect recovery evidence");
        assert!(error.to_string().contains("quarantine is still active"));
        assert!(evidence.exists());
    }
}
