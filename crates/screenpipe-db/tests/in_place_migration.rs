// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use screenpipe_db::{
    storage::{migrate, MigrationOptions, Projection},
    DatabaseManager,
};

async fn fixture(root: &std::path::Path) {
    let db = DatabaseManager::new(root.join("db.sqlite").to_str().unwrap(), Default::default())
        .await
        .unwrap();
    let mut tx = db.begin_immediate_with_retry().await.unwrap();
    let detail = "capture detail ".repeat(32768);
    for id in 1..=48 {
        sqlx::query("INSERT INTO frames(id,timestamp,full_text,accessibility_tree_json) VALUES(?,'2026-09-11T12:00:00Z','searchable migration history',?)").bind(id).bind(&detail).execute(&mut **tx.conn()).await.unwrap();
        sqlx::query("INSERT INTO elements(id,frame_id,source,role,text,properties) VALUES(?,?,'accessibility','AXText','searchable element',?)").bind(id).bind(id).bind(&detail).execute(&mut **tx.conn()).await.unwrap();
    }
    tx.commit().await.unwrap();
    db.close().await;
}

#[tokio::test]
#[ignore = "requires a marked disposable volume no larger than 512 MiB"]
async fn migration_completes_with_less_free_space_than_its_final_payloads() {
    use std::io::Write;
    let volume = std::path::PathBuf::from(std::env::var("SCREENPIPE_CONSTRAINED_VOLUME").unwrap());
    assert!(volume.join(".screenpipe-disposable-volume").is_file());
    assert!(fs2::total_space(&volume).unwrap() <= 512 * 1024 * 1024);
    let root = tempfile::tempdir_in(&volume).unwrap();
    let db = DatabaseManager::new(
        root.path().join("db.sqlite").to_str().unwrap(),
        Default::default(),
    )
    .await
    .unwrap();
    let mut state = 0x852df832c973ba01_u64;
    let alphabet = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789+/";
    for id in 1..=96 {
        let detail: String = (0..256 * 1024)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                alphabet[(state & 63) as usize] as char
            })
            .collect();
        let mut tx = db.begin_immediate_with_retry().await.unwrap();
        sqlx::query("INSERT INTO frames(id,timestamp,full_text,accessibility_tree_json) VALUES(?,'2026-09-14','constrained disk history',?)").bind(id).bind(&detail).execute(&mut **tx.conn()).await.unwrap();
        sqlx::query("INSERT INTO elements(id,frame_id,source,role,properties) VALUES(?,?,'accessibility','AXText',?)").bind(id).bind(id).bind(&detail).execute(&mut **tx.conn()).await.unwrap();
        tx.commit().await.unwrap();
    }
    db.close().await;
    let mut filler = std::fs::File::create(root.path().join("unrelated-data")).unwrap();
    let block = vec![0xa5; 1024 * 1024];
    while fs2::available_space(&volume).unwrap() > 13 * 1024 * 1024 {
        filler.write_all(&block).unwrap();
        filler.sync_all().unwrap();
    }
    drop(filler);
    let initial_free = fs2::available_space(&volume).unwrap();
    let mut options = MigrationOptions::default();
    options.budget.file_bytes = 512 * 1024;
    options.budget.record_bytes = 1024 * 1024;
    options.budget.disk_reserve_bytes = 0;
    let updates = std::sync::Mutex::new(Vec::new());
    let result = screenpipe_db::storage::migrate_with_progress(
        root.path(),
        Default::default(),
        options,
        |p| {
            eprintln!(
                "phase={} completed={:?} saved={:?} available={:?}",
                p.message, p.completed_records, p.bytes_saved, p.available_bytes
            );
            updates.lock().unwrap().push(p);
        },
    )
    .await;
    let report = match result {
        Ok(report) => report,
        Err(error) => {
            let kept = root.keep();
            panic!(
                "{error}; retained fixture={} free={}",
                kept.display(),
                fs2::available_space(&volume).unwrap()
            );
        }
    };
    assert!(
        report.payload_bytes > initial_free,
        "fixture must require progressive reclamation"
    );
    assert!(updates.lock().unwrap().iter().any(
        |u| u.completed_records.unwrap_or(0) < 192 && u.bytes_saved.unwrap_or(0) > 1024 * 1024
    ));
    eprintln!("constrained migration: initial_free={initial_free}, final_payloads={}, allocated_before={}, allocated_after={}, remaining_free={}", report.payload_bytes,report.allocated_before_bytes.unwrap(),report.allocated_after_bytes.unwrap(),fs2::available_space(&volume).unwrap());
    let db = DatabaseManager::new(
        root.path().join("db.sqlite").to_str().unwrap(),
        Default::default(),
    )
    .await
    .unwrap();
    db.verify_storage().await.unwrap();
    assert_eq!(
        db.frame_payloads(&[1, 96], Projection::All)
            .await
            .unwrap()
            .len(),
        2
    );
    db.close().await;
}

