// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use screenpipe_db::DatabaseManager;

#[tokio::test]
async fn element_parent_order_and_export_preserve_complete_transactions() {
    let root = tempfile::tempdir().unwrap();
    let db = DatabaseManager::new_hybrid(root.path(), Default::default(), Default::default())
        .await
        .unwrap();
    db.execute_raw_sql_write("INSERT INTO frames(id,timestamp,full_text) VALUES(1,'2026-09-11T12:00:00Z','frame'); INSERT INTO elements(id,frame_id,source,role,parent_id,text) VALUES(-3,1,'accessibility','AXText',7,'child'),(7,1,'accessibility','AXGroup',NULL,'parent');").await.unwrap();
    while db.seal_payloads().await.unwrap() != 0 {}
    let expected = db
        .query_raw_sql("SELECT * FROM elements ORDER BY id")
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT id FROM elements WHERE id='7'")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        7
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM elements WHERE frame_id='1'")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        2
    );
    db.verify_storage().await.unwrap();
    db.close().await;
    let output = tempfile::tempdir().unwrap();
    let path = output.path().join("export.sqlite");
    screenpipe_db::storage::export_sqlite(root.path(), &path, Default::default())
        .await
        .unwrap();
    let db = DatabaseManager::new(path.to_str().unwrap(), Default::default())
        .await
        .unwrap();
    assert_eq!(
        db.query_raw_sql("SELECT * FROM elements ORDER BY id")
            .await
            .unwrap(),
        expected
    );
    db.verify_storage().await.unwrap();
    db.close().await;
}

#[tokio::test]
async fn complete_element_ranges_preserve_mutations_relationships_and_browsing() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("db.sqlite");
    let db = DatabaseManager::new(path.to_str().unwrap(), Default::default())
        .await
        .unwrap();
    db.execute_raw_sql_write("INSERT INTO frames(id,timestamp,full_text) VALUES(1,'2026-09-11T12:00:00Z','one'),(2,'2026-09-11T12:01:00Z','two'); WITH RECURSIVE seq(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM seq WHERE x<33000) INSERT INTO elements(id,frame_id,source,role,text,parent_id,sort_order,on_screen) SELECT x*2,1+(x%2),'accessibility',CASE WHEN x%3=0 THEN 'AXButton' ELSE 'AXText' END,'range token '||x,CASE WHEN x=2 THEN 2 ELSE NULL END,x,CASE WHEN x%3=0 THEN NULL ELSE x%2 END FROM seq; INSERT INTO elements(id,frame_id,source,role,text) VALUES(-9223372036854775808,1,'ocr','word','minimum token');").await.unwrap();
    let expected = db
        .search_elements(
            "",
            None,
            None,
            Some("AXButton"),
            None,
            None,
            None,
            None,
            25,
            9,
        )
        .await
        .unwrap();
    let ranking = db.query_raw_sql("SELECT rowid,bm25(elements_fts) AS score FROM elements_fts WHERE elements_fts MATCH 'token' ORDER BY rowid LIMIT 128").await.unwrap();
    db.close().await;
    let report =
        screenpipe_db::storage::migrate(root.path(), Default::default(), Default::default())
            .await
            .unwrap();
    assert_eq!(
        report
            .tables
            .iter()
            .find(|t| t.table == "elements")
            .unwrap()
            .rows,
        33001
    );
    let db = DatabaseManager::new(path.to_str().unwrap(), Default::default())
        .await
        .unwrap();
    assert_eq!(db.query_raw_sql("SELECT rowid,bm25(elements_fts) AS score FROM elements_fts WHERE elements_fts MATCH 'token' ORDER BY rowid LIMIT 128").await.unwrap(),ranking);
    assert_eq!(
        serde_json::to_value(
            db.search_elements(
                "",
                None,
                None,
                Some("AXButton"),
                None,
                None,
                None,
                None,
                25,
                9
            )
            .await
            .unwrap()
        )
        .unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM _bulk_element_rows")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        0
    );
    assert!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM _bulk_element_ranges")
            .fetch_one(&db.pool)
            .await
            .unwrap()
            >= 2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT id FROM elements ORDER BY id DESC LIMIT 1")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        66000
    );
    assert!(db
        .execute_raw_sql_write("DELETE FROM elements WHERE id=2")
        .await
        .is_err());
    assert!(db.execute_raw_sql_write("INSERT INTO elements(id,frame_id,source,role,parent_id) VALUES(99001,1,'ocr','word',99000)").await.is_err());
    assert!(db
        .execute_raw_sql_write("DELETE FROM frames WHERE id=1")
        .await
        .is_err());
    db.execute_raw_sql_write("SAVEPOINT probe; UPDATE elements SET text='rolled back',frame_id=2 WHERE id=4; ROLLBACK TO probe; RELEASE probe;").await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT text FROM elements WHERE id=4")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        "range token 2"
    );
    db.execute_raw_sql_write("INSERT INTO elements(id,frame_id,source,role,text) VALUES(35001,2,'ocr','word','inserted gap token'); UPDATE elements SET frame_id=1,source='ocr',role='word',on_screen=NULL,text='changed token' WHERE id=66000; DELETE FROM elements WHERE id IN (2,4);").await.unwrap();
    while db.seal_payloads().await.unwrap() != 0 {}
    db.reclaim_frame_payloads().await.unwrap();
    db.verify_storage().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT text FROM elements WHERE id=35001")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        "inserted gap token"
    );
    assert_eq!(
        db.search_elements("changed", None, None, None, None, None, None, None, 10, 0)
            .await
            .unwrap()
            .1,
        1
    );
    assert_eq!(
        db.search_elements("", None, None, None, None, None, None, None, 10, 0)
            .await
            .unwrap()
            .1,
        33000
    );
    db.close().await;
}

