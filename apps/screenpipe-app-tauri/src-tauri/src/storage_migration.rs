// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use crate::{recording::RecordingState, store::SettingsStore};
use screenpipe_db::storage::{migration_report, MigrationProgress, StorageDescriptor};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::Instant,
};
use tauri::{Emitter, Manager, State};

// Shared by every webview and recorder restart; renewed only by a new app process.
static APP_SESSION_ID: LazyLock<String> = LazyLock::new(|| uuid::Uuid::new_v4().to_string());

const MIGRATION_ERROR_FILE: &str = "storage-migration-error.txt";
const INTERRUPTED_MIGRATION: &str =
    "The previous storage migration did not finish. Automatic retries are disabled; retry explicitly.";

fn saved_migration_error(root: &Path) -> Option<String> {
    match std::fs::read_to_string(root.join(MIGRATION_ERROR_FILE)) {
        Ok(error) if !error.trim().is_empty() => Some(error),
        Ok(_) => Some(INTERRUPTED_MIGRATION.into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Also protect interrupted attempts made before the durable block
            // existed. Ordinary startup can pause this journal and record.
            root.join("storage-migration.json")
                .exists()
                .then(|| INTERRUPTED_MIGRATION.into())
        }
        Err(error) => Some(format!(
            "Could not read the previous migration result: {error}. Automatic retries are disabled."
        )),
    }
}

fn save_migration_error(root: &Path, error: &str) -> Result<(), String> {
    crate::store::durable_write(&root.join(MIGRATION_ERROR_FILE), error.as_bytes())
        .map_err(|e| format!("Could not save the migration retry block: {e}"))
}

fn clear_migration_error(root: &Path) -> Result<(), String> {
    match std::fs::remove_file(root.join(MIGRATION_ERROR_FILE)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Could not clear the migration retry block: {error}"
        )),
    }
}

#[derive(Default, Clone)]
struct Operation {
    root: Option<PathBuf>,
    busy: bool,
    message: String,
    error: Option<String>,
    started_at: Option<Instant>,
    elapsed_seconds: u64,
    completed_records: Option<u64>,
    total_records: Option<u64>,
    bytes_saved: Option<u64>,
    available_bytes: Option<u64>,
    completed: bool,
    background: bool,
}

impl Operation {
    fn activity(&self) -> StorageMigrationActivity {
        StorageMigrationActivity {
            root: self.root.as_ref().map(|root| root.display().to_string()),
            busy: self.busy,
            message: self.message.clone(),
            error: self.error.clone(),
            elapsed_seconds: self
                .started_at
                .map_or(self.elapsed_seconds, |start| start.elapsed().as_secs()),
            completed_records: self.completed_records,
            total_records: self.total_records,
            bytes_saved: self.bytes_saved,
            available_bytes: self.available_bytes,
            // A hidden migration can remove the old recovery copy.
            completed: self.completed && !self.background,
        }
    }
}

#[derive(Default)]
pub struct StorageMigrationState(Mutex<Operation>);

#[derive(Clone, Serialize, specta::Type)]
pub struct StorageMigrationActivity {
    pub root: Option<String>,
    pub busy: bool,
    pub message: String,
    pub error: Option<String>,
    pub elapsed_seconds: u64,
    pub completed_records: Option<u64>,
    pub total_records: Option<u64>,
    pub bytes_saved: Option<u64>,
    pub available_bytes: Option<u64>,
    pub completed: bool,
}

#[tauri::command]
#[specta::specta]
pub fn get_storage_migration_activity(
    state: State<'_, StorageMigrationState>,
) -> StorageMigrationActivity {
    let operation = state.0.lock().unwrap_or_else(|e| e.into_inner());
    operation.activity()
}

fn update_operation(app: &tauri::AppHandle, update: impl FnOnce(&mut Operation)) {
    let state = app.state::<StorageMigrationState>();
    let activity = {
        let mut operation = state.0.lock().unwrap_or_else(|e| e.into_inner());
        update(&mut operation);
        operation.activity()
    };
    let _ = app.emit("storage-migration-activity", activity);
}

#[derive(Clone, Serialize, specta::Type)]
pub struct StorageMigrationStatus {
    pub root: String,
    pub app_session_id: String,
    pub busy: bool,
    pub message: String,
    pub error: Option<String>,
    pub pending: bool,
    pub in_place: bool,
    pub completed: bool,
    pub using_new_storage: bool,
    pub generation: Option<String>,
    pub source_bytes: u64,
    pub migrated_bytes: Option<u64>,
    pub bytes_saved: Option<u64>,
    pub available_bytes: Option<u64>,
    pub can_migrate: bool,
    pub can_cancel: bool,
    pub can_delete_source: bool,
    pub blocked_reason: Option<String>,
}

