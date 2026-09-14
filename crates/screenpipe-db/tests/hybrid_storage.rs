// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use screenpipe_db::storage::{migrate, MigrationOptions, PrivacyPolicy, Projection, StorageMode};
use screenpipe_db::{ContentType, DatabaseManager};

async fn seed(db: &DatabaseManager, id: i64, text: Option<&str>, accessibility: Option<&str>) {
    let mut tx = db.begin_immediate_with_retry().await.unwrap();
    sqlx::query("INSERT INTO frames(id,timestamp,full_text,accessibility_text,text_json,accessibility_tree_json,app_name,window_name,device_name) VALUES(?,'2026-09-11T12:00:00Z',?,?,'[{\"text\":\"hello\",\"left\":0.5}]','{\"text\":\"hello\"}','Editor','notes','display')")
        .bind(id).bind(text).bind(accessibility).execute(&mut **tx.conn()).await.unwrap();
    tx.commit().await.unwrap();
}

async fn search(db: &DatabaseManager, query: &str) -> serde_json::Value {
    let results = db
        .search(
            query,
            ContentType::OCR,
            100,
            0,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    serde_json::to_value(results).unwrap()
}

#[tokio::test]
async fn staged_sealed_reopened_search_and_full_payloads_match() {
    let root = tempfile::tempdir().unwrap();
    let db = DatabaseManager::new_hybrid(root.path(), Default::default(), Default::default())
        .await
        .unwrap();
    seed(&db, 1, Some("hello café 東京"), Some("accessibility")).await;
    seed(&db, 2, None, Some("fallback")).await;
    seed(&db, 3, Some(""), Some("must remain empty")).await;
    let before = db
        .frame_payloads(&[1, 2, 3], Projection::All)
        .await
        .unwrap();
    let results = search(&db, "hello").await;
    assert_eq!(db.seal_frame_payloads().await.unwrap(), 3);
    assert_eq!(
        before,
        db.frame_payloads(&[1, 2, 3], Projection::All)
            .await
            .unwrap()
    );
    assert_eq!(results, search(&db, "hello").await);
    let stored:i64=sqlx::query_scalar("SELECT count(*) FROM frames WHERE full_text IS NOT NULL OR text_json IS NOT NULL OR accessibility_tree_json IS NOT NULL OR accessibility_text IS NOT NULL").fetch_one(&db.pool).await.unwrap();
    assert_eq!(stored, 0);
    let durability: i64 = sqlx::query_scalar("PRAGMA synchronous")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(durability, 2);
    db.close().await;
    let reopened = DatabaseManager::new(
        root.path().join("db.sqlite").to_str().unwrap(),
        Default::default(),
    )
    .await
    .unwrap();
    assert_eq!(reopened.storage_mode(), StorageMode::HybridParquetV1);
    assert_eq!(
        before,
        reopened
            .frame_payloads(&[1, 2, 3], Projection::All)
            .await
            .unwrap()
    );
    assert_eq!(results, search(&reopened, "hello").await);
    reopened.close().await;
    let relative = std::process::Command::new(env!("CARGO_BIN_EXE_screenpipe-storage"))
        .current_dir(root.path())
        .arg("verify")
        .arg("")
        .output()
        .unwrap();
    assert!(
        relative.status.success(),
        "{}",
        String::from_utf8_lossy(&relative.stderr)
    );
}

#[tokio::test]
async fn privacy_completion_and_reader_leases_own_original_file_removal() {
    let root = tempfile::tempdir().unwrap();
    let options = MigrationOptions {
        privacy: PrivacyPolicy {
            identity: "policy-1".into(),
            required_surfaces: 15,
        },
        ..Default::default()
    };
    let db = DatabaseManager::new_hybrid(root.path(), Default::default(), options)
        .await
        .unwrap();
    seed(&db, 1, Some("secret"), Some("secret")).await;
    seed(&db, 2, Some("keep"), Some("keep")).await;
    assert_eq!(db.seal_frame_payloads().await.unwrap(), 0);
    for p in db
        .frame_payloads(&[1, 2], Projection::All)
        .await
        .unwrap()
        .into_values()
    {
        assert!(db
            .replace_frame_payload(&p, "policy-1", 15, None, None)
            .await
            .unwrap());
    }
    assert_eq!(db.seal_frame_payloads().await.unwrap(), 2);
    let original: String =
        sqlx::query_scalar("SELECT search_path FROM payload_files WHERE state='published'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let token = db.storage_read_token().await.unwrap();
    let mut payload = db
        .frame_payloads(&[1], Projection::All)
        .await
        .unwrap()
        .remove(&1)
        .unwrap();
    payload.full_text = Some("[REDACTED]".into());
    payload.accessibility_text = Some("[REDACTED]".into());
    assert!(db
        .replace_frame_payload(&payload, "policy-1", 15, None, None)
        .await
        .unwrap());
    assert!(!db
        .replace_frame_payload(&payload, "policy-1", 15, None, None)
        .await
        .unwrap());
    assert!(token.admit(&db.pool).await.is_err());
    db.reclaim_frame_payloads().await.unwrap();
    assert!(root.path().join(&original).exists());
    drop(token);
    db.reclaim_frame_payloads().await.unwrap();
    assert!(!root.path().join(&original).exists());
    assert_eq!(
        db.frame_payloads(&[2], Projection::Search).await.unwrap()[&2].text(),
        "keep"
    );
    assert!(search(&db, "secret").await.as_array().unwrap().is_empty());
    db.close().await;
}

#[tokio::test]
async fn migration_preserves_original_until_explicit_deletion_on_live_generation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("db.sqlite");
    let db = DatabaseManager::new(path.to_str().unwrap(), Default::default())
        .await
        .unwrap();
    seed(&db, 7, Some("migration café"), None).await;
    let identity = db.upload_source_id().await.unwrap().to_owned();
    db.execute_raw_sql_write("CREATE TABLE test_mutable_state(id INTEGER PRIMARY KEY,state BLOB); INSERT INTO test_mutable_state VALUES(1,x'000102ff');").await.unwrap();
    let before = db.frame_payloads(&[7], Projection::All).await.unwrap();
    let mut projected = Vec::new();
    for projection in [Projection::Search, Projection::Detail] {
        let payloads = db.frame_payloads(&[7], projection).await.unwrap();
        match projection {
            Projection::Search => assert!(payloads[&7].text_json.is_none()),
            Projection::Detail => assert!(payloads[&7].full_text.is_none()),
            Projection::All => unreachable!(),
        }
        projected.push((projection, payloads));
    }
    db.close().await;
    let report = migrate(root.path(), Default::default(), Default::default())
        .await
        .unwrap();
    assert_eq!(report.frames, 1);
    assert!(path.exists());
    assert!(root.path().join("storage.json").exists());
    assert!(!root.path().join("storage-migration.json").exists());
    let db = DatabaseManager::new(path.to_str().unwrap(), Default::default())
        .await
        .unwrap();
    let after = db.frame_payloads(&[7], Projection::All).await.unwrap();
    assert_eq!(before[&7].full_text, after[&7].full_text);
    assert_eq!(db.upload_source_id().await.unwrap(), identity);
    for (projection, mut expected) in projected {
        let mut actual = db.frame_payloads(&[7], projection).await.unwrap();
        for payload in expected.values_mut().chain(actual.values_mut()) {
            payload.generation = 0;
        }
        assert_eq!(expected, actual);
    }
    let blob: Vec<u8> = sqlx::query_scalar("SELECT state FROM test_mutable_state")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(blob, [0, 1, 2, 255]);
    seed(&db, 8, Some("after activation"), None).await;
    assert_eq!(
        db.frame_payloads(&[8], Projection::Search).await.unwrap()[&8].text(),
        "after activation"
    );
    assert_eq!(
        db.retained_migration_source_bytes().unwrap(),
        Some(report.source_bytes)
    );
    assert!(db
        .delete_migration_source("stale-generation")
        .await
        .is_err());
    assert!(path.exists());
    let receipt_path = root.path().join("storage-migration-complete.json");
    let receipt = std::fs::read(&receipt_path).unwrap();
    std::fs::remove_file(&receipt_path).unwrap();
    assert!(db
        .delete_migration_source(&report.generation)
        .await
        .is_err());
    assert!(path.exists());
    std::fs::write(&receipt_path, receipt).unwrap();
    let pending_path = root.path().join("storage-migration.json");
    std::fs::write(&pending_path, b"{}").unwrap();
    assert!(db
        .delete_migration_source(&report.generation)
        .await
        .is_err());
    std::fs::remove_file(pending_path).unwrap();
    let original_owner =
        screenpipe_sqlite_coordinator::acquire_sqlite_manager_lease(&path).unwrap();
    assert!(db
        .delete_migration_source(&report.generation)
        .await
        .is_err());
    assert!(path.exists());
    original_owner.release();
    // A leftover WAL index contains no database records. Completion/status must
    // tolerate it while real WAL/journal data still prevents source deletion.
    let shm_path = root.path().join("db.sqlite-shm");
    std::fs::write(&shm_path, [0u8; 32768]).unwrap();
    assert_eq!(
        db.retained_migration_source_bytes().unwrap(),
        Some(report.source_bytes)
    );
    assert!(path.exists());
    let wal_path = root.path().join("db.sqlite-wal");
    std::fs::write(&wal_path, b"unmigrated writes").unwrap();
    assert!(db
        .delete_migration_source(&report.generation)
        .await
        .is_err());
    std::fs::remove_file(wal_path).unwrap();
    let journal_path = root.path().join("db.sqlite-journal");
    std::fs::write(&journal_path, b"unmigrated writes").unwrap();
    assert!(db
        .delete_migration_source(&report.generation)
        .await
        .is_err());
    assert!(path.exists());
    std::fs::remove_file(journal_path).unwrap();
    let original_meta = std::fs::metadata(&path).unwrap();
    let original = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    original.set_len(original_meta.len() + 1).unwrap();
    assert!(db
        .delete_migration_source(&report.generation)
        .await
        .is_err());
    assert!(path.exists());
    original.set_len(original_meta.len()).unwrap();
    original
        .set_times(std::fs::FileTimes::new().set_modified(original_meta.modified().unwrap()))
        .unwrap();
    drop(original);
    assert_eq!(
        db.delete_migration_source(&report.generation)
            .await
            .unwrap(),
        report.source_bytes
    );
    assert!(!path.exists());
    assert!(db
        .delete_migration_source(&report.generation)
        .await
        .is_err());
    assert_eq!(
        db.frame_payloads(&[8], Projection::Search).await.unwrap()[&8].text(),
        "after activation"
    );
    db.close().await;
}

#[tokio::test]
async fn corruption_fails_reads_without_returning_partial_payloads() {
    let root = tempfile::tempdir().unwrap();
    let db = DatabaseManager::new_hybrid(root.path(), Default::default(), Default::default())
        .await
        .unwrap();
    seed(&db, 1, Some("hello"), None).await;
    db.seal_frame_payloads().await.unwrap();
    let path: String =
        sqlx::query_scalar("SELECT detail_path FROM payload_files WHERE state='published'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    std::fs::write(root.path().join(path), b"corrupt").unwrap();
    assert!(db
        .frame_payloads(&[1], Projection::All)
        .await
        .unwrap_err()
        .to_string()
        .contains("checksum"));
    assert_eq!(
        db.frame_payloads(&[1], Projection::Search).await.unwrap()[&1].text(),
        "hello"
    );
    db.close().await;
}

#[tokio::test]
async fn backup_restores_staging_files_and_source_identity() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("source");
    let db = DatabaseManager::new_hybrid(&root, Default::default(), Default::default())
        .await
        .unwrap();
    seed(&db, 1, Some("archived"), None).await;
    db.seal_frame_payloads().await.unwrap();
    seed(&db, 2, Some("staged"), Some("other")).await;
    let identity = db.upload_source_id().await.unwrap().to_owned();
    let before = db.frame_payloads(&[1, 2], Projection::All).await.unwrap();
    let backup = parent.path().join("backup");
    db.backup_to(backup.to_str().unwrap()).await.unwrap();
    let restored = parent.path().join("restored");
    screenpipe_db::storage::restore(&backup, &restored, Default::default())
        .await
        .unwrap();
    let copy = DatabaseManager::new(
        restored.join("db.sqlite").to_str().unwrap(),
        Default::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        before,
        copy.frame_payloads(&[1, 2], Projection::All).await.unwrap()
    );
    assert_eq!(identity, copy.upload_source_id().await.unwrap());
    copy.close().await;
    db.close().await;
}

#[tokio::test]
async fn lean_retention_replaces_archived_details_and_preserves_search() {
    let root = tempfile::tempdir().unwrap();
    let db = DatabaseManager::new_hybrid(root.path(), Default::default(), Default::default())
        .await
        .unwrap();
    seed(&db, 1, Some("retained text"), Some("retained a11y")).await;
    db.seal_frame_payloads().await.unwrap();
    db.strip_heavy_text_in_range(
        "2026-09-11T00:00:00Z".parse().unwrap(),
        "2026-09-12T00:00:00Z".parse().unwrap(),
    )
    .await
    .unwrap();
    let payload = db
        .frame_payloads(&[1], Projection::All)
        .await
        .unwrap()
        .remove(&1)
        .unwrap();
    assert_eq!(payload.text(), "retained text");
    assert!(payload.text_json.is_none());
    assert!(payload.accessibility_tree_json.is_none());
    assert!(!search(&db, "retained").await.as_array().unwrap().is_empty());
    db.close().await;
}

#[tokio::test]
async fn resident_sql_resolves_wildcards_aliases_and_views() {
    let root = tempfile::tempdir().unwrap();
    let db = DatabaseManager::new_hybrid(root.path(), Default::default(), Default::default())
        .await
        .unwrap();
    seed(&db, 1, Some("private payload"), None).await;
    db.execute_raw_sql_write("CREATE VIEW payload_view AS SELECT full_text AS words FROM frames")
        .await
        .unwrap();
    for query in [
        "SELECT f.* FROM frames f LIMIT 1",
        "SELECT words FROM payload_view LIMIT 1",
        "SELECT full_text AS t FROM frames LIMIT 1",
    ] {
        assert!(
            db.query_raw_sql(query)
                .await
                .unwrap_err()
                .to_string()
                .contains("unsupported-storage-query"),
            "{query}"
        );
    }
    assert!(db
        .query_raw_sql("SELECT 'full_text' AS label, id FROM frames LIMIT 1")
        .await
        .is_ok());
    db.seal_frame_payloads().await.unwrap();
    assert_eq!(search(&db, "private").await.as_array().unwrap().len(), 1);
    db.close().await;
}

#[tokio::test]
async fn compact_and_legacy_export_preserve_logical_records() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("source");
    let db = DatabaseManager::new_hybrid(&root, Default::default(), Default::default())
        .await
        .unwrap();
    seed(&db, 1, Some("old text"), None).await;
    db.seal_frame_payloads().await.unwrap();
    let mut payload = db
        .frame_payloads(&[1], Projection::All)
        .await
        .unwrap()
        .remove(&1)
        .unwrap();
    payload.full_text = Some("replacement café".into());
    db.replace_frame_payload(&payload, "", 0, None, None)
        .await
        .unwrap();
    db.seal_frame_payloads().await.unwrap();
    db.reclaim_frame_payloads().await.unwrap();
    let before = search(&db, "replacement").await;
    let old = screenpipe_db::storage::StorageDescriptor::read(&root)
        .unwrap()
        .unwrap();
    db.close().await;
    screenpipe_db::storage::compact(&root, Default::default())
        .await
        .unwrap();
    let new = screenpipe_db::storage::StorageDescriptor::read(&root)
        .unwrap()
        .unwrap();
    assert_ne!(old.generation, new.generation);
    assert_eq!(old.payloads, new.payloads);
    assert!(!root.join(old.index).exists());
    let export = parent.path().join("legacy.sqlite");
    screenpipe_db::storage::export_sqlite(&root, &export, Default::default())
        .await
        .unwrap();
    let db = DatabaseManager::new(export.to_str().unwrap(), Default::default())
        .await
        .unwrap();
    assert_eq!(db.storage_mode(), StorageMode::Sqlite);
    assert_eq!(before, search(&db, "replacement").await);
    assert!(search(&db, "old").await.as_array().unwrap().is_empty());
    seed(&db, 2, Some("new legacy write"), None).await;
    assert_eq!(search(&db, "legacy").await.as_array().unwrap().len(), 1);
    db.close().await;
}