async fn seed(db: &DatabaseManager) {
    db.execute_raw_sql_write("INSERT INTO frames(id,timestamp,full_text) VALUES(1,'2026-09-11T12:00:00Z','capture'),(2,'2026-09-11T12:01:00Z','second'); INSERT INTO elements(id,frame_id,source,role,text,depth,left_bound,properties) VALUES(1,1,'accessibility','AXTextField','café 東京 secret',2,0.25,'{\"value\":\"secret\"}'),(2,1,'accessibility','AXGroup',NULL,0,NULL,NULL); INSERT INTO pipe_executions(id,pipe_name,status,finished_at,stdout,stderr) VALUES(1,'test','completed','2026-09-11T12:00:00Z','output payload','error payload'); INSERT INTO audio_chunks(id,file_path) VALUES(1,'audio.wav'); INSERT INTO audio_transcriptions(id,audio_chunk_id,offset_index,timestamp,transcription,device) VALUES(1,1,0,'2026-09-11T12:00:00Z','spoken secret','microphone'); INSERT INTO ui_events(id,timestamp,event_type,text_content,element_value,element_ancestors) VALUES(1,'2026-09-11T12:00:00Z','text','typed secret','value secret','[{\"name\":\"secret\"}]');").await.unwrap();
}

async fn seed_remaining(db: &DatabaseManager) {
    db.execute_raw_sql_write("INSERT INTO semantic_items(id,entity_fingerprint,version_fingerprint,kind,item_key,identity_quality,title,body,metadata_json) VALUES(1,zeroblob(32),zeroblob(32),'document','document-one','stable','title','semantic secret','{\"summary\":\"secret\"}'); INSERT INTO outputs(id,source,title,output_path,preview,metadata) VALUES(1,'test','output','test.md','preview secret','{\"value\":1}'); INSERT INTO meetings(id,meeting_start,meeting_app) VALUES(1,'2026-09-11T12:00:00Z','test'); INSERT INTO meeting_transcript_segments(id,meeting_id,provider,item_id,transcript,captured_at) VALUES(1,1,'test','one','meeting secret','2026-09-11T12:00:00Z');").await.unwrap();
}

async fn logical(db: &DatabaseManager) -> serde_json::Value {
    db.query_raw_sql("SELECT e.id,e.frame_id,e.source,e.role,e.text,e.depth,e.left_bound,e.properties,p.stdout,p.stderr,a.transcription,u.text_content,u.element_value,u.element_ancestors FROM elements e CROSS JOIN pipe_executions p CROSS JOIN audio_transcriptions a CROSS JOIN ui_events u ORDER BY e.id").await.unwrap()
}