fn should_start_hidden_ui_migration(
    status: &StorageMigrationStatus,
    ui_hidden: bool,
    authorized: bool,
    recorder_ready: bool,
) -> bool {
    ui_hidden
        && authorized
        && recorder_ready
        && !status.busy
        // Failures and interrupted attempts stay blocked across app restarts.
        && status.error.is_none()
        && !status.pending
        && status.blocked_reason.is_none()
        && (status.can_migrate || status.can_delete_source)
}

/// Called by the native policy watcher, including when no webview exists.
/// Completion, failure and cleanup are recorded in logs without opening UI.
pub(crate) async fn maybe_start_hidden_ui_migration(app: tauri::AppHandle) {
    if !crate::enterprise_policy::is_app_ui_hidden()
        || !crate::enterprise_policy::recording_authorized()
    {
        return;
    }
    let result = async {
        let root = selected_root(&app)?;
        start_storage_migration_inner(
            app.clone(),
            app.state::<RecordingState>(),
            root.display().to_string(),
            true,
        )
        .await
    }
    .await;
    if let Err(error) = result {
        tracing::debug!(%error, "background storage migration deferred");
    }
}

pub(crate) fn is_running(app: &tauri::AppHandle) -> bool {
    app.try_state::<StorageMigrationState>()
        .is_some_and(|state| state.0.lock().unwrap_or_else(|e| e.into_inner()).busy)
}

fn selected_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let settings = SettingsStore::get(app)
        .map_err(|e| e.to_string())?
        .ok_or("Storage settings are unavailable. Reopen settings and try again.")?;
    crate::config::selected_recording_data_dir(&settings.data_dir)
        .map_err(|e| e.to_string())?
        .canonicalize()
        .map_err(|e| e.to_string())
}

fn require_selected_root(app: &tauri::AppHandle, expected: &str) -> Result<PathBuf, String> {
    let root = selected_root(app)?;
    if root != Path::new(expected) {
        return Err(
            "The data directory changed. Review the current storage before continuing.".into(),
        );
    }
    Ok(root)
}

fn progress(app: &tauri::AppHandle, update: MigrationProgress) {
    update_operation(app, |operation| {
        operation.message = update.message.into();
        operation.completed_records = update.completed_records;
        operation.total_records = update.total_records;
        if update.bytes_saved.is_some() {
            operation.bytes_saved = update.bytes_saved;
        }
        if update.available_bytes.is_some() {
            operation.available_bytes = update.available_bytes;
        }
    });
}

pub(crate) struct StartupMigration {
    root: PathBuf,
    _awake: screenpipe_engine::power::KeepAwakeGuard,
}

const RECORDING_PREFERENCE: &str = "pendingStorageMigration";

#[derive(Deserialize, Serialize)]
struct RecordingPreference {
    root: PathBuf,
    recording: bool,
}

fn saved_recording_preference(app: &tauri::AppHandle, root: &Path) -> Result<Option<bool>, String> {
    let store = crate::store::get_store(app, None).map_err(|e| e.to_string())?;
    let Some(value) = store.get(RECORDING_PREFERENCE) else {
        return Ok(None);
    };
    let saved: RecordingPreference = serde_json::from_value(value).map_err(|e| e.to_string())?;
    Ok((root.canonicalize().map_err(|e| e.to_string())? == saved.root).then_some(saved.recording))
}

fn save_recording_preference(
    app: &tauri::AppHandle,
    root: &Path,
    recording: Option<bool>,
) -> Result<(), String> {
    // A separate key in the existing atomic app store survives frontend settings
    // writes. This is application activation state, not batch progress.
    let store = crate::store::get_store(app, None).map_err(|e| e.to_string())?;
    if let Some(recording) = recording {
        store.set(
            RECORDING_PREFERENCE,
            serde_json::json!(RecordingPreference {
                root: root.to_path_buf(),
                recording
            }),
        );
    } else {
        store.delete(RECORDING_PREFERENCE);
    }
    crate::store::save_store_with_permission_repair(app, store.as_ref())?;
    crate::store::reencrypt_store_file(app);
    Ok(())
}