#[tokio::test]
async fn legacy_exports_can_be_migrated_again() {
    for populated in [false, true] {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("source");
        let db = DatabaseManager::new_hybrid(&root, Default::default(), Default::default())
            .await
            .unwrap();
        if populated {
            seed(&db, 1, Some("export café 東京"), Some("accessible text")).await;
            db.seal_frame_payloads().await.unwrap();
            seed(&db, 2, None, Some("")).await;
        }
        let expected_payloads = db.frame_payloads(&[1, 2], Projection::All).await.unwrap();
        let expected_search = search(&db, "export").await;
        db.close().await;

        let exported_root = parent.path().join("exported");
        std::fs::create_dir(&exported_root).unwrap();
        let exported_path = exported_root.join("db.sqlite");
        screenpipe_db::storage::export_sqlite(&root, &exported_path, Default::default())
            .await
            .unwrap();
        let original = std::fs::read(&exported_path).unwrap();
        migrate(&exported_root, Default::default(), Default::default())
            .await
            .unwrap();
        assert_eq!(std::fs::read(&exported_path).unwrap(), original);

        let remigrated = DatabaseManager::new(exported_path.to_str().unwrap(), Default::default())
            .await
            .unwrap();
        assert_eq!(remigrated.storage_mode(), StorageMode::HybridParquetV1);
        assert_eq!(
            remigrated
                .frame_payloads(&[1, 2], Projection::All)
                .await
                .unwrap(),
            expected_payloads
        );
        assert_eq!(search(&remigrated, "export").await, expected_search);
        remigrated.verify_storage().await.unwrap();
        remigrated.close().await;
    }
}