#[tokio::test]
async fn external_reader_prevents_conversion_before_rename() {
    use sqlx::Connection;
    let root = tempfile::tempdir().unwrap();
    fixture(root.path()).await;
    let source = root.path().join("db.sqlite");
    let mut reader = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&source)
            .read_only(true),
    )
    .await
    .unwrap();
    let mut tx = reader.begin().await.unwrap();
    sqlx::query("SELECT count(*) FROM frames")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert!(migrate(root.path(), Default::default(), Default::default())
        .await
        .is_err());
    assert!(source.is_file());
    assert!(!root.path().join("storage-migration.json").exists());
    tx.rollback().await.unwrap();
    reader.close().await.unwrap();
    migrate(root.path(), Default::default(), Default::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn insufficient_headroom_keeps_the_complete_source() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path()).await;
    let mut options = MigrationOptions::default();
    options.budget.disk_reserve_bytes = fs2::available_space(root.path()).unwrap();
    assert!(migrate(root.path(), Default::default(), options)
        .await
        .is_err());
    assert!(root.path().join("db.sqlite").is_file());
    assert!(!root.path().join("storage-migration.json").exists());
}

#[cfg(feature = "storage-fault-injection")]
#[tokio::test]
async fn legacy_candidate_is_retired_only_with_a_verified_source() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path()).await;
    let crashed = std::process::Command::new(env!("CARGO_BIN_EXE_screenpipe-storage"))
        .arg("migrate")
        .arg(root.path())
        .env("SCREENPIPE_STORAGE_CRASH_AT", "migration_before_rename")
        .output()
        .unwrap();
    assert_eq!(crashed.status.code(), Some(86));
    let journal_path = root.path().join("storage-migration.json");
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&journal_path).unwrap()).unwrap();
    legacy["format"] = 1.into();
    let mut unknown = legacy.clone();
    unknown["source"] = serde_json::json!([]);
    unknown["snapshot"] = serde_json::Value::Null;
    std::fs::write(&journal_path, serde_json::to_vec(&unknown).unwrap()).unwrap();
    assert!(migrate(root.path(), Default::default(), Default::default())
        .await
        .is_err());
    assert!(root.path().join("db.sqlite").is_file());
    assert!(journal_path.is_file());

    std::fs::write(&journal_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let report = migrate(root.path(), Default::default(), Default::default())
        .await
        .unwrap();
    assert_eq!(report.frames, 48);
    assert!(report.allocated_before_bytes.is_some());
    assert!(!root.path().join("db.sqlite").exists());
    assert!(!journal_path.exists());
}

#[cfg(feature = "storage-fault-injection")]
#[tokio::test]
async fn schema_staging_and_reclamation_interruptions_retain_completed_files() {
    for (point, hit) in [
        ("migration_schema_step", 2),
        ("migration_schema_step", 8),
        ("migration_schema_step", 16),
        ("migration_schema_step", 32),
        ("migration_batch_staged", 2),
        ("seal_committed", 2),
        ("bulk_committed", 2),
        ("migration_blocks_reclaimed", 2),
    ] {
        let root = tempfile::tempdir().unwrap();
        fixture(root.path()).await;
        let crashed = std::process::Command::new(env!("CARGO_BIN_EXE_screenpipe-storage"))
            .arg("migrate")
            .arg(root.path())
            .env("SCREENPIPE_STORAGE_CRASH_AT", point)
            .env("SCREENPIPE_STORAGE_CRASH_HIT", hit.to_string())
            .output()
            .unwrap();
        assert_eq!(
            crashed.status.code(),
            Some(86),
            "{point}/{hit}: {}",
            String::from_utf8_lossy(&crashed.stderr)
        );
        let before: Vec<_> = screenpipe_db::storage::inventory(root.path())
            .unwrap()
            .into_iter()
            .filter(|p| p.extension().is_some_and(|s| s == "parquet"))
            .map(|p| {
                let bytes = std::fs::read(&p).unwrap();
                (p, bytes)
            })
            .collect();
        assert!(screenpipe_db::storage::migration_requires_resume(root.path()).unwrap());
        let report = migrate(root.path(), Default::default(), Default::default())
            .await
            .unwrap();
        assert_eq!(report.frames, 48);
        for (path, bytes) in before {
            assert_eq!(
                std::fs::read(path).unwrap(),
                bytes,
                "completed Parquet must not be rebuilt at {point}/{hit}"
            );
        }
        let db = DatabaseManager::new(
            root.path().join("db.sqlite").to_str().unwrap(),
            Default::default(),
        )
        .await
        .unwrap();
        db.verify_storage().await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM elements")
                .fetch_one(&db.pool)
                .await
                .unwrap(),
            48
        );
        db.close().await;
    }
}