#[tokio::test]
async fn bulk_columns_search_overrides_and_reopening_preserve_values() {
    let root = tempfile::tempdir().unwrap();
    let db = DatabaseManager::new_hybrid(root.path(), Default::default(), Default::default())
        .await
        .unwrap();
    seed(&db).await;
    seed_remaining(&db).await;
    let original = logical(&db).await;
    let staged_revision = db.storage_read_token().await.unwrap();
    while db.seal_payloads().await.unwrap() != 0 {}
    assert!(staged_revision.admit(&db.pool).await.is_ok());
    drop(staged_revision);
    assert_eq!(logical(&db).await, original);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM _bulk_element_rows")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db.search_elements("secret", None, None, None, None, None, None, None, 10, 0)
            .await
            .unwrap()
            .1,
        1
    );
    db.execute_raw_sql_write("INSERT OR IGNORE INTO audio_transcriptions(audio_chunk_id,offset_index,timestamp,transcription,device) VALUES(1,1,'2026-09-11T12:00:00Z','spoken secret','microphone'); INSERT INTO audio_transcriptions(id,audio_chunk_id,offset_index,timestamp,transcription,device) VALUES(2,1,1,'2026-09-11T12:00:00Z','different words','microphone');").await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM audio_transcriptions")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        2
    );
    assert!(db
        .execute_raw_sql_write(
            "UPDATE audio_transcriptions SET transcription='spoken secret' WHERE id=2"
        )
        .await
        .is_err());
    db.execute_raw_sql_write("DELETE FROM audio_transcriptions WHERE id=2")
        .await
        .unwrap();
    let revision = db.storage_read_token().await.unwrap();
    db.execute_raw_sql_write("UPDATE elements SET text=NULL,properties='{\"value\":\"safe\"}' WHERE id=1; UPDATE audio_transcriptions SET speaker_id=NULL WHERE id=1; UPDATE ui_events SET text_content='safe',element_value=NULL WHERE id=1; UPDATE pipe_executions SET stderr=NULL WHERE id=1;").await.unwrap();
    assert!(revision.admit(&db.pool).await.is_err());
    drop(revision);
    db.execute_raw_sql_write("UPDATE semantic_items SET body='semantic replacement',metadata_json='{\"summary\":\"safe\"}' WHERE id=1; UPDATE outputs SET preview=NULL WHERE id=1; UPDATE meeting_transcript_segments SET transcript='meeting replacement' WHERE id=1;").await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM semantic_items_fts WHERE semantic_items_fts MATCH 'secret'"
        )
        .fetch_one(&db.pool)
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM semantic_items_fts WHERE semantic_items_fts MATCH 'replacement'"
        )
        .fetch_one(&db.pool)
        .await
        .unwrap(),
        1
    );
    let remaining = db.query_raw_sql("SELECT s.body,s.metadata_json,o.preview,o.metadata,m.transcript FROM semantic_items s,outputs o,meeting_transcript_segments m").await.unwrap();
    assert!(remaining[0]["preview"].is_null());
    let changed = logical(&db).await;
    assert_eq!(db.get_frame_elements(1, None).await.unwrap()[0].text, None);
    assert_eq!(db.get_frame_elements(1, None).await.unwrap()[0].depth, 2);
    assert_eq!(
        db.search_elements("secret", None, None, None, None, None, None, None, 10, 0)
            .await
            .unwrap()
            .1,
        0
    );
    while db.seal_payloads().await.unwrap() != 0 {}
    db.reclaim_frame_payloads().await.unwrap();
    assert_eq!(logical(&db).await, changed);
    assert_eq!(db.query_raw_sql("SELECT s.body,s.metadata_json,o.preview,o.metadata,m.transcript FROM semantic_items s,outputs o,meeting_transcript_segments m").await.unwrap(),remaining);
    db.verify_storage().await.unwrap();
    db.close().await;
    let db = DatabaseManager::new(
        root.path().join("db.sqlite").to_str().unwrap(),
        Default::default(),
    )
    .await
    .unwrap();
    assert_eq!(logical(&db).await, changed);
    db.execute_raw_sql_write("UPDATE elements SET frame_id=2,source='ocr',role='word' WHERE id=1")
        .await
        .unwrap();
    let elements = db.get_frame_elements(2, None).await.unwrap();
    assert_eq!(elements.len(), 1);
    assert_eq!(elements[0].role, "word");
    db.execute_raw_sql_write("DELETE FROM elements; DELETE FROM frames; DELETE FROM audio_transcriptions; DELETE FROM ui_events; DELETE FROM pipe_executions; DELETE FROM semantic_items; DELETE FROM outputs; DELETE FROM meetings;").await.unwrap();
    for _ in 0..20 {
        db.reclaim_frame_payloads().await.unwrap();
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM _bulk_files")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        0
    );
    db.close().await;
}