#[tokio::test]
async fn storage_budget_rejects_before_acknowledging_and_accepts_after_reclamation() {
    let root = tempfile::tempdir().unwrap();
    let mut options = MigrationOptions::default();
    options.budget.staging_bytes = options.budget.record_bytes as u64;
    let limit = options.budget.record_bytes;
    let db = DatabaseManager::new_hybrid(root.path(), Default::default(), options)
        .await
        .unwrap();
    let large = "x".repeat(limit + 1);
    let mut tx = db.begin_immediate_with_retry().await.unwrap();
    let error = sqlx::query("INSERT INTO frames(id,timestamp,full_text) VALUES(1,'2026-09-11',?)")
        .bind(&large)
        .execute(&mut **tx.conn())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("record budget"));
    tx.rollback().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM frames")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        0
    );
    seed(&db, 2, Some("complete payload"), None).await;
    db.seal_frame_payloads().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT staging_bytes FROM storage_metadata")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        0
    );
    assert!(db
        .admit_consumer_sync()
        .unwrap_err()
        .to_string()
        .contains("binding unavailable"));
    db.close().await;
}

#[cfg(feature = "storage-fault-injection")]
#[tokio::test]
async fn migration_crashes_resume_without_losing_acknowledged_records() {
    for point in [
        "migration_schema_ready",
        "migration_schema_step",
        "migration_elements_copied",
        "seal_reserved",
        "seal_files_synced",
        "seal_before_commit",
        "seal_committed",
        "migration_batch_sealed",
        "migration_ready",
        "migration_activated",
        "migration_completed",
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("db.sqlite");
        let db = DatabaseManager::new(path.to_str().unwrap(), Default::default())
            .await
            .unwrap();
        seed(
            &db,
            7,
            Some("survives crash café"),
            Some("distinct accessibility"),
        )
        .await;
        let identity = db.upload_source_id().await.unwrap().to_owned();
        let before = search(&db, "survives").await;
        db.close().await;
        let killed = std::process::Command::new(env!("CARGO_BIN_EXE_screenpipe-storage"))
            .arg("migrate")
            .arg(root.path())
            .env("SCREENPIPE_STORAGE_CRASH_AT", point)
            .output()
            .unwrap();
        assert_eq!(
            killed.status.code(),
            Some(86),
            "{point}: {}",
            String::from_utf8_lossy(&killed.stderr)
        );
        assert!(
            path.exists(),
            "original must survive interruption at {point}"
        );
        migrate(root.path(), Default::default(), Default::default())
            .await
            .unwrap();
        let db = DatabaseManager::new(path.to_str().unwrap(), Default::default())
            .await
            .unwrap();
        assert!(
            path.exists(),
            "original must survive completion after {point}"
        );
        assert!(
            db.retained_migration_source_bytes()
                .unwrap_or_else(|error| panic!("{point}: {error}"))
                .is_some(),
            "{point}"
        );
        assert_eq!(db.upload_source_id().await.unwrap(), identity, "{point}");
        assert_eq!(before, search(&db, "survives").await, "{point}");
        db.reclaim_frame_payloads().await.unwrap();
        db.verify_storage().await.unwrap();
        db.close().await;
    }
}