/// Restore recording preference and acknowledge a completed migration at startup.
/// Conversion retries require explicit action. The caller holds the lifecycle lock.
pub(crate) async fn resume_before_startup(
    app: &tauri::AppHandle,
    recording: &RecordingState,
) -> Result<Option<StartupMigration>, String> {
    let settings = SettingsStore::get(app).ok().flatten().unwrap_or_default();
    let root = crate::config::selected_recording_data_dir(&settings.data_dir)
        .map_err(|e| e.to_string())?;
    let pending =
        screenpipe_db::storage::migration_requires_resume(&root).map_err(|e| e.to_string())?;
    let preference = saved_recording_preference(app, &root)?;
    if !pending && preference.is_none() {
        return Ok(None);
    }
    if recording.server.lock().await.is_some() {
        return Err("Stop Screenpipe before resuming migration.".into());
    }
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    if let Some(wants_recording) = preference {
        recording.set_capture_intent(
            wants_recording && crate::recording::recording_access_allowed(app, &settings),
        );
    }
    // An explicit migration already owns progress, the wake lock and final
    // activation verification. Its reopen must not start a second operation.
    if is_running(app) {
        return Ok(None);
    }
    if pending || saved_migration_error(&root).is_some() {
        let error = saved_migration_error(&root).unwrap_or_else(|| INTERRUPTED_MIGRATION.into());
        report_migration_failure(app, &root, &error);
        if let Err(save_error) = save_migration_error(&root, &error) {
            tracing::error!(%save_error, "failed to save migration retry block");
        }
        // Do not retry conversion during startup. The DB currently rejects an
        // incomplete in-place index; its recording recovery remains a blocker.
        return Ok(None);
    }
    if migration_report(&root)
        .map_err(|e| e.to_string())?
        .is_none()
    {
        return Ok(None);
    }
    let awake = screenpipe_engine::power::KeepAwakeGuard::acquire().map_err(|e| e.to_string())?;
    crate::health::set_boot_phase(
        "migrating_database",
        Some("Resuming saved storage migration"),
    );
    update_operation(app, |operation| {
        *operation = Operation {
            root: Some(root.clone()),
            busy: true,
            message: "resuming saved migration".into(),
            started_at: Some(Instant::now()),
            ..Default::default()
        };
    });
    progress(
        app,
        MigrationProgress {
            message: "reopening history and restoring recording",
            completed_records: None,
            total_records: None,
            bytes_saved: None,
            available_bytes: None,
        },
    );
    Ok(Some(StartupMigration {
        root,
        _awake: awake,
    }))
}

async fn verify_running(app: &tauri::AppHandle, root: &Path) -> Result<(), String> {
    let recording = app.state::<RecordingState>();
    let descriptor = StorageDescriptor::read(root)
        .map_err(|e| e.to_string())?
        .ok_or("The migrated storage is not active. Saved progress has been kept.")?;
    {
        let server = recording.server.lock().await;
        let server = server
            .as_ref()
            .ok_or("Screenpipe has not restarted. Try again to finish migration.")?;
        if server.data_dir.canonicalize().map_err(|e| e.to_string())? != root
            || server.db.storage_descriptor() != Some(&descriptor)
        {
            return Err(
                "Screenpipe has not opened the migrated storage. Try again to finish restarting."
                    .into(),
            );
        }
        server
            .db
            .query_raw_sql("SELECT id FROM frames LIMIT 1")
            .await
            .map_err(|e| e.to_string())?;
    }
    if recording.capture_intended() && recording.capture.lock().await.is_none() {
        return Err("Your history was migrated, but recording could not resume. Try again to finish restarting.".into());
    }
    // Keep the preference through storage activation and process restarts. Only
    // the successful application reopen acknowledges it, including a paused user.
    if saved_recording_preference(app, root)?.is_some() {
        save_recording_preference(app, root, None)?;
    }
    Ok(())
}

fn finish_operation(app: &tauri::AppHandle, root: &Path, result: &Result<(), String>) {
    if result.is_ok() {
        track_completed_migration(app, root);
    }
    update_operation(app, |operation| {
        operation.elapsed_seconds = operation.activity().elapsed_seconds;
        operation.started_at = None;
        operation.busy = false;
        operation.completed = result.is_ok();
        operation.error = result.as_ref().err().cloned();
        operation.message = if operation.completed {
            if app.state::<RecordingState>().capture_intended() {
                "Your history has been migrated and recording has resumed."
            } else {
                "Your history has been migrated. Recording remains paused as you selected."
            }
        } else {
            "Migration needs attention. Saved progress will resume when you try again."
        }
        .into();
        operation.completed_records = None;
        operation.total_records = None;
    });
}