#[tokio::test]
async fn bulk_backup_export_and_snapshot_leases_preserve_records() {
    let root = tempfile::tempdir().unwrap();
    let db = DatabaseManager::new_hybrid(root.path(), Default::default(), Default::default())
        .await
        .unwrap();
    seed(&db).await;
    while db.seal_payloads().await.unwrap() != 0 {}
    let expected = logical(&db).await;
    let originals: Vec<String> = sqlx::query_scalar("SELECT path FROM _bulk_files")
        .fetch_all(&db.pool)
        .await
        .unwrap();
    let mut snapshot = db.pool.begin().await.unwrap();
    let text: String = sqlx::query_scalar("SELECT text FROM elements WHERE id=1")
        .fetch_one(&mut *snapshot)
        .await
        .unwrap();
    db.execute_raw_sql_write("UPDATE elements SET text='replacement' WHERE id=1")
        .await
        .unwrap();
    for _ in 0..3 {
        db.reclaim_frame_payloads().await.unwrap();
    }
    assert!(originals.iter().all(|p| root.path().join(p).exists()));
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT text FROM elements WHERE id=1")
            .fetch_one(&mut *snapshot)
            .await
            .unwrap(),
        text
    );
    snapshot.rollback().await.unwrap();
    db.execute_raw_sql_write("UPDATE elements SET text='café 東京 secret' WHERE id=1")
        .await
        .unwrap();
    while db.seal_payloads().await.unwrap() != 0 {}
    let backup = root.path().join("backup");
    db.backup_to(backup.to_str().unwrap()).await.unwrap();
    let restored = root.path().join("restored");
    screenpipe_db::storage::restore(&backup, &restored, Default::default())
        .await
        .unwrap();
    let restored_db = DatabaseManager::new(
        restored.join("db.sqlite").to_str().unwrap(),
        Default::default(),
    )
    .await
    .unwrap();
    assert_eq!(logical(&restored_db).await, expected);
    restored_db.close().await;
    db.close().await;
    let export = root.path().join("export.sqlite");
    screenpipe_db::storage::export_sqlite(root.path(), &export, Default::default())
        .await
        .unwrap();
    let exported = DatabaseManager::new(export.to_str().unwrap(), Default::default())
        .await
        .unwrap();
    assert_eq!(logical(&exported).await, expected);
    exported.close().await;

    let remigrated_root = tempfile::tempdir().unwrap();
    let remigrated_path = remigrated_root.path().join("db.sqlite");
    std::fs::copy(&export, &remigrated_path).unwrap();
    screenpipe_db::storage::migrate(
        remigrated_root.path(),
        Default::default(),
        Default::default(),
    )
    .await
    .unwrap();
    let remigrated = DatabaseManager::new(remigrated_path.to_str().unwrap(), Default::default())
        .await
        .unwrap();
    assert_eq!(logical(&remigrated).await, expected);
    remigrated.verify_storage().await.unwrap();
    remigrated.close().await;
}