#[cfg(feature = "storage-fault-injection")]
#[tokio::test]
async fn interrupted_initialization_resumes_its_recorded_generation() {
    for point in [
        "migration_schema_step",
        "initialization_ready",
        "initialization_activated",
    ] {
        let root = tempfile::tempdir().unwrap();
        let killed = std::process::Command::new(env!("CARGO_BIN_EXE_screenpipe-storage"))
            .arg("init")
            .arg(root.path())
            .env("SCREENPIPE_STORAGE_CRASH_AT", point)
            .output()
            .unwrap();
        assert_eq!(
            killed.status.code(),
            Some(86),
            "{}",
            String::from_utf8_lossy(&killed.stderr)
        );
        let expected: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.path().join("storage-init.json")).unwrap())
                .unwrap();
        let db = DatabaseManager::new_hybrid(root.path(), Default::default(), Default::default())
            .await
            .unwrap();
        let active = screenpipe_db::storage::StorageDescriptor::read(root.path())
            .unwrap()
            .unwrap();
        assert_eq!(active.generation, expected["generation"].as_str().unwrap());
        seed(&db, 1, Some("ready after interruption"), None).await;
        db.close().await;
    }
}

#[cfg(feature = "storage-fault-injection")]
#[tokio::test]
async fn interrupted_desktop_migration_allows_recording_after_restart_and_safe_resume() {
    use screenpipe_db::storage::pause_interrupted_migration;

    for point in [
        "migration_schema_ready",
        "migration_ready",
        "migration_activated",
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("db.sqlite");
        let db = DatabaseManager::new(path.to_str().unwrap(), Default::default())
            .await
            .unwrap();
        seed(&db, 7, Some("before restart"), None).await;
        db.close().await;
        let killed = std::process::Command::new(env!("CARGO_BIN_EXE_screenpipe-storage"))
            .arg("migrate")
            .arg(root.path())
            .env("SCREENPIPE_STORAGE_CRASH_AT", point)
            .output()
            .unwrap();
        assert_eq!(killed.status.code(), Some(86), "{point}");

        if point != "migration_activated" {
            assert!(
                DatabaseManager::new(path.to_str().unwrap(), Default::default())
                    .await
                    .is_err()
            );
            let lock = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(root.path().join(".storage.lock"))
                .unwrap();
            fs2::FileExt::try_lock_exclusive(&lock).unwrap();
            assert!(pause_interrupted_migration(root.path()).is_err());
            drop(lock);
        }
        pause_interrupted_migration(root.path()).unwrap();
        pause_interrupted_migration(root.path()).unwrap();
        let db = DatabaseManager::new(path.to_str().unwrap(), Default::default())
            .await
            .unwrap();
        assert_eq!(
            db.storage_descriptor().is_some(),
            point == "migration_activated"
        );
        seed(&db, 8, Some("after restart"), None).await;
        let before = search(&db, "restart").await;
        db.close().await;

        migrate(root.path(), Default::default(), Default::default())
            .await
            .unwrap();
        let db = DatabaseManager::new(path.to_str().unwrap(), Default::default())
            .await
            .unwrap();
        assert_eq!(search(&db, "restart").await, before, "{point}");
        assert!(db.storage_descriptor().is_some());
        assert!(path.is_file(), "original retained at {point}");
        db.close().await;
    }
}