pub(crate) async fn finish_startup(
    app: &tauri::AppHandle,
    migration: StartupMigration,
    result: Result<(), String>,
) -> Result<(), String> {
    let result = match result {
        Ok(()) => verify_running(app, &migration.root).await,
        error => error,
    };
    finish_operation(app, &migration.root, &result);
    result
}

fn track_completed_migration(app: &tauri::AppHandle, root: &Path) {
    let Some(analytics) = app.try_state::<std::sync::Arc<crate::analytics::AnalyticsManager>>()
    else {
        return;
    };
    // Use sizes captured in the verified conversion receipt, so resumed recording
    // cannot change the comparison. Exclude the recovery copy and external media.
    let report = match migration_report(root) {
        Ok(Some(report)) => report,
        result => {
            tracing::warn!(?result, "migration telemetry receipt unavailable");
            return;
        }
    };
    let migrated_db_bytes = report
        .allocated_after_bytes
        .unwrap_or_else(|| report.index_bytes.saturating_add(report.payload_bytes));
    let original_db_bytes = report.allocated_before_bytes.unwrap_or(report.source_bytes);
    if migrated_db_bytes == 0 {
        return;
    }
    let event = "storage_migration_completed";
    let properties = serde_json::json!({
        "$insert_id": format!("{event}:{}", report.generation),
        "original_db_bytes": original_db_bytes,
        "migrated_db_bytes": migrated_db_bytes,
        "compression_multiplier": original_db_bytes as f64 / migrated_db_bytes as f64,
    });
    let analytics = std::sync::Arc::clone(&analytics);
    // AnalyticsManager honors the existing telemetry preference. Delivery never
    // holds up the migration UI, recording, or release of the wake lock.
    tauri::async_runtime::spawn(async move {
        if let Err(error) = analytics.send_event(event, Some(properties)).await {
            tracing::warn!(%error, event, "migration telemetry delivery failed");
        }
    });
}

#[tauri::command]
#[specta::specta]
pub async fn get_storage_migration_status(
    app: tauri::AppHandle,
    recording: State<'_, RecordingState>,
) -> Result<StorageMigrationStatus, String> {
    let root = selected_root(&app)?;
    let operation = app
        .state::<StorageMigrationState>()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let descriptor = StorageDescriptor::read(&root).map_err(|e| e.to_string())?;
    let report = migration_report(&root).map_err(|e| e.to_string())?;
    let completed = descriptor
        .as_ref()
        .zip(report.as_ref())
        .is_some_and(|(d, r)| d.database_id == r.database_id && d.generation == r.generation);
    let pending = root.join("storage-migration.json").exists();
    let in_place = if pending {
        screenpipe_db::storage::migration_requires_resume(&root).map_err(|e| e.to_string())?
    } else {
        report
            .as_ref()
            .is_none_or(|r| r.allocated_before_bytes.is_some())
    };
    let source_bytes = std::fs::metadata(root.join("db.sqlite"))
        .map(|m| m.len())
        .unwrap_or(0);
    let mut blocked_reason = None;
    let mut using_new_storage = false;
    let mut can_delete_source = false;
    {
        let server = recording.server.try_lock();
        if server.is_err() && !operation.busy {
            blocked_reason = Some("Screenpipe is restarting. Wait for startup to finish.".into());
        }
        if let Ok(server) = server {
            if let Some(server) = server.as_ref() {
                if server.data_dir.canonicalize().map_err(|e| e.to_string())? != root {
                    blocked_reason = Some(
                        "Apply the data directory change and restart before migrating.".into(),
                    );
                } else {
                    using_new_storage = descriptor.is_some()
                        && server.db.storage_descriptor() == descriptor.as_ref()
                        && !server.db.pool.is_closed();
                    if completed && using_new_storage {
                        match server.db.retained_migration_source_bytes() {
                            Ok(bytes) => can_delete_source = bytes.is_some(),
                            Err(error) => blocked_reason = Some(error.to_string()),
                        }
                    }
                }
            }
        }
    }
    if root.join("vault.meta").exists() || root.join(".vault_locked").exists() {
        blocked_reason =
            Some("Storage migration is unavailable while vault protection is enabled.".into());
    }
    let error = if operation.busy {
        None
    } else {
        operation
            .error
            .filter(|_| operation.root.as_ref() == Some(&root))
            .or_else(|| saved_migration_error(&root))
    };
    let can_migrate = !operation.busy
        && blocked_reason.is_none()
        && (pending
            || (!completed && descriptor.is_none() && source_bytes > 0)
            || (completed && (!using_new_storage || error.is_some())));
    Ok(StorageMigrationStatus {
        root: root.display().to_string(),
        app_session_id: APP_SESSION_ID.clone(),
        busy: operation.busy,
        message: if operation.busy {
            operation.message
        } else {
            String::new()
        },
        error: error.clone(),
        pending,
        in_place,
        completed,
        using_new_storage,
        generation: descriptor.map(|d| d.generation),
        source_bytes,
        migrated_bytes: report.as_ref().map(|r| {
            r.allocated_after_bytes
                .unwrap_or_else(|| r.index_bytes.saturating_add(r.payload_bytes))
        }),
        bytes_saved: report
            .as_ref()
            .and_then(|r| r.allocated_before_bytes.zip(r.allocated_after_bytes))
            .map(|(before, after)| before.saturating_sub(after))
            .or(operation.bytes_saved),
        available_bytes: fs2::available_space(&root).ok(),
        can_migrate,
        can_cancel: !operation.busy
            && !in_place
            && blocked_reason.is_none()
            && pending
            && !root.join("storage.json").exists(),
        can_delete_source: can_delete_source
            && blocked_reason.is_none()
            && !operation.busy
            && !pending
            && error.is_none(),
        blocked_reason,
    })
}

