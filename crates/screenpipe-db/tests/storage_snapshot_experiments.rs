// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

#![cfg(feature = "storage-bench-experiments")]

use screenpipe_db::{
    storage::{experiments, Projection},
    DatabaseManager,
};
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn snapshot_candidate_preserves_rows_across_writes_and_rejects_revocation() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join(".screenpipe-api-benchmark"), "").unwrap();
    experiments::configure(
        root.path(),
        "snapshot,snapshot-admission,frame-cache,element-cache,selective-decode",
    )
    .unwrap();
    let db = Arc::new(
        DatabaseManager::new_hybrid(
            root.path(),
            Default::default(),
            screenpipe_db::storage::MigrationOptions {
                budget: screenpipe_db::storage::StorageBudget {
                    row_group_rows: 2,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await
        .unwrap(),
    );
    for sql in [
        "CREATE TABLE _benchmark_revocation(id INTEGER PRIMARY KEY, revision INTEGER NOT NULL)",
        "INSERT INTO _benchmark_revocation VALUES(1,0)",
        "CREATE TRIGGER _benchmark_revoke_delete AFTER DELETE ON frames BEGIN UPDATE _benchmark_revocation SET revision=revision+1; END",
        "CREATE TRIGGER _benchmark_revoke_replace AFTER UPDATE OF generation ON frame_payloads WHEN (SELECT maintenance FROM storage_metadata)=1 AND NEW.generation!=OLD.generation AND (NEW.completed_surfaces!=0 OR NEW.policy!=OLD.policy) BEGIN UPDATE _benchmark_revocation SET revision=revision+1; END",
        "INSERT INTO frames(id,timestamp,full_text) VALUES(1,'2026-09-11T12:00:00Z','sealed original')",
    ] { db.execute_raw_sql_write(sql).await.unwrap(); }
    db.seal_frame_payloads().await.unwrap();
    db.execute_raw_sql_write("INSERT INTO frames(id,timestamp,full_text) VALUES(2,'2026-09-11T12:00:01Z','staged original')").await.unwrap();

    let (ready, wait_ready) = tokio::sync::oneshot::channel();
    let (proceed, wait_proceed) = tokio::sync::oneshot::channel();
    let reader = Arc::clone(&db);
    let reading = tokio::spawn(async move {
        experiments::snapshot(&reader.pool, async {
            let token = reader.storage_read_token().await.unwrap();
            ready.send(()).unwrap();
            wait_proceed.await.unwrap();
            let count = reader
                .query_raw_sql("SELECT count(*) AS n FROM frames")
                .await
                .unwrap();
            assert_eq!(count[0]["n"], 2);
            let payloads = reader
                .frame_payloads(&[1, 2, 3], Projection::All)
                .await
                .unwrap();
            assert_eq!(payloads.len(), 2);
            assert_eq!(payloads[&1].text(), "sealed original");
            assert_eq!(payloads[&2].text(), "staged original");
            token.admit(&reader.pool).await.unwrap();
        })
        .await
        .unwrap();
    });
    wait_ready.await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        db.insert_ocr_text(2,"new capture text","",Arc::new(screenpipe_db::OcrEngine::default())).await.unwrap();
        db.execute_raw_sql_write("INSERT INTO frames(id,timestamp,full_text) VALUES(3,'2026-09-11T12:00:02Z','new capture')").await.unwrap();
        db.seal_frame_payloads().await.unwrap();
    }).await.expect("a snapshot must not block the writer or sealer");
    proceed.send(()).unwrap();
    reading.await.unwrap();
    let current = db
        .frame_payloads(&[1, 2, 3], Projection::All)
        .await
        .unwrap();
    assert_eq!(current.len(), 3);
    assert_eq!(current[&2].text(), "new capture text");

    // Sparse, nullable selections cross frame row groups and bulk row groups.
    let mut tx = db.begin_immediate_with_retry().await.unwrap();
    for id in 10..50 {
        sqlx::query("INSERT INTO frames(id,timestamp,full_text,accessibility_text,text_json) VALUES(?,'2026-09-11T12:00:00Z',?,?,?)")
            .bind(id).bind((id%3!=0).then(||format!("frame {id}"))).bind((id%5!=0).then_some("a11y"))
            .bind((id%2!=0).then_some("[]")).execute(&mut **tx.conn()).await.unwrap();
    }
    tx.commit().await.unwrap();
    let selected = [10, 11, 24, 37, 49];
    let expected = db.frame_payloads(&selected, Projection::All).await.unwrap();
    db.seal_frame_payloads().await.unwrap();
    assert_eq!(
        expected,
        db.frame_payloads(&selected, Projection::All).await.unwrap()
    );
    db.execute_raw_sql_write("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<10000) INSERT INTO elements(id,frame_id,source,role,text,depth,sort_order,on_screen) SELECT i,10+i%40,'ocr','text',CASE WHEN i%3=0 THEN NULL ELSE 'selected' END,0,i,CASE WHEN i%5=0 THEN NULL ELSE i%2 END FROM n").await.unwrap();
    let expected = db.get_frame_elements(11, None).await.unwrap();
    db.seal_payloads().await.unwrap();
    assert_eq!(
        serde_json::to_value(expected).unwrap(),
        serde_json::to_value(db.get_frame_elements(11, None).await.unwrap()).unwrap()
    );

    let readers = (0..db.pool.options().get_max_connections() + 4)
        .map(|_| {
            let db = Arc::clone(&db);
            tokio::spawn(async move {
                experiments::snapshot(&db.pool, async {
                    let token = db.storage_read_token().await.unwrap();
                    assert_eq!(
                        db.frame_payloads(&[10], Projection::All)
                            .await
                            .unwrap()
                            .len(),
                        1
                    );
                    token.admit(&db.pool).await.unwrap();
                })
                .await
                .unwrap();
            })
        })
        .collect::<Vec<_>>();
    tokio::time::timeout(Duration::from_secs(5), async {
        for reader in readers {
            reader.await.unwrap();
        }
    })
    .await
    .expect("snapshot readers must reserve admission capacity");

    // A prepared response cannot be admitted after a replacement or deletion.
    for deletion in [false, true] {
        let (ready, wait_ready) = tokio::sync::oneshot::channel();
        let (proceed, wait_proceed) = tokio::sync::oneshot::channel();
        let reader = Arc::clone(&db);
        let reading = tokio::spawn(async move {
            experiments::snapshot(&reader.pool, async {
                let token = reader.storage_read_token().await.unwrap();
                reader.frame_payloads(&[1], Projection::All).await.unwrap();
                ready.send(()).unwrap();
                wait_proceed.await.unwrap();
                assert!(token.admit(&reader.pool).await.is_err());
            })
            .await
            .unwrap();
        });
        wait_ready.await.unwrap();
        if deletion {
            db.execute_raw_sql_write("DELETE FROM frames WHERE id=1")
                .await
                .unwrap();
        } else {
            let mut payload = db
                .frame_payloads(&[1], Projection::All)
                .await
                .unwrap()
                .remove(&1)
                .unwrap();
            payload.full_text = Some("redacted".into());
            assert!(db
                .replace_frame_payload(&payload, "", 15, None, None)
                .await
                .unwrap());
        }
        proceed.send(()).unwrap();
        reading.await.unwrap();
    }

    // Dropping a request releases its transaction so future reads and close drain.
    let (ready, wait_ready) = tokio::sync::oneshot::channel();
    let reader = Arc::clone(&db);
    let reading = tokio::spawn(async move {
        experiments::snapshot(&reader.pool, async {
            let _token = reader.storage_read_token().await.unwrap();
            ready.send(()).unwrap();
            std::future::pending::<()>().await;
        })
        .await
        .unwrap();
    });
    wait_ready.await.unwrap();
    reading.abort();
    let _ = reading.await;
    tokio::time::timeout(Duration::from_secs(2), db.close())
        .await
        .unwrap();
}
