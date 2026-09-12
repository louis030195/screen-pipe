// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use screenpipe_db::{
    storage::{MigrationOptions, PrivacyPolicy, Projection},
    DatabaseManager,
};
use screenpipe_redact::{
    worker::{Worker, WorkerConfig},
    Pipeline, TextRedactionPolicy,
};
use std::sync::Arc;

#[tokio::test]
async fn archived_history_and_malformed_json_use_generation_completion() {
    let root = tempfile::tempdir().unwrap();
    let db = Arc::new(
        DatabaseManager::new_hybrid(root.path(), Default::default(), MigrationOptions::default())
            .await
            .unwrap(),
    );
    let mut tx = db.begin_immediate_with_retry().await.unwrap();
    sqlx::query("INSERT INTO frames(id,timestamp,full_text,accessibility_text,accessibility_tree_json,text_json,window_name) VALUES(1,'2026-09-11','alice@example.com','alice@example.com','{\"text\":\"alice@example.com\"}','[{\"text\":\"alice@example.com\",\"left\":\"0.25\"}]','alice@example.com'),(2,'2026-09-11','bob@example.com',NULL,'{broken',NULL,NULL)").execute(&mut **tx.conn()).await.unwrap();
    tx.commit().await.unwrap();
    db.seal_frame_payloads().await.unwrap();
    let policy = TextRedactionPolicy::from_labels(&["email".to_owned()]);
    let worker = Worker::new_with_writer(
        db.pool.clone(),
        db.coordinated_writer(),
        Arc::new(Pipeline::regex_only_with_policy(policy)),
        WorkerConfig::default(),
    )
    .with_frame_storage(Arc::clone(&db));
    assert_eq!(worker.process_hybrid_frames(16).await.unwrap(), 1);
    let payloads = db.frame_payloads(&[1, 2], Projection::All).await.unwrap();
    let first = serde_json::to_string(&payloads[&1]).unwrap();
    assert!(!first.contains("alice@example.com"));
    assert!(first.contains("0.25"));
    assert_eq!(payloads[&2].full_text.as_deref(), Some("bob@example.com"));
    let blocked: (i64, Option<i64>) =
        sqlx::query_as("SELECT completed_surfaces,retry_at FROM frame_payloads WHERE frame_id=2")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(blocked.0, 0);
    assert!(blocked.1.is_some());
    assert_eq!(db.seal_frame_payloads().await.unwrap(), 1);
    db.set_frame_privacy_policy(&PrivacyPolicy::default())
        .await
        .unwrap();
    db.close().await;
}

struct UnavailableDetector;
#[async_trait::async_trait]
impl screenpipe_redact::Redactor for UnavailableDetector {
    fn name(&self) -> &str {
        "unavailable-test-detector"
    }
    fn version(&self) -> u32 {
        1
    }
    async fn redact_batch(
        &self,
        _: &[String],
    ) -> Result<Vec<screenpipe_redact::RedactionOutput>, screenpipe_redact::RedactError> {
        Err(screenpipe_redact::RedactError::Unavailable(
            "test outage".into(),
        ))
    }
}

#[tokio::test]
async fn detector_fallback_does_not_complete_archive_processing() {
    use screenpipe_redact::Redactor;
    let root = tempfile::tempdir().unwrap();
    let db = Arc::new(
        DatabaseManager::new_hybrid(root.path(), Default::default(), Default::default())
            .await
            .unwrap(),
    );
    db.execute_raw_sql_write("INSERT INTO frames(id,timestamp,full_text) VALUES(1,'2026-09-11','long input requiring the configured detector')").await.unwrap();
    let pipeline = Arc::new(Pipeline::regex_then_ai(
        Arc::new(UnavailableDetector),
        Default::default(),
    ));
    assert!(pipeline
        .redact("long input requiring the configured detector")
        .await
        .is_ok());
    let worker = Worker::new_with_writer(
        db.pool.clone(),
        db.coordinated_writer(),
        pipeline,
        WorkerConfig::default(),
    )
    .with_frame_storage(Arc::clone(&db));
    assert_eq!(worker.process_hybrid_frames(1).await.unwrap(), 0);
    assert_eq!(db.seal_frame_payloads().await.unwrap(), 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT attempts FROM frame_payloads WHERE frame_id=1")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        1
    );
    db.close().await;
}

#[tokio::test]
async fn archived_elements_audio_and_ui_use_the_existing_privacy_worker() {
    let root = tempfile::tempdir().unwrap();
    let db = Arc::new(
        DatabaseManager::new_hybrid(root.path(), Default::default(), Default::default())
            .await
            .unwrap(),
    );
    db.execute_raw_sql_write("INSERT INTO frames(id,timestamp,full_text) VALUES(1,'2026-09-11T12:00:00Z','capture'); INSERT INTO elements(id,frame_id,source,role,text,properties) VALUES(1,1,'accessibility','AXTextField','alice@example.com','{\"value\":\"alice@example.com\"}'); INSERT INTO audio_chunks(id,file_path) VALUES(1,'test.wav'); INSERT INTO audio_transcriptions(id,audio_chunk_id,offset_index,timestamp,transcription) VALUES(1,1,0,'2026-09-11T12:00:00Z','alice@example.com'); INSERT INTO ui_events(id,timestamp,event_type,text_content,element_value,window_title) VALUES(1,'2026-09-11T12:00:00Z','text','alice@example.com','alice@example.com','alice@example.com');").await.unwrap();
    while db.seal_payloads().await.unwrap() != 0 {}
    let old_files: Vec<String> = sqlx::query_scalar("SELECT path FROM _bulk_files")
        .fetch_all(&db.pool)
        .await
        .unwrap();
    let shutdown = Arc::new(tokio::sync::Notify::new());
    let worker = Worker::new_with_writer(
        db.pool.clone(),
        db.coordinated_writer(),
        Arc::new(Pipeline::regex_only_with_policy(
            TextRedactionPolicy::from_labels(&["email".into()]),
        )),
        WorkerConfig {
            resource_governor: None,
            poll_interval: std::time::Duration::from_millis(10),
            idle_between_batches: std::time::Duration::from_millis(1),
            max_cpu_cooldown: std::time::Duration::from_millis(10),
            ..Default::default()
        },
    )
    .with_frame_storage(Arc::clone(&db))
    .spawn_with_shutdown(Arc::clone(&shutdown));
    let mut completed = false;
    for _ in 0..200 {
        let count:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM elements WHERE redacted_at IS NOT NULL)+(SELECT count(*) FROM audio_transcriptions WHERE redacted_at IS NOT NULL)+(SELECT count(*) FROM ui_events WHERE redacted_at IS NOT NULL)").fetch_one(&db.pool).await.unwrap();
        if count == 3 {
            completed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    shutdown.notify_one();
    worker.await.unwrap();
    assert!(completed);
    let output=db.query_raw_sql("SELECT e.text,e.properties,a.transcription,u.text_content,u.element_value,u.window_title FROM elements e CROSS JOIN audio_transcriptions a CROSS JOIN ui_events u").await.unwrap();
    assert!(!output.to_string().contains("alice@example.com"));
    while db.seal_payloads().await.unwrap() != 0 {}
    for _ in 0..16 {
        db.reclaim_frame_payloads().await.unwrap();
    }
    assert!(old_files.iter().all(|p| !root.path().join(p).exists()));
    db.verify_storage().await.unwrap();
    db.close().await;
}
