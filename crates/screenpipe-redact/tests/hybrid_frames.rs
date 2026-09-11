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