#[cfg(feature = "storage-fault-injection")]
#[tokio::test]
async fn cancellation_preserves_the_source_before_activation() {
    for point in [
        "migration_schema_ready",
        "migration_ready",
        "migration_activated",
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("db.sqlite");
        let db = DatabaseManager::new(path.to_str().unwrap(), Default::default())
            .await
            .unwrap();
        seed(&db, 1, Some("survives cancellation"), None).await;
        let before = search(&db, "survives").await;
        db.close().await;
        let killed = std::process::Command::new(env!("CARGO_BIN_EXE_screenpipe-storage"))
            .arg("migrate")
            .arg(root.path())
            .env("SCREENPIPE_STORAGE_CRASH_AT", point)
            .output()
            .unwrap();
        assert_eq!(killed.status.code(), Some(86));
        let cancelled =
            screenpipe_db::storage::cancel_migration(root.path(), Default::default()).await;
        if point == "migration_activated" {
            assert!(cancelled.is_err());
            migrate(root.path(), Default::default(), Default::default())
                .await
                .unwrap();
        } else {
            cancelled.unwrap();
            assert!(path.is_file());
            assert!(!root.path().join("storage-migration.json").exists());
            assert!(!root.path().join("storage.json").exists());
        }
        let db = DatabaseManager::new(path.to_str().unwrap(), Default::default())
            .await
            .unwrap();
        assert_eq!(search(&db, "survives").await, before);
        db.close().await;
    }
}

