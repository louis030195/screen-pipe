// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use super::*;
use crate::DatabaseManager;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Condvar,
};
use std::time::Duration;

struct Pause(Arc<(Mutex<bool>, Condvar)>);
impl Pause {
    fn new() -> Self {
        Self(Arc::new((Mutex::new(false), Condvar::new())))
    }
    fn wait(&self) {
        let (lock, wake) = &*self.0;
        let (_guard, timeout) = wake
            .wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(10), |done| !*done)
            .unwrap();
        assert!(!timeout.timed_out(), "test did not release decoder");
    }
    fn release(&self) {
        *self.0 .0.lock().unwrap() = true;
        self.0 .1.notify_all();
    }
}
impl Drop for Pause {
    fn drop(&mut self) {
        self.release();
    }
}

async fn fixture_mode(hybrid: bool) -> (tempfile::TempDir, Arc<DatabaseManager>) {
    let root = tempfile::tempdir().unwrap();
    let db = Arc::new(
        if hybrid {
            DatabaseManager::new_hybrid(root.path(), Default::default(), Default::default()).await
        } else {
            DatabaseManager::new(
                root.path().join("db.sqlite").to_str().unwrap(),
                Default::default(),
            )
            .await
        }
        .unwrap(),
    );
    db.execute_raw_sql_write("INSERT INTO frames(id,timestamp,full_text) VALUES(1,'2026-09-12','frame'); INSERT INTO elements(id,frame_id,source,role,text) VALUES(1,1,'accessibility','AXText','original'); INSERT INTO audio_chunks(id,file_path) VALUES(1,'test.wav'); INSERT INTO audio_transcriptions(id,audio_chunk_id,offset_index,timestamp,transcription,device) VALUES(1,1,0,'2026-09-12','audio original','test');").await.unwrap();
    if hybrid {
        while db.seal_payloads().await.unwrap() != 0 {}
    }
    (root, db)
}

