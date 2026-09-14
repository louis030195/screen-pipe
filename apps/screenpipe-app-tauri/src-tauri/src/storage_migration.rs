// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use crate::{recording::RecordingState, store::SettingsStore};
use screenpipe_db::storage::{migration_report, MigrationProgress, StorageDescriptor};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::Instant,
};
use tauri::{Emitter, Manager, State};

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
    completed: bool,
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
            completed: self.completed,
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
    pub busy: bool,
    pub message: String,
    pub error: Option<String>,
    pub pending: bool,
    pub completed: bool,
    pub using_new_storage: bool,
    pub generation: Option<String>,
    pub source_bytes: u64,
    pub migrated_bytes: Option<u64>,
    pub can_migrate: bool,
    pub can_cancel: bool,
    pub can_delete_source: bool,
    pub blocked_reason: Option<String>,
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
    });
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
    let migrated_db_bytes = report.index_bytes.saturating_add(report.payload_bytes);
    if migrated_db_bytes == 0 {
        return;
    }
    let event = "storage_migration_completed";
    let properties = serde_json::json!({
        "$insert_id": format!("{event}:{}", report.generation),
        "original_db_bytes": report.source_bytes,
        "migrated_db_bytes": migrated_db_bytes,
        "compression_multiplier": report.source_bytes as f64 / migrated_db_bytes as f64,
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
    let error = if operation.root.as_ref() == Some(&root) {
        operation.error
    } else {
        None
    };
    let can_migrate = !operation.busy
        && blocked_reason.is_none()
        && (pending
            || (!completed && descriptor.is_none() && source_bytes > 0)
            || (completed && (!using_new_storage || error.is_some())));
    Ok(StorageMigrationStatus {
        root: root.display().to_string(),
        busy: operation.busy,
        message: if operation.busy {
            operation.message
        } else {
            String::new()
        },
        error: error.clone(),
        pending,
        completed,
        using_new_storage,
        generation: descriptor.map(|d| d.generation),
        source_bytes,
        migrated_bytes: report.map(|r| r.index_bytes.saturating_add(r.payload_bytes)),
        can_migrate,
        can_cancel: !operation.busy
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
    let root = require_selected_root(&app, &root)?;
    let lifecycle = recording
        .server_lifecycle
        .clone()
        .try_lock_owned()
        .map_err(|_| {
            "Screenpipe is already restarting or changing storage. Try again when it finishes."
        })?;
    let status = get_storage_migration_status(app.clone(), recording).await?;
    if !status.can_migrate {
        return Err(status
            .blocked_reason
            .unwrap_or_else(|| "Migration is unavailable in the current storage state.".into()));
    }
    // Independent of the recording preference, which startup reapplies during switchover.
    // Acquire before pausing so a failed wake lock never strands recording.
    let awake = screenpipe_engine::power::KeepAwakeGuard::acquire()
        .map_err(|error| format!("Could not prevent sleep. Migration has not started: {error}"))?;
    update_operation(&app, |operation| {
        *operation = Operation {
            root: Some(root.clone()),
            busy: true,
            message: "pausing recording".into(),
            started_at: Some(Instant::now()),
            ..Default::default()
        };
    });
    tauri::async_runtime::spawn(async move {
        let _lifecycle = lifecycle;
        let _awake = awake;
        let result = async {
            let recording = app.state::<RecordingState>();
            crate::recording::stop_screenpipe_inner(&recording).await?;
            if !status.completed || status.pending {
                screenpipe_db::storage::migrate_with_progress(&root, Default::default(), Default::default(), |message| progress(&app, message))
                    .await.map_err(|e| e.to_string())?;
            }
            require_selected_root(&app, &root.display().to_string())?;
            progress(&app, MigrationProgress {
                message: "resuming recording on the new storage",
                completed_records: None,
                total_records: None,
            });
            crate::recording::spawn_screenpipe_inner(&recording, app.clone()).await?;
            let descriptor = StorageDescriptor::read(&root).map_err(|e| e.to_string())?
                .ok_or("The new storage is not active. The original database has been kept.")?;
            {
            let server = recording.server.lock().await;
            let server = server.as_ref().ok_or("Screenpipe has not restarted. The original database has been kept.")?;
            if server.data_dir.canonicalize().map_err(|e| e.to_string())? != root
                || server.db.storage_descriptor() != Some(&descriptor) {
                return Err("Screenpipe has not switched to the new storage. The original database has been kept.".into());
            }
            server.db.query_raw_sql("SELECT id FROM frames LIMIT 1").await.map_err(|e| e.to_string())?;
            }
            if recording.capture_intended() && recording.capture.lock().await.is_none() {
                return Err("Your history was migrated, but recording could not resume. Try again to finish restarting. The original database has been kept.".into());
            }
            Ok::<_, String>(())
        }.await;
        if result.is_ok() {
            track_completed_migration(&app, &root);
        }
        update_operation(&app, |operation| {
            operation.elapsed_seconds = operation.activity().elapsed_seconds;
            operation.started_at = None;
            operation.busy = false;
            operation.completed = result.is_ok();
            operation.error = result.err();
            operation.message = if operation.completed {
                if app.state::<RecordingState>().capture_intended() {
                    "Your history has been migrated and recording has resumed."
                } else {
                    "Your history has been migrated. Recording remains paused as you selected."
                }
            } else {
                "Migration needs attention. Your original database has been kept."
            }
            .into();
            operation.completed_records = None;
            operation.total_records = None;
        });
    });
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