#[cfg(unix)]
#[tokio::test]
async fn descriptor_and_catalog_paths_stay_inside_their_generation() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let db = DatabaseManager::new_hybrid(root.path(), Default::default(), Default::default())
        .await
        .unwrap();
    seed(&db, 1, Some("path ownership"), None).await;
    db.seal_frame_payloads().await.unwrap();
    let original: String =
        sqlx::query_scalar("SELECT search_path FROM payload_files WHERE state='published'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let original_path = root.path().join(&original);
    std::fs::rename(&original_path, outside.path().join("search.parquet")).unwrap();
    std::os::unix::fs::symlink(outside.path().join("search.parquet"), &original_path).unwrap();
    assert!(db.frame_payloads(&[1], Projection::Search).await.is_err());
    db.close().await;
    assert!(DatabaseManager::new(
        root.path().join("db.sqlite").to_str().unwrap(),
        Default::default()
    )
    .await
    .is_err());
    let mut descriptor = screenpipe_db::storage::StorageDescriptor::read(root.path())
        .unwrap()
        .unwrap();
    descriptor.index = "elsewhere/index.sqlite".into();
    std::fs::write(
        root.path().join("storage.json"),
        serde_json::to_vec(&descriptor).unwrap(),
    )
    .unwrap();
    assert!(DatabaseManager::new(
        root.path().join("db.sqlite").to_str().unwrap(),
        Default::default()
    )
    .await
    .is_err());
}