async fn fixture() -> (tempfile::TempDir, Arc<DatabaseManager>) {
    fixture_mode(true).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cache_hits_and_unrelated_readers_progress_during_a_decode() {
    let (_root, db) = fixture().await;
    let storage = db.storage.as_ref().unwrap();
    let element_path: String =
        sqlx::query_scalar("SELECT path FROM _bulk_files WHERE table_name='elements'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let expected: String =
        sqlx::query_scalar("SELECT transcription FROM audio_transcriptions WHERE id=1")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    // Force only the element file cold; the audio cache hit must remain usable.
    storage.bulk.cache.retire(&[element_path.clone()]).unwrap();
    let capacity = storage.decoder.clone().acquire_owned().await.unwrap();
    let pause = Pause::new();
    let hold = pause.0.clone();
    let (started, ready) = tokio::sync::oneshot::channel();
    let started = Mutex::new(Some(started));
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    *storage.bulk.decode_hook.lock().unwrap() = Some(Arc::new(move |key| {
        if key.starts_with("element-frame:") && key.contains(&element_path) {
            counter.fetch_add(1, Ordering::SeqCst);
            if let Some(started) = started.lock().unwrap().take() {
                let _ = started.send(());
            }
            Pause(hold.clone()).wait();
        }
    }));
    let reader = db.clone();
    let reading = tokio::spawn(async move {
        sqlx::query_scalar::<_, String>("SELECT text FROM elements WHERE id=1")
            .fetch_one(&reader.pool)
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), ready)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(storage.decoder.available_permits(), 0);
    let hit = tokio::time::timeout(
        Duration::from_secs(1),
        sqlx::query_scalar::<_, String>(
            "SELECT transcription FROM audio_transcriptions WHERE id=1",
        )
        .fetch_one(&db.pool),
    )
    .await;
    let write = tokio::time::timeout(
        Duration::from_secs(1),
        db.execute_raw_sql_write("INSERT INTO frames(id,timestamp,full_text) VALUES(2,'2026-09-12','concurrent capture')"),
    )
    .await;
    pause.release();
    assert_eq!(reading.await.unwrap().unwrap(), "original");
    assert_eq!(
        hit.expect("cached reader was blocked by an unrelated decoder")
            .unwrap(),
        expected
    );
    write.unwrap().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    drop(capacity);
    db.close().await;
}

#[tokio::test]
async fn sql_readers_start_during_file_unlink_without_interrupts() {
    let (_root, db) = fixture().await;
    let storage = db.storage.as_ref().unwrap();
    // Reclamation has passed its reader cutoff. New SQLite snapshots can only
    // see current locators, so they can read while obsolete files are unlinked.
    let unlink = storage.leases.clone().write_owned().await;
    let result = sqlx::query_scalar::<_, String>("SELECT text FROM elements WHERE id=1")
        .fetch_one(&db.pool)
        .await;
    drop(unlink);
    assert_eq!(result.unwrap(), "original");
    db.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cold_files_decode_concurrently_with_shared_bounded_admission() {
    let (_root, db) = fixture().await;
    let storage = db.storage.as_ref().unwrap();
    let paths: Vec<String> = sqlx::query_scalar("SELECT path FROM _bulk_files")
        .fetch_all(&db.pool)
        .await
        .unwrap();
    storage.bulk.cache.retire(&paths).unwrap();
    let pause = Pause::new();
    let hold = pause.0.clone();
    let (started, mut ready) = tokio::sync::mpsc::unbounded_channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    *storage.bulk.decode_hook.lock().unwrap() = Some(Arc::new(move |key| {
        if key.starts_with("records:") || key.starts_with("element-frame:") {
            counter.fetch_add(1, Ordering::SeqCst);
            started.send(()).unwrap();
            Pause(hold.clone()).wait();
        }
    }));
    let mut reads = Vec::new();
    for sql in [
        "SELECT text FROM elements WHERE id=1",
        "SELECT transcription FROM audio_transcriptions WHERE id=1",
        "SELECT text FROM elements WHERE id=1",
    ] {
        let db = db.clone();
        reads.push(tokio::spawn(async move {
            sqlx::query_scalar::<_, String>(sql)
                .fetch_one(&db.pool)
                .await
        }));
    }
    for _ in 0..2 {
        tokio::time::timeout(Duration::from_secs(2), ready.recv())
            .await
            .expect("independent cold files did not decode concurrently")
            .unwrap();
    }
    assert_eq!(storage.decoder.available_permits(), 0);
    tokio::time::timeout(Duration::from_secs(2), async {
        while storage.bulk.cache.max_in_flight_readers() < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("same-file reader did not join the pending decode");
    // Frame payloads use the same two decoder slots. Cancelling a queued
    // frame read must leave these active bulk jobs and their leases intact.
    assert!(tokio::time::timeout(
        Duration::from_millis(100),
        db.frame_payloads(&[1], crate::storage::Projection::All)
    )
    .await
    .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    pause.release();
    for (read, expected) in reads
        .into_iter()
        .zip(["original", "audio original", "original"])
    {
        assert_eq!(read.await.unwrap().unwrap(), expected);
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "same-file readers must share the decode"
    );
    // Join the already-dispatched frame decode before checking its permit.
    assert_eq!(
        db.frame_payloads(&[1], crate::storage::Projection::All)
            .await
            .unwrap()[&1]
            .full_text
            .as_deref(),
        Some("frame")
    );
    assert_eq!(storage.decoder.available_permits(), 2);
    db.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn snapshots_and_streams_match_sqlite_while_writing_and_reclaiming() {
    use futures::TryStreamExt;
    for hybrid in [false, true] {
        let (root, db) = fixture_mode(hybrid).await;
        let originals: Vec<String> = if hybrid {
            sqlx::query_scalar("SELECT path FROM _bulk_files")
                .fetch_all(&db.pool)
                .await
                .unwrap()
        } else {
            vec![]
        };
        let mut snapshot = db.pool.begin().await.unwrap();
        // Establish the snapshot before requesting any archived column.
        let _: i64 = sqlx::query_scalar("SELECT count(*) FROM frames")
            .fetch_one(&mut *snapshot)
            .await
            .unwrap();
        let mut connection = db.pool.acquire().await.unwrap();
        let mut stream = sqlx::query_scalar::<_, String>("WITH RECURSIVE seq(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM seq WHERE n<1000) SELECT e.text FROM seq CROSS JOIN elements e WHERE e.id=1").fetch(&mut *connection);
        assert_eq!(
            stream.try_next().await.unwrap().as_deref(),
            Some("original")
        );
        tokio::time::timeout(Duration::from_secs(2), db.execute_raw_sql_write("UPDATE elements SET text='replacement' WHERE id=1; UPDATE audio_transcriptions SET transcription='audio replacement' WHERE id=1;")).await.unwrap().unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT text FROM elements WHERE id=1")
                .fetch_one(&db.pool)
                .await
                .unwrap(),
            "replacement"
        );
        if hybrid {
            for _ in 0..3 {
                db.reclaim_frame_payloads().await.unwrap();
            }
            assert!(originals.iter().all(|p| root.path().join(p).exists()));
            db.storage
                .as_ref()
                .unwrap()
                .bulk
                .cache
                .retire(&originals)
                .unwrap();
        }
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT text FROM elements WHERE id=1")
                .fetch_one(&mut *snapshot)
                .await
                .unwrap(),
            "original"
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT transcription FROM audio_transcriptions WHERE id=1"
            )
            .fetch_one(&mut *snapshot)
            .await
            .unwrap(),
            "audio original"
        );
        snapshot.rollback().await.unwrap();
        if hybrid {
            for _ in 0..3 {
                db.reclaim_frame_payloads().await.unwrap();
            }
            assert!(
                originals.iter().all(|p| root.path().join(p).exists()),
                "stream must pin originals independently of transaction"
            );
        }
        let mut rows = 1;
        while let Some(text) = stream.try_next().await.unwrap() {
            assert_eq!(text, "original");
            rows += 1;
        }
        assert_eq!(rows, 1000);
        drop(stream);
        drop(connection);
        if hybrid {
            for _ in 0..3 {
                db.reclaim_frame_payloads().await.unwrap();
            }
            assert!(originals.iter().all(|p| !root.path().join(p).exists()));
        }
        db.verify_storage().await.unwrap();
        db.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_releases_sql_workers_waiting_for_decoder_admission() {
    let (_root, db) = fixture().await;
    let storage = db.storage.as_ref().unwrap();
    let paths: Vec<String> = sqlx::query_scalar("SELECT path FROM _bulk_files")
        .fetch_all(&db.pool)
        .await
        .unwrap();
    storage.bulk.cache.retire(&paths).unwrap();
    let capacity = storage.decoder.clone().acquire_many_owned(2).await.unwrap();
    let reader = db.clone();
    let reading = tokio::spawn(async move {
        sqlx::query_scalar::<_, String>("SELECT text FROM elements WHERE id=1")
            .fetch_one(&reader.pool)
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while storage.bulk.cache.max_in_flight_readers() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let owner = db.clone();
    let closing = tokio::spawn(async move { owner.close().await });
    assert!(tokio::time::timeout(Duration::from_secs(2), reading)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    drop(capacity);
    tokio::time::timeout(Duration::from_secs(2), closing)
        .await
        .unwrap()
        .unwrap();
    assert!(!storage.sql_readers_active());
}