#[tokio::test]
async fn privacy_pending_payloads_remain_resident_and_searchable() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path()).await;
    let source = DatabaseManager::new(
        root.path().join("db.sqlite").to_str().unwrap(),
        Default::default(),
    )
    .await
    .unwrap();
    source.execute_raw_sql_write("INSERT INTO audio_chunks(id,file_path) VALUES(1,'audio.wav'); INSERT INTO audio_transcriptions(id,audio_chunk_id,offset_index,timestamp,transcription,device) VALUES(1,1,0,'2026-09-14','private spoken history','microphone'); INSERT INTO ui_events(id,timestamp,event_type,text_content) VALUES(1,'2026-09-14','text','private typed history'); INSERT INTO pipe_executions(id,pipe_name,status,stdout) VALUES(1,'running-task','running','mutable progress');").await.unwrap();
    source.close().await;
    let options = MigrationOptions {
        privacy: screenpipe_db::storage::PrivacyPolicy {
            identity: "required-redaction".into(),
            required_surfaces: 15,
        },
        ..Default::default()
    };
    migrate(root.path(), Default::default(), options)
        .await
        .unwrap();
    let db = DatabaseManager::new(
        root.path().join("db.sqlite").to_str().unwrap(),
        Default::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM frame_payloads WHERE state='staged'")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        48
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM _bulk_element_rows")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        48
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM elements_fts WHERE elements_fts MATCH 'searchable'"
        )
        .fetch_one(&db.pool)
        .await
        .unwrap(),
        48
    );
    assert_eq!(db.seal_frame_payloads().await.unwrap(), 0);
    for table in ["audio_transcriptions", "ui_events"] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
                "SELECT count(*) FROM {table}_fts WHERE {table}_fts MATCH 'private'"
            )))
            .fetch_one(&db.pool)
            .await
            .unwrap(),
            1
        );
    }
    db.execute_raw_sql_write("UPDATE pipe_executions SET stdout='still running' WHERE id=1")
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT stdout FROM pipe_executions WHERE id=1")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        "still running"
    );
    db.verify_storage().await.unwrap();
    db.close().await;
}

#[tokio::test]
#[ignore = "requires a marked disposable filesystem without hole punching"]
async fn unsupported_volume_fails_before_conversion() {
    let volume = std::path::PathBuf::from(std::env::var("SCREENPIPE_UNSUPPORTED_VOLUME").unwrap());
    assert!(volume.join(".screenpipe-disposable-volume").is_file());
    assert!(fs2::total_space(&volume).unwrap() <= 512 * 1024 * 1024);
    let root = tempfile::tempdir_in(volume).unwrap();
    fixture(root.path()).await;
    let mut options = MigrationOptions::default();
    options.budget.disk_reserve_bytes = 0;
    options.budget.file_bytes = 1024 * 1024;
    options.budget.record_bytes = 1024 * 1024;
    let error = migrate(root.path(), Default::default(), options)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("filesystem reclamation unavailable"),
        "{error}"
    );
    assert!(root.path().join("db.sqlite").is_file());
    assert!(!root.path().join("storage-migration.json").exists());
    let db = DatabaseManager::new(
        root.path().join("db.sqlite").to_str().unwrap(),
        Default::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM frames")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        48
    );
    db.close().await;
}

#[cfg(unix)]
#[tokio::test]
async fn failed_payload_write_resumes_without_discarding_published_files() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    fixture(root.path()).await;
    let changed = std::sync::Mutex::new(Vec::new());
    let result = screenpipe_db::storage::migrate_with_progress(
        root.path(),
        Default::default(),
        Default::default(),
        |p| {
            let mut changed = changed.lock().unwrap();
            if p.completed_records.unwrap_or(0) == 0 || !changed.is_empty() {
                return;
            }
            let journal: serde_json::Value = serde_json::from_slice(
                &std::fs::read(root.path().join("storage-migration.json")).unwrap(),
            )
            .unwrap();
            let directory = root
                .path()
                .join(journal["descriptor"]["payloads"].as_str().unwrap());
            let permissions = std::fs::metadata(&directory).unwrap().permissions();
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o500)).unwrap();
            changed.push((directory, permissions));
        },
    )
    .await;
    for (path, permissions) in changed.into_inner().unwrap() {
        std::fs::set_permissions(path, permissions).unwrap();
    }
    assert!(result.is_err());
    assert!(screenpipe_db::storage::migration_requires_resume(root.path()).unwrap());
    let files: Vec<_> = screenpipe_db::storage::inventory(root.path())
        .unwrap()
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "parquet"))
        .map(|p| {
            let data = std::fs::read(&p).unwrap();
            (p, data)
        })
        .collect();
    assert!(!files.is_empty());
    migrate(root.path(), Default::default(), Default::default())
        .await
        .unwrap();
    for (p, data) in files {
        assert_eq!(std::fs::read(p).unwrap(), data);
    }
}