#[tokio::test]
async fn migration_streams_large_payloads_with_bounded_temporary_space() {
    use std::sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    };
    fn bytes(path: &std::path::Path) -> u64 {
        std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| {
                if entry.path().is_dir() {
                    bytes(&entry.path())
                } else {
                    entry.metadata().map(|m| m.len()).unwrap_or(0)
                }
            })
            .sum()
    }
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("db.sqlite");
    let source = DatabaseManager::new(path.to_str().unwrap(), Default::default())
        .await
        .unwrap();
    let payload = format!("{{\"value\":\"{}\"}}", "x".repeat(1024 * 1024));
    let mut tx = source.begin_immediate_with_retry().await.unwrap();
    for id in 1..=64 {
        sqlx::query("INSERT INTO frames(id,timestamp,full_text,accessibility_tree_json) VALUES(?,'2026-09-11T12:00:00Z','streamed history',?)")
            .bind(id).bind(&payload).execute(&mut **tx.conn()).await.unwrap();
    }
    tx.commit().await.unwrap();
    source.execute_raw_sql_write("CREATE TABLE migration_audit(id INTEGER PRIMARY KEY,n INTEGER); INSERT INTO migration_audit VALUES(1,1); CREATE TRIGGER audit_frame AFTER INSERT ON frames BEGIN UPDATE migration_audit SET n=n+1; END;").await.unwrap();
    // Import visits the child first; its parent is in a later byte-bounded batch.
    let mut tx = source.begin_immediate_with_retry().await.unwrap();
    sqlx::query("INSERT INTO elements(id,frame_id,source,role,properties) VALUES(2,1,'accessibility','AXGroup',?)")
        .bind(&payload).execute(&mut **tx.conn()).await.unwrap();
    sqlx::query("INSERT INTO elements(id,frame_id,source,role,properties,parent_id) VALUES(1,1,'accessibility','AXText',?,2)")
        .bind(&payload).execute(&mut **tx.conn()).await.unwrap();
    tx.commit().await.unwrap();
    source.close().await;
    let original = std::fs::metadata(&path).unwrap();
    let mut options = MigrationOptions::default();
    options.budget.file_bytes = 2 * 1024 * 1024;
    // This permits at most 48 MiB of additional allocation before the reserve;
    // even one complete source copy would exceed it.
    let headroom = 48 * 1024 * 1024;
    options.budget.disk_reserve_bytes = fs2::available_space(root.path()).unwrap() - headroom;
    assert!(original.len() > headroom);
    let done = Arc::new(AtomicBool::new(false));
    let peak = Arc::new(AtomicU64::new(0));
    let monitor = {
        let (done, peak, directory) = (done.clone(), peak.clone(), root.path().join("storage"));
        std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                peak.fetch_max(bytes(&directory), Ordering::Relaxed);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        })
    };
    let updates = std::sync::Mutex::new(Vec::new());
    let result = screenpipe_db::storage::migrate_with_progress(
        root.path(),
        Default::default(),
        options,
        |update| {
            updates.lock().unwrap().push(update);
        },
    )
    .await;
    done.store(true, Ordering::Relaxed);
    monitor.join().unwrap();
    let report = result.unwrap();
    let updates = updates.into_inner().unwrap();
    let counts: Vec<_> = updates
        .iter()
        .filter_map(|update| update.completed_records.zip(update.total_records))
        .collect();
    assert!(
        counts.len() > 3,
        "byte-bounded batches must report intermediate work"
    );
    assert_eq!(counts.first().unwrap().0, 0);
    assert!(
        counts.last().unwrap().1 >= 66,
        "frames and elements contribute to the total"
    );
    assert!(counts
        .windows(2)
        .all(|pair| pair[0].0 <= pair[1].0 && pair[0].1 == pair[1].1));
    let (converted, total) = *counts.last().unwrap();
    assert_eq!(converted, total);
    assert!(
        updates.last().unwrap().total_records.is_none(),
        "conversion percentage must not pretend to measure verification or activation"
    );
    let peak = peak.load(Ordering::Relaxed);
    eprintln!("streaming migration: source={} peak_candidate={} final_index={} parquet={} headroom={headroom}", original.len(), peak, report.index_bytes, report.payload_bytes);
    assert!(
        peak < headroom,
        "peak {peak} exceeds batch headroom {headroom}"
    );
    assert!(peak < original.len() / 2);
    assert!(report.index_bytes + report.payload_bytes < original.len() / 4);
    let retained = std::fs::metadata(&path).unwrap();
    assert_eq!(original.len(), retained.len());
    assert_eq!(original.modified().unwrap(), retained.modified().unwrap());
    let db = DatabaseManager::new(path.to_str().unwrap(), Default::default())
        .await
        .unwrap();
    assert_eq!(
        db.frame_payloads(&[1, 64], Projection::All).await.unwrap()[&64]
            .accessibility_tree_json
            .as_deref(),
        Some(payload.as_str())
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT n FROM migration_audit")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("PRAGMA auto_vacuum")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("PRAGMA freelist_count")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        0
    );
    seed(&db, 65, Some("after migration"), None).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT n FROM migration_audit")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        2
    );
    db.verify_storage().await.unwrap();
    db.close().await;
}