#[cfg(feature = "storage-fault-injection")]
#[tokio::test]
async fn bulk_publication_interruptions_resume_with_full_parity() {
    for point in [
        "bulk_reserved",
        "bulk_files_synced",
        "bulk_before_commit",
        "bulk_committed",
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("db.sqlite");
        let source = DatabaseManager::new(path.to_str().unwrap(), Default::default())
            .await
            .unwrap();
        seed(&source).await;
        let expected = logical(&source).await;
        source.close().await;
        let crashed = std::process::Command::new(env!("CARGO_BIN_EXE_screenpipe-storage"))
            .arg("migrate")
            .arg(root.path())
            .env("SCREENPIPE_STORAGE_CRASH_AT", point)
            .output()
            .unwrap();
        assert_eq!(
            crashed.status.code(),
            Some(86),
            "{point}: {}",
            String::from_utf8_lossy(&crashed.stderr)
        );
        assert!(path.exists());
        screenpipe_db::storage::migrate(root.path(), Default::default(), Default::default())
            .await
            .unwrap();
        let db = DatabaseManager::new(path.to_str().unwrap(), Default::default())
            .await
            .unwrap();
        assert_eq!(logical(&db).await, expected);
        db.verify_storage().await.unwrap();
        db.close().await;
    }
}

#[tokio::test]
async fn deferred_elements_complete_under_staging_pressure() {
    let root = tempfile::tempdir().unwrap();
    let mut options = screenpipe_db::storage::MigrationOptions::default();
    options.budget.record_bytes = 1024;
    options.budget.staging_bytes = 1024;
    let db = DatabaseManager::new_hybrid(root.path(), Default::default(), options)
        .await
        .unwrap();
    db.execute_raw_sql_write(
        "INSERT INTO frames(id,timestamp,full_text) VALUES(1,'2026-09-11T12:00:00Z','capture');",
    )
    .await
    .unwrap();
    let mut tx = db.begin_immediate_with_retry().await.unwrap();
    for _ in 0..3 {
        sqlx::query(
            "INSERT INTO elements(frame_id,source,role,text) VALUES(1,'accessibility','AXText',?)",
        )
        .bind("x".repeat(500))
        .execute(&mut **tx.conn())
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    assert_eq!(db.get_frame_elements(1, None).await.unwrap().len(), 3);
    assert!(db
        .execute_raw_sql_write(
            "INSERT INTO frames(id,timestamp,full_text) VALUES(2,'2026-09-11T12:01:00Z','next');"
        )
        .await
        .is_err());
    while db.seal_payloads().await.unwrap() != 0 {}
    db.execute_raw_sql_write(
        "INSERT INTO frames(id,timestamp,full_text) VALUES(2,'2026-09-11T12:01:00Z','next');",
    )
    .await
    .unwrap();
    assert_eq!(db.get_frame_elements(1, None).await.unwrap().len(), 3);
    db.close().await;
}

#[tokio::test]
async fn verification_detects_corruption_even_after_cache_hit() {
    let root = tempfile::tempdir().unwrap();
    let db = DatabaseManager::new_hybrid(root.path(), Default::default(), Default::default())
        .await
        .unwrap();
    seed(&db).await;
    while db.seal_payloads().await.unwrap() != 0 {}
    db.get_frame_elements(1, None).await.unwrap();
    let path: String =
        sqlx::query_scalar("SELECT path FROM _bulk_files WHERE table_name='elements'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    std::fs::write(root.path().join(path), b"corrupt").unwrap();
    assert!(db.verify_storage().await.is_err());
    db.close().await;
}

#[tokio::test]
async fn speaker_reassignment_matches_sealed_transcripts_inside_transaction() {
    let root = tempfile::tempdir().unwrap();
    let db = DatabaseManager::new_hybrid(root.path(), Default::default(), Default::default())
        .await
        .unwrap();
    seed(&db).await;
    seed_remaining(&db).await;
    db.execute_raw_sql_write("UPDATE meeting_transcript_segments SET transcript='spoken secret' WHERE id=1; INSERT INTO meeting_transcript_segments(id,meeting_id,provider,item_id,transcript,captured_at) VALUES(2,1,'test','two','different words','2026-09-11T12:00:00Z')").await.unwrap();
    while db.seal_payloads().await.unwrap() != 0 {}
    let result = db
        .reassign_speaker(1, "Test Speaker", false, screenpipe_db::ReassignScope::Auto)
        .await
        .unwrap();
    let ids: Vec<(i64, Option<i64>)> =
        sqlx::query_as("SELECT id,speaker_id FROM meeting_transcript_segments ORDER BY id")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(ids, vec![(1, Some(result.speaker_id)), (2, None)]);
    db.close().await;
}