/// Own the stop/convert/restart sequence in the native app even if settings closes.
#[tauri::command]
#[specta::specta]
pub async fn start_storage_migration(
    app: tauri::AppHandle,
    recording: State<'_, RecordingState>,
    root: String,
) -> Result<(), String> {
    start_storage_migration_inner(app, recording, root, false).await
}

async fn start_storage_migration_inner(
    app: tauri::AppHandle,
    recording: State<'_, RecordingState>,
    root: String,
    background: bool,
) -> Result<(), String> {
    let root = require_selected_root(&app, &root)?;
    let lifecycle = recording
        .server_lifecycle
        .clone()
        .try_lock_owned()
        .map_err(|_| {
            "Screenpipe is already restarting or changing storage. Try again when it finishes."
        })?;
    let status = get_storage_migration_status(app.clone(), recording).await?;
    if background {
        if let Some(error) = &status.error {
            report_migration_failure(&app, &root, error);
            return Ok(());
        }
        let recording = app.state::<RecordingState>();
        let server_ready = recording.server.try_lock().is_ok_and(|server| {
            server
                .as_ref()
                .is_some_and(|server| !server.db.pool.is_closed())
        });
        let capture_ready = !recording.capture_intended()
            || recording
                .capture
                .try_lock()
                .is_ok_and(|capture| capture.is_some());
        if !should_start_hidden_ui_migration(
            &status,
            crate::enterprise_policy::is_app_ui_hidden(),
            crate::enterprise_policy::recording_authorized(),
            server_ready && capture_ready,
        ) {
            return Ok(());
        }
    } else if !status.can_migrate {
        return Err(status
            .blocked_reason
            .unwrap_or_else(|| "Migration is unavailable in the current storage state.".into()));
    }
    // Independent of the recording preference, which startup reapplies during switchover.
    // Acquire before pausing so a failed wake lock never strands recording.
    let awake = screenpipe_engine::power::KeepAwakeGuard::acquire().map_err(|error| {
        let error = format!("Could not prevent sleep. Migration has not started: {error}");
        report_migration_failure(&app, &root, &error);
        let _ = save_migration_error(&root, &error);
        error
    })?;
    // Write BEFORE pausing recording. A crash, kill, or failed error write must
    // never erase the block and let the next launch pause recording again.
    save_migration_error(&root, INTERRUPTED_MIGRATION).map_err(|error| {
        report_migration_failure(&app, &root, &error);
        error
    })?;
    let recording_preference = app.state::<RecordingState>().capture_intended();
    if saved_recording_preference(&app, &root)?.is_none() {
        save_recording_preference(&app, &root, Some(recording_preference))?;
    }
    update_operation(&app, |operation| {
        *operation = Operation {
            root: Some(root.clone()),
            busy: true,
            message: if status.can_delete_source {
                "removing the original database"
            } else {
                "pausing recording"
            }
            .into(),
            started_at: Some(Instant::now()),
            background,
            ..Default::default()
        };
    });
    tauri::async_runtime::spawn(async move {
        let _lifecycle = lifecycle;
        let _awake = awake;
        let mut result = async {
            let recording = app.state::<RecordingState>();
            // A completed legacy migration may retain its original source.
            // Verify and clean it up without pausing capture.
            if !background || !status.can_delete_source {
                crate::recording::stop_screenpipe_inner(&recording).await?;
                if !status.completed || status.pending {
                    screenpipe_db::storage::migrate_with_progress(
                        &root,
                        Default::default(),
                        Default::default(),
                        |message| progress(&app, message),
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                }
                require_selected_root(&app, &root.display().to_string())?;
                progress(
                    &app,
                    MigrationProgress {
                        message: "resuming recording on the new storage",
                        completed_records: None,
                        total_records: None,
                        bytes_saved: None,
                        available_bytes: None,
                    },
                );
                crate::recording::spawn_screenpipe_inner(&recording, app.clone()).await?;
            }
            require_selected_root(&app, &root.display().to_string())?;
            verify_running(&app, &root).await?;
            let descriptor = StorageDescriptor::read(&root)
                .map_err(|e| e.to_string())?
                .ok_or("The migrated storage is not active. Saved progress has been kept.")?;
            if background
                && crate::enterprise_policy::is_app_ui_hidden()
                && crate::enterprise_policy::recording_authorized()
            {
                let server = recording.server.lock().await;
                let server = server.as_ref().ok_or(
                    "The new storage is no longer running. The original database has been kept.",
                )?;
                // The DB primitive rechecks the live generation, parity receipt,
                // source identity, WAL/journal absence and a real query under
                // the source lease before deleting only the old db.sqlite.
                if server
                    .db
                    .retained_migration_source_bytes()
                    .map_err(|e| e.to_string())?
                    .is_some()
                {
                    let bytes = server
                        .db
                        .delete_migration_source(&descriptor.generation)
                        .await
                        .map_err(|e| e.to_string())?;
                    tracing::info!(
                        bytes,
                        "background storage migration removed the verified original database"
                    );
                }
            }
            clear_migration_error(&root)?;
            Ok::<_, String>(())
        }
        .await;
        if let Err(error) = &result {
            report_migration_failure(&app, &root, error);
            if let Err(save_error) = save_migration_error(&root, error) {
                // The pre-pause marker still blocks automatic retries.
                tracing::error!(%save_error, "failed to save migration error details");
            }
            if let Err(restart_error) = resume_after_failed_migration(&app, &root).await {
                let error = format!("{error}\nRecording recovery failed: {restart_error}");
                report_migration_failure(&app, &root, &error);
                let _ = save_migration_error(&root, &error);
                result = Err(error);
            }
        }
        finish_operation(&app, &root, &result);
    });
    Ok(())
}

fn report_migration_failure(app: &tauri::AppHandle, root: &Path, error: &str) {
    let state = app.state::<StorageMigrationState>();
    let mut operation = state.0.lock().unwrap_or_else(|e| e.into_inner());
    if operation.root.as_deref() == Some(root) && operation.error.as_deref() == Some(error) {
        return;
    }
    operation.root = Some(root.to_owned());
    operation.error = Some(error.into());
    drop(operation);
    // ERROR events go through the existing Sentry tracing layer when telemetry
    // is enabled. No webview, notification, or network await blocks recovery.
    tracing::error!(%error, "storage migration failed; automatic retries disabled");
}

async fn resume_after_failed_migration(app: &tauri::AppHandle, root: &Path) -> Result<(), String> {
    require_selected_root(app, &root.display().to_string())?;
    let recording = app.state::<RecordingState>();
    let server_ready = recording
        .server
        .lock()
        .await
        .as_ref()
        .is_some_and(|server| !server.db.pool.is_closed());
    let capture_ready = !recording.capture_intended() || recording.capture.lock().await.is_some();
    if server_ready && capture_ready {
        return Ok(());
    }
    // Preserve capture intent, including an explicit pause. Teardown also
    // clears the restart cooldown, whose deferred path requires a webview.
    // Startup must not restart conversion. Reopen usable storage; an incomplete
    // in-place index still needs a safe DB recovery path before this can succeed.
    crate::recording::stop_screenpipe_inner(&recording).await?;
    crate::recording::spawn_screenpipe_inner(&recording, app.clone()).await?;
    {
        let server = recording.server.lock().await;
        let server = server.as_ref().ok_or("Recording server did not restart.")?;
        server
            .db
            .query_raw_sql("SELECT id FROM frames LIMIT 1")
            .await
            .map_err(|e| e.to_string())?;
    }
    if recording.capture_intended() && recording.capture.lock().await.is_none() {
        return Err("Recording did not resume.".into());
    }
    tracing::info!("recording restored after migration failure; migration remains blocked");
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_storage_migration(
    app: tauri::AppHandle,
    recording: State<'_, RecordingState>,
    root: String,
) -> Result<(), String> {
    let root = require_selected_root(&app, &root)?;
    let _lifecycle = recording
        .server_lifecycle
        .try_lock()
        .map_err(|_| "Migration is still running.")?;
    if is_running(&app) {
        return Err("Migration is still running.".into());
    }
    let status = get_storage_migration_status(app.clone(), app.state::<RecordingState>()).await?;
    if !status.can_cancel {
        return Err("Only an unfinished migration can be cancelled.".into());
    }
    crate::recording::stop_screenpipe_inner(&recording).await?;
    screenpipe_db::storage::cancel_migration(&root, Default::default())
        .await
        .map_err(|e| e.to_string())?;
    crate::recording::spawn_screenpipe_inner(&recording, app.clone()).await?;
    *app.state::<StorageMigrationState>()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Operation::default();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_original_storage_database(
    app: tauri::AppHandle,
    recording: State<'_, RecordingState>,
    root: String,
    generation: String,
    confirm_permanent_deletion: bool,
) -> Result<u64, String> {
    if !confirm_permanent_deletion {
        return Err("Confirm permanent deletion of the original database first.".into());
    }
    let root = require_selected_root(&app, &root)?;
    let _lifecycle = recording
        .server_lifecycle
        .try_lock()
        .map_err(|_| "Storage is still switching. The original database has been kept.")?;
    let status = get_storage_migration_status(app.clone(), app.state::<RecordingState>()).await?;
    if !status.can_delete_source {
        return Err("Complete migration and switch to the new storage before deleting the original database.".into());
    }
    let server = recording.server.lock().await;
    let server = server
        .as_ref()
        .ok_or("Start Screenpipe on the new storage before deleting the original database.")?;
    if server.data_dir.canonicalize().map_err(|e| e.to_string())? != root {
        return Err("The running data directory differs from the selected directory.".into());
    }
    server
        .db
        .delete_migration_source(&generation)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_status() -> StorageMigrationStatus {
        StorageMigrationStatus {
            root: "/fixture".into(),
            app_session_id: "test-process".into(),
            busy: false,
            message: String::new(),
            error: None,
            pending: false,
            in_place: true,
            completed: false,
            using_new_storage: false,
            generation: None,
            source_bytes: 1024,
            migrated_bytes: None,
            bytes_saved: None,
            available_bytes: None,
            can_migrate: true,
            can_cancel: false,
            can_delete_source: false,
            blocked_reason: None,
        }
    }

    #[test]
    fn hidden_ui_migration_requires_live_authorization_and_ready_recorder() {
        let status = legacy_status();
        assert!(should_start_hidden_ui_migration(&status, true, true, true));
        for (hidden, authorized, ready) in [
            (false, true, true),
            (true, false, true),
            (true, true, false),
        ] {
            assert!(!should_start_hidden_ui_migration(
                &status, hidden, authorized, ready
            ));
        }
    }

    #[test]
    fn hidden_ui_migration_waits_for_safe_storage_and_does_not_loop_on_failure() {
        for status in [
            StorageMigrationStatus {
                busy: true,
                ..legacy_status()
            },
            StorageMigrationStatus {
                error: Some("conversion failed".into()),
                ..legacy_status()
            },
            StorageMigrationStatus {
                blocked_reason: Some("vault is protected".into()),
                ..legacy_status()
            },
            StorageMigrationStatus {
                can_migrate: false,
                ..legacy_status()
            },
        ] {
            assert!(!should_start_hidden_ui_migration(&status, true, true, true));
        }
        // Even a journal from an older app without a saved error must not
        // automatically pause recording again. Explicit retry remains available.
        let resumed = StorageMigrationStatus {
            pending: true,
            ..legacy_status()
        };
        assert!(!should_start_hidden_ui_migration(
            &resumed, true, true, true
        ));
        assert!(resumed.can_migrate);
    }

    #[test]
    fn migration_failure_and_interruption_block_automatic_retries_across_restarts() {
        let root = tempfile::tempdir().unwrap();
        let other_root = tempfile::tempdir().unwrap();
        for error in [INTERRUPTED_MIGRATION, "not enough free disk space"] {
            save_migration_error(root.path(), error).unwrap();
            // A new process has no Operation.error. Read only durable state.
            let fresh_process = Operation::default();
            assert!(fresh_process.error.is_none());
            let status = StorageMigrationStatus {
                error: saved_migration_error(root.path()),
                ..legacy_status()
            };
            assert_eq!(status.error.as_deref(), Some(error));
            assert!(!should_start_hidden_ui_migration(&status, true, true, true));
            assert!(status.can_migrate, "explicit retry remains possible");
            assert!(saved_migration_error(other_root.path()).is_none());
        }
        // Only successful completion clears the durable block.
        clear_migration_error(root.path()).unwrap();
        assert!(saved_migration_error(root.path()).is_none());
    }

    #[test]
    fn older_interrupted_attempts_and_unreadable_markers_also_block_retries() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("storage-migration.json"), b"{}").unwrap();
        assert_eq!(
            saved_migration_error(root.path()).as_deref(),
            Some(INTERRUPTED_MIGRATION)
        );
        std::fs::remove_file(root.path().join("storage-migration.json")).unwrap();
        std::fs::create_dir(root.path().join(MIGRATION_ERROR_FILE)).unwrap();
        assert!(saved_migration_error(root.path()).is_some());
    }

    #[tokio::test]
    async fn preflight_failure_preserves_original_writes_without_retrying_on_later_launches() {
        use screenpipe_db::{
            storage::{migrate, pause_interrupted_migration, MigrationOptions},
            DatabaseManager,
        };

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("db.sqlite");
        let db = DatabaseManager::new(path.to_str().unwrap(), Default::default())
            .await
            .unwrap();
        db.execute_raw_sql_write("INSERT INTO frames(id,timestamp,full_text) VALUES(1,'2026-09-14T12:00:00Z','before failed migration')").await.unwrap();
        db.close().await;
        save_migration_error(root.path(), INTERRUPTED_MIGRATION).unwrap();
        let mut options = MigrationOptions::default();
        options.budget.disk_reserve_bytes = u64::MAX / 2;
        let error = migrate(root.path(), Default::default(), options)
            .await
            .unwrap_err();
        save_migration_error(root.path(), &error.to_string()).unwrap();

        for id in 2..=3 {
            // Same storage recovery used by ServerCore::start. Each simulated
            // launch restores recording's DB but never invokes migrate again.
            pause_interrupted_migration(root.path()).unwrap();
            let db = DatabaseManager::new(path.to_str().unwrap(), Default::default())
                .await
                .unwrap();
            assert!(db.storage_descriptor().is_none());
            db.execute_raw_sql_write(&format!("INSERT INTO frames(id,timestamp,full_text) VALUES({id},'2026-09-14T12:01:00Z','recording after failed migration')")).await.unwrap();
            let rows = db
                .query_raw_sql("SELECT id FROM frames ORDER BY id")
                .await
                .unwrap();
            assert_eq!(rows.as_array().unwrap().len(), id as usize);
            let status = StorageMigrationStatus {
                error: saved_migration_error(root.path()),
                pending: root.path().join("storage-migration.json").exists(),
                ..legacy_status()
            };
            assert!(!should_start_hidden_ui_migration(&status, true, true, true));
            db.close().await;
        }
        assert!(path.is_file());
    }

    #[test]
    fn hidden_ui_cleanup_recovers_after_restart_and_stops_after_source_removal() {
        let mut status = StorageMigrationStatus {
            completed: true,
            using_new_storage: true,
            can_migrate: false,
            can_delete_source: true,
            ..legacy_status()
        };
        assert!(should_start_hidden_ui_migration(&status, true, true, true));
        status.can_delete_source = false;
        status.source_bytes = 0;
        assert!(!should_start_hidden_ui_migration(&status, true, true, true));
    }

    #[test]
    fn background_completion_does_not_offer_the_interactive_recovery_copy_prompt() {
        let mut operation = Operation {
            background: true,
            busy: true,
            ..Default::default()
        };
        // If the admin reveals the UI mid-conversion, writes remain blocked.
        assert!(operation.activity().busy);
        operation.busy = false;
        operation.completed = true;
        assert!(!operation.activity().completed);
        operation.background = false;
        assert!(operation.activity().completed);
    }
}