#[tokio::test]
#[ignore = "fills a marked disposable volume to exercise saved disk-full progress"]
async fn exhausted_volume_resumes_after_space_is_restored() {
    use std::io::Write;
    let volume = std::path::PathBuf::from(std::env::var("SCREENPIPE_CONSTRAINED_VOLUME").unwrap());
    assert!(volume.join(".screenpipe-disposable-volume").is_file());
    assert!(fs2::total_space(&volume).unwrap() <= 512 * 1024 * 1024);
    let root = tempfile::tempdir_in(&volume).unwrap();
    fixture(root.path()).await;
    let filler = root.path().join("unrelated-data");
    let filled = std::sync::atomic::AtomicBool::new(false);
    let mut options = MigrationOptions::default();
    options.budget.file_bytes = 1024 * 1024;
    options.budget.record_bytes = 1024 * 1024;
    options.budget.disk_reserve_bytes = 0;
    let result = screenpipe_db::storage::migrate_with_progress(
        root.path(),
        Default::default(),
        options,
        |p| {
            if p.completed_records.unwrap_or(0) == 0
                || filled.swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                return;
            }
            let mut file = std::fs::File::create(&filler).unwrap();
            let block = vec![0x5a; 1024 * 1024];
            while fs2::available_space(&volume).unwrap() > 2 * 1024 * 1024 {
                if file
                    .write_all(&block)
                    .and_then(|_| file.sync_all())
                    .is_err()
                {
                    break;
                }
            }
        },
    )
    .await;
    assert!(result.is_err());
    assert!(screenpipe_db::storage::migration_requires_resume(root.path()).unwrap());
    assert!(DatabaseManager::new(
        root.path().join("db.sqlite").to_str().unwrap(),
        Default::default()
    )
    .await
    .is_err());
    let files: Vec<_> = screenpipe_db::storage::inventory(root.path())
        .unwrap()
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "parquet"))
        .map(|p| {
            let data = std::fs::read(&p).unwrap();
            (p, data)
        })
        .collect();
    assert!(!files.is_empty());
    std::fs::remove_file(filler).unwrap();
    migrate(root.path(), Default::default(), Default::default())
        .await
        .unwrap();
    for (p, data) in files {
        assert_eq!(std::fs::read(p).unwrap(), data);
    }
}

#[tokio::test]
async fn reuses_index_and_preserves_history_and_compact_backup() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path()).await;
    let source = std::fs::metadata(root.path().join("db.sqlite")).unwrap();
    let mut options = MigrationOptions::default();
    options.budget.file_bytes = 1024 * 1024;
    let report = migrate(root.path(), Default::default(), options)
        .await
        .unwrap();
    assert!(!root.path().join("db.sqlite").exists());
    let descriptor = screenpipe_db::storage::StorageDescriptor::read(root.path())
        .unwrap()
        .unwrap();
    let index = std::fs::metadata(root.path().join(descriptor.index)).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            source.ino(),
            index.ino(),
            "migration must reuse the physical index"
        );
        assert!(index.blocks() < source.blocks());
    }
    assert!(report.allocated_after_bytes.unwrap() < report.allocated_before_bytes.unwrap());
    let db = DatabaseManager::new(
        root.path().join("db.sqlite").to_str().unwrap(),
        Default::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        db.frame_payloads(&[1, 48], Projection::All)
            .await
            .unwrap()
            .len(),
        2
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM elements WHERE text='searchable element'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(count, 48);
    let backup_root = tempfile::tempdir().unwrap();
    let backup = backup_root.path().join("backup");
    db.backup_to(backup.to_str().unwrap()).await.unwrap();
    let backup_descriptor = screenpipe_db::storage::StorageDescriptor::read(&backup)
        .unwrap()
        .unwrap();
    assert!(
        std::fs::metadata(backup.join(backup_descriptor.index))
            .unwrap()
            .len()
            < source.len() / 2
    );
    db.close().await;
}
