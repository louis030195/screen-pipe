// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use screenpipe_db::{storage::Projection, DatabaseManager};
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn existing_hybrid_index_builds_lookup_and_maintains_mutations() {
    let root = tempfile::tempdir().unwrap();
    let db = DatabaseManager::new_hybrid(root.path(), Default::default(), Default::default())
        .await
        .unwrap();
    db.execute_raw_sql_write("INSERT INTO frames(id,timestamp,full_text) VALUES(1,'2026-09-11T12:00:00Z','one'); WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<9000) INSERT INTO elements(id,frame_id,source,role,text,sort_order,on_screen) SELECT i,1,'ocr','text','lookup token',i,CASE WHEN i%2=0 THEN NULL ELSE 1 END FROM n").await.unwrap();
    let expected = serde_json::to_value(
        db.search_elements("token", None, None, None, None, None, None, None, 25, 9)
            .await
            .unwrap(),
    )
    .unwrap();
    db.seal_payloads().await.unwrap();
    // Reproduce the pre-optimization catalog on a disposable, sealed database.
    db.execute_raw_sql_write("DROP TRIGGER hybrid_read_element_lookup_INSERT; DROP TRIGGER hybrid_read_element_lookup_UPDATE; DROP VIEW _bulk_element_search; DROP TABLE _bulk_element_lookup; DELETE FROM _hybrid_migrations WHERE version=3").await.unwrap();
    db.close().await;
    let db = DatabaseManager::new(
        root.path().join("db.sqlite").to_str().unwrap(),
        Default::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        expected,
        serde_json::to_value(
            db.search_elements("token", None, None, None, None, None, None, None, 25, 9)
                .await
                .unwrap()
        )
        .unwrap()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM _bulk_element_lookup")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        9000
    );
    db.execute_raw_sql_write("INSERT INTO elements(id,frame_id,source,role,text) VALUES(9001,1,'accessibility','AXButton','fresh token'); UPDATE elements SET role='AXLink',text='updated token' WHERE id=9001; DELETE FROM elements WHERE id=1").await.unwrap();
    let (found, count) = db
        .search_elements(
            "updated",
            None,
            None,
            Some("AXLink"),
            None,
            None,
            None,
            None,
            25,
            0,
        )
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(found[0].id, 9001);
    db.seal_payloads().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM _bulk_element_lookup")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        9000
    );
    assert_eq!(
        db.search_elements(
            "updated",
            None,
            None,
            Some("AXLink"),
            None,
            None,
            None,
            None,
            25,
            0
        )
        .await
        .unwrap()
        .1,
        1
    );
    db.close().await;
}

#[tokio::test]
async fn snapshots_are_scoped_to_their_manager_and_interrupt_abandoned_sql() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let mut config = screenpipe_config::DbConfig::default();
    config.read_pool_max = 2;
    config.read_pool_min = 1;
    let a = Arc::new(
        DatabaseManager::new_hybrid(first.path(), config.clone(), Default::default())
            .await
            .unwrap(),
    );
    let b = DatabaseManager::new_hybrid(second.path(), config, Default::default())
        .await
        .unwrap();
    a.execute_raw_sql_write(
        "INSERT INTO frames(id,timestamp,full_text) VALUES(1,'2026-09-11','first')",
    )
    .await
    .unwrap();
    b.execute_raw_sql_write(
        "INSERT INTO frames(id,timestamp,full_text) VALUES(1,'2026-09-11','second')",
    )
    .await
    .unwrap();
    a.read_snapshot(async {
        assert_eq!(
            a.frame_payloads(&[1], Projection::All).await.unwrap()[&1].text(),
            "first"
        );
        assert_eq!(
            b.frame_payloads(&[1], Projection::All).await.unwrap()[&1].text(),
            "second"
        );
    })
    .await
    .unwrap();
    let (ready, wait_ready) = tokio::sync::oneshot::channel();
    let reader = Arc::clone(&a);
    let task = tokio::spawn(async move {
        reader.read_snapshot(async {
            let mut connection=reader.acquire_search_read().await.unwrap();
            ready.send(()).unwrap();
            sqlx::query("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<1000000000) SELECT sum(i) FROM n").fetch_one(&mut *connection).await
        }).await
    });
    wait_ready.await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!task.is_finished());
    task.abort();
    let _ = task.await;
    tokio::time::timeout(
        Duration::from_secs(2),
        a.frame_payloads(&[1], Projection::All),
    )
    .await
    .unwrap()
    .unwrap();
    tokio::time::timeout(Duration::from_secs(2), a.close())
        .await
        .unwrap();
    let b = Arc::new(b);
    let reader = Arc::clone(&b);
    let (ready, wait_ready) = tokio::sync::oneshot::channel();
    let running = tokio::spawn(async move {
        reader.read_snapshot(async {
            let mut connection = reader.acquire_search_read().await.unwrap();
            ready.send(()).unwrap();
            sqlx::query("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<1000000000) SELECT sum(i) FROM n").fetch_one(&mut *connection).await
        }).await
    });
    wait_ready.await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    tokio::time::timeout(Duration::from_secs(2), b.close())
        .await
        .unwrap();
    assert!(running.await.unwrap().is_err());
}

#[tokio::test]
async fn snapshot_preserves_rows_across_writes_and_rejects_revocation() {
    let root = tempfile::tempdir().unwrap();
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
    db.execute_raw_sql_write("INSERT INTO frames(id,timestamp,full_text) VALUES(1,'2026-09-11T12:00:00Z','sealed original')").await.unwrap();
    db.seal_frame_payloads().await.unwrap();
    db.execute_raw_sql_write("INSERT INTO frames(id,timestamp,full_text) VALUES(2,'2026-09-11T12:00:01Z','staged original')").await.unwrap();

    let (ready, wait_ready) = tokio::sync::oneshot::channel();
    let (proceed, wait_proceed) = tokio::sync::oneshot::channel();
    let reader = Arc::clone(&db);
    let reading = tokio::spawn(async move {
        reader
            .read_snapshot(async {
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
                db.read_snapshot(async {
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
            reader
                .read_snapshot(async {
                    let token = reader.storage_read_token().await.unwrap();
                    reader.frame_payloads(&[1], Projection::All).await.unwrap();
                    ready.send(()).unwrap();
                    wait_proceed.await.unwrap();
                    assert!(token.admit(&reader.pool).await.is_err());
                })
                .await
                .expect_err("the whole response must be revoked");
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
        reader
            .read_snapshot(async {
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
