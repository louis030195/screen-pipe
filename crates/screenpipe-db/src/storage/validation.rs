// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

//! Reproducible comparisons use complete typed results against indexed SQLite.
//! Reports contain timings and counts, never captured text or query terms.
use super::{storage_error, HybridStorage, Projection, StorageDescriptor};
use crate::{ContentType, DatabaseManager, Order};
use serde::Serialize;
use std::{path::Path, time::Instant};

#[derive(Serialize)]
pub struct ComparisonReport {
    pub logical_tables: usize,
    pub logical_rows: u64,
    pub queries: usize,
    pub result_rows: usize,
    pub detail_records: usize,
    pub sqlite_median_ms: f64,
    pub sqlite_p95_ms: f64,
    pub hybrid_median_ms: f64,
    pub hybrid_p95_ms: f64,
    pub cache_condition: &'static str,
    pub frame_transactions: Vec<FrameTransactionReport>,
    pub bulk_queries: usize,
    pub bulk_result_rows: usize,
    pub element_batches: Vec<ElementBatchReport>,
    pub element_search: Vec<ElementSearchReport>,
}

#[derive(Serialize)]
pub struct ElementSearchReport {
    pub case: usize,
    pub on_screen: Option<bool>,
    pub matches: i64,
    pub sqlite_ms: f64,
    pub hybrid_ms: f64,
}

#[derive(Serialize)]
pub struct ElementBatchReport {
    pub mode: &'static str,
    pub batches: usize,
    pub elements: i64,
    pub median_ms: f64,
    pub p95_ms: f64,
}

async fn bulk_queries(
    source: &DatabaseManager,
    target: &DatabaseManager,
    terms: &[String],
) -> Result<(usize, usize, Vec<ElementSearchReport>), sqlx::Error> {
    let mut queries = 0;
    let mut rows = 0;
    let mut timings = Vec::new();
    for query in terms.iter().filter(|q| !q.is_empty()).take(6) {
        for on_screen in [None, Some(true), Some(false)] {
            let start = Instant::now();
            let left = source
                .search_elements(query, None, None, None, None, None, None, on_screen, 25, 7)
                .await?;
            let sqlite_ms = start.elapsed().as_secs_f64() * 1000.0;
            let start = Instant::now();
            let right = target
                .search_elements(query, None, None, None, None, None, None, on_screen, 25, 7)
                .await?;
            let hybrid_ms = start.elapsed().as_secs_f64() * 1000.0;
            if serde_json::to_vec(&left).map_err(storage_error)?
                != serde_json::to_vec(&right).map_err(storage_error)?
            {
                return Err(storage_error("element search/count parity failed"));
            }
            rows += left.0.len();
            timings.push(ElementSearchReport {
                case: timings.len(),
                on_screen,
                matches: left.1,
                sqlite_ms,
                hybrid_ms,
            });
            tracing::info!(
                completed_queries = timings.len(),
                sqlite_ms,
                hybrid_ms,
                "element search comparison"
            );
            queries += 1;
        }
    }
    for table in [
        "elements",
        "audio_transcriptions",
        "ui_events",
        "semantic_items",
        "pipe_executions",
        "meeting_transcript_segments",
        "outputs",
    ] {
        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info(?) ORDER BY cid")
                .bind(table)
                .fetch_all(&source.pool)
                .await?;
        let sql = format!(
            "SELECT {} FROM {table} ORDER BY id DESC LIMIT 64",
            columns.join(",")
        );
        let left = source.query_raw_sql(&sql).await?;
        let right = target.query_raw_sql(&sql).await?;
        if left != right {
            return Err(storage_error(format!(
                "bulk SQL projection parity failed: {table}"
            )));
        }
        rows += left.as_array().map_or(0, Vec::len);
        queries += 1;
    }
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT DISTINCT frame_id FROM elements WHERE frame_id % 997=0 ORDER BY frame_id LIMIT 32",
    )
    .fetch_all(&source.pool)
    .await?;
    for id in ids {
        let left = source.get_frame_elements(id, None).await?;
        let right = target.get_frame_elements(id, None).await?;
        if serde_json::to_vec(&left).map_err(storage_error)?
            != serde_json::to_vec(&right).map_err(storage_error)?
        {
            return Err(storage_error("complete element tree parity failed"));
        }
        rows += left.len();
        queries += 1;
    }
    Ok((queries, rows, timings))
}

async fn element_batches(
    payloads: &[super::FramePayload],
) -> Result<Vec<ElementBatchReport>, sqlx::Error> {
    let mut reports = Vec::new();
    for hybrid in [false, true] {
        let root = tempfile::tempdir()?;
        let db = if hybrid {
            DatabaseManager::new_hybrid(root.path(), Default::default(), Default::default()).await?
        } else {
            DatabaseManager::new_with_storage(
                root.path().join("db.sqlite").to_str().unwrap(),
                Default::default(),
                None,
                false,
                false,
            )
            .await?
        };
        let result = async {
            let mut times = Vec::new();
            for (index, payload) in payloads.iter().take(32).enumerate() {
                let id = index as i64 + 1;
                db.execute_raw_sql_write(&format!(
                    "INSERT INTO frames(id,timestamp) VALUES({id},'2026-09-11T12:00:00Z')"
                ))
                .await?;
                let start = Instant::now();
                let mut tx = db.begin_immediate_with_retry().await?;
                if let Some(json) = &payload.text_json {
                    DatabaseManager::insert_ocr_elements(tx.conn(), id, json).await;
                }
                if let Some(json) = &payload.accessibility_tree_json {
                    DatabaseManager::insert_accessibility_elements(tx.conn(), id, json).await;
                }
                tx.commit().await?;
                times.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM elements")
                .fetch_one(&db.pool)
                .await?;
            Ok::<_, sqlx::Error>(ElementBatchReport {
                mode: if hybrid {
                    "hybrid-parquet-v1"
                } else {
                    "sqlite"
                },
                batches: times.len(),
                elements: count,
                median_ms: percentile(&mut times, 0.5),
                p95_ms: percentile(&mut times, 0.95),
            })
        }
        .await;
        db.close().await;
        reports.push(result?);
    }
    if reports[0].elements != reports[1].elements {
        return Err(storage_error("element replay row count differs"));
    }
    Ok(reports)
}

#[derive(Serialize)]
pub struct FrameTransactionReport {
    pub mode: &'static str,
    pub frames: usize,
    pub payload_bytes: usize,
    pub commit_median_ms: f64,
    pub commit_p95_ms: f64,
    pub seal_ms: f64,
}

fn percentile(values: &mut [f64], p: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    values[((values.len() - 1) as f64 * p).round() as usize]
}

/// Replay real payload sizes through the serialized frame transaction. Each
/// mode gets a private temporary database and identical synthetic metadata.
/// This measures DB commits and sealing, excluding capture/detector work.
async fn frame_transactions(
    payloads: &[super::FramePayload],
) -> Result<Vec<FrameTransactionReport>, sqlx::Error> {
    let mut reports = Vec::new();
    for hybrid in [false, true] {
        let root = tempfile::tempdir()?;
        let db = if hybrid {
            DatabaseManager::new_hybrid(root.path(), Default::default(), Default::default()).await?
        } else {
            DatabaseManager::new_with_storage(
                root.path().join("db.sqlite").to_str().unwrap(),
                Default::default(),
                None,
                false,
                false,
            )
            .await?
        };
        let result = async {
            let mut commits = Vec::new();
            for (id, p) in payloads.iter().enumerate() {
                let start = Instant::now();
                let mut tx = db.begin_immediate_with_retry().await?;
                sqlx::query("INSERT INTO frames(id,timestamp,offset_index,device_name,app_name,full_text,accessibility_text,accessibility_tree_json,text_json) VALUES(?,'2026-09-11T12:00:00Z',0,'storage-validation','storage-validation',?,?,?,?)")
                    .bind(id as i64 + 1).bind(&p.full_text).bind(&p.accessibility_text).bind(&p.accessibility_tree_json).bind(&p.text_json)
                    .execute(&mut **tx.conn()).await?;
                tx.commit().await?;
                commits.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            let start = Instant::now();
            if hybrid {while db.seal_frame_payloads().await? != 0 {}}
            let seal_ms = if hybrid {start.elapsed().as_secs_f64() * 1000.0} else {0.0};
            let ids: Vec<i64> = (1..=payloads.len() as i64).collect();
            let decoded = db.frame_payloads(&ids, Projection::All).await?;
            for (id, expected) in payloads.iter().enumerate() {
                let actual = decoded.get(&(id as i64+1)).ok_or_else(||storage_error("replayed frame missing"))?;
                if actual.full_text != expected.full_text || actual.accessibility_text != expected.accessibility_text
                    || actual.accessibility_tree_json != expected.accessibility_tree_json || actual.text_json != expected.text_json {
                    return Err(storage_error("replayed frame payload differs"));
                }
            }
            Ok(FrameTransactionReport {mode:if hybrid {"hybrid-parquet-v1"} else {"sqlite"}, frames:payloads.len(), payload_bytes:payloads.iter().map(super::FramePayload::bytes).sum(), commit_median_ms:percentile(&mut commits,0.5),commit_p95_ms:percentile(&mut commits,0.95),seal_ms})
        }.await;
        db.close().await;
        reports.push(result?);
    }
    Ok(reports)
}

pub async fn compare(legacy: &Path, root: &Path) -> Result<ComparisonReport, sqlx::Error> {
    let root = root.canonicalize()?;
    let descriptor = StorageDescriptor::read(&root)?
        .ok_or_else(|| storage_error("comparison requires hybrid root"))?;
    let source = DatabaseManager::new_with_storage(
        legacy
            .to_str()
            .ok_or_else(|| storage_error("non-UTF8 source path"))?,
        Default::default(),
        None,
        false,
        false,
    )
    .await?;
    let target_result = DatabaseManager::new_with_storage(
        root.join(&descriptor.index).to_str().unwrap(),
        Default::default(),
        Some(HybridStorage::new(root.clone(), descriptor)?),
        false,
        false,
    )
    .await;
    let target = match target_result {
        Ok(db) => db,
        Err(error) => {
            source.close().await;
            return Err(error);
        }
    };
    let result = compare_open(&source, &target).await;
    target.close().await;
    source.close().await;
    result
}

async fn compare_open(
    source: &DatabaseManager,
    target: &DatabaseManager,
) -> Result<ComparisonReport, sqlx::Error> {
    let source_receipts = super::lifecycle::table_receipts(source, None).await?;
    if source_receipts != super::lifecycle::table_receipts(target, Some(&source_receipts)).await? {
        return Err(storage_error("comparison logical-record parity failed"));
    }
    let mut report = compare_queries(source, target, true).await?;
    report.logical_tables = source_receipts.len();
    report.logical_rows = source_receipts.iter().map(|r| r.rows).sum();
    Ok(report)
}

pub(super) async fn verify_typed_parity(
    source: &DatabaseManager,
    target: &DatabaseManager,
) -> Result<(), sqlx::Error> {
    compare_queries(source, target, false).await.map(|_| ())
}

async fn compare_queries(
    source: &DatabaseManager,
    target: &DatabaseManager,
    benchmark: bool,
) -> Result<ComparisonReport, sqlx::Error> {
    let texts:Vec<String>=sqlx::query_scalar("SELECT full_text FROM frames WHERE full_text IS NOT NULL AND id % 997 = 0 ORDER BY id LIMIT 24").fetch_all(&source.pool).await?;
    let mut queries = vec![
        String::new(),
        "meeting".into(),
        "screenpipe".into(),
        "東京".into(),
    ];
    for text in texts {
        if let Some(term) = text
            .split_whitespace()
            .find(|t| t.chars().all(char::is_alphabetic) && t.chars().count() > 3)
        {
            queries.push(term.to_owned());
        }
    }
    queries.sort();
    queries.dedup();
    let apps: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT app_name FROM frames WHERE app_name IS NOT NULL ORDER BY app_name LIMIT 3",
    )
    .fetch_all(&source.pool)
    .await?;
    let mut a = Vec::new();
    let mut b = Vec::new();
    let mut result_rows = 0;
    for (query_index, query) in queries.iter().enumerate() {
        for variant in 0..12 {
            let kind = match variant % 3 {
                0 => ContentType::OCR,
                1 => ContentType::Accessibility,
                _ => ContentType::All,
            };
            let offset = if variant >= 6 { 17 } else { 0 };
            let order = if variant % 2 == 0 {
                Order::Ascending
            } else {
                Order::Descending
            };
            let app = if variant >= 9 {
                apps.first().map(String::as_str)
            } else {
                None
            };
            let length = if variant >= 9 { Some(30) } else { None };
            let mut responses = Vec::new();
            for (db, timings) in [(source, &mut a), (target, &mut b)] {
                let start = Instant::now();
                let rows = db
                    .search_with_tags_ordered_lightweight(
                        query,
                        kind.clone(),
                        50,
                        offset,
                        None,
                        None,
                        app,
                        None,
                        length,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        false,
                        &[],
                        order,
                    )
                    .await?;
                let count = db
                    .count_search_results(
                        query,
                        kind.clone(),
                        None,
                        None,
                        app,
                        None,
                        length,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    )
                    .await?;
                let serialized = serde_json::to_vec(&(rows, count)).map_err(storage_error)?;
                timings.push(start.elapsed().as_secs_f64() * 1000.0);
                responses.push(serialized);
            }
            if responses[0] != responses[1] {
                return Err(storage_error(format!(
                    "typed search parity failed at workload {query_index}/{variant}"
                )));
            }
            result_rows += serde_json::from_slice::<serde_json::Value>(&responses[0])
                .map_err(storage_error)?[0]
                .as_array()
                .map_or(0, Vec::len);
        }
        tracing::info!(completed_queries = a.len(), "typed storage comparison");
    }
    let ids: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM frames WHERE id % 499 = 0 ORDER BY id LIMIT 300")
            .fetch_all(&source.pool)
            .await?;
    for batch in ids.chunks(64) {
        let mut left = source.frame_payloads(batch, Projection::All).await?;
        let mut right = target.frame_payloads(batch, Projection::All).await?;
        for row in left.values_mut().chain(right.values_mut()) {
            row.generation = 0;
        }
        if left != right {
            return Err(storage_error("typed detail parity failed"));
        }
    }
    let payloads = write_payloads(source, &ids[..ids.len().min(128)]).await?;
    let frame_transactions = if payloads.is_empty() || !benchmark {
        Vec::new()
    } else {
        frame_transactions(&payloads).await?
    };
    let (bulk_queries, bulk_result_rows, element_search) =
        bulk_queries(source, target, &queries).await?;
    let element_batches = if benchmark && !payloads.is_empty() {
        element_batches(&payloads).await?
    } else {
        Vec::new()
    };
    Ok(ComparisonReport {logical_tables:0,logical_rows:0,queries:a.len(),result_rows,detail_records:ids.len(),sqlite_median_ms:percentile(&mut a,0.5),sqlite_p95_ms:percentile(&mut a,0.95),hybrid_median_ms:percentile(&mut b,0.5),hybrid_p95_ms:percentile(&mut b,0.95),cache_condition:"one fresh manager per mode; subsequent reads share SQLite and OS caches; no cold-disk claim",frame_transactions,bulk_queries,bulk_result_rows,element_batches,element_search})
}

async fn write_payloads(
    source: &DatabaseManager,
    ids: &[i64],
) -> Result<Vec<super::FramePayload>, sqlx::Error> {
    let payloads: Vec<_> = source
        .frame_payloads(ids, Projection::All)
        .await?
        .into_values()
        .collect();
    Ok(payloads)
}

#[tokio::test]
#[ignore = "requires SCREENPIPE_STORAGE_TEST_CLONE pointing to a private SQLite clone"]
async fn production_clone_write_replay() {
    let path = std::env::var("SCREENPIPE_STORAGE_TEST_CLONE").unwrap();
    let source = DatabaseManager::new_with_storage(&path, Default::default(), None, false, false)
        .await
        .unwrap();
    let ids = sqlx::query_scalar("SELECT id FROM frames WHERE id % 499 = 0 ORDER BY id LIMIT 128")
        .fetch_all(&source.pool)
        .await
        .unwrap();
    let payloads = write_payloads(&source, &ids).await.unwrap();
    source.close().await;
    assert!(!payloads.is_empty());
    let frames = frame_transactions(&payloads).await.unwrap();
    let elements = element_batches(&payloads).await.unwrap();
    assert!(elements[0].elements > 0);
    println!(
        "{}",
        serde_json::json!({"frame_transactions": frames, "element_batches": elements})
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires SCREENPIPE_STORAGE_TEST_CLONE and SCREENPIPE_STORAGE_TEST_HYBRID private fixtures"]
async fn production_clone_concurrent_reads() {
    use futures::{stream, StreamExt, TryStreamExt};
    use sha2::{Digest, Sha256};
    use std::sync::Arc;

    async fn query(db: &DatabaseManager, case: usize, frame: i64) -> Result<Vec<u8>, sqlx::Error> {
        match case % 4 {
            0 => {
                let mut rows = db.frame_payloads(&[frame], Projection::All).await?;
                for row in rows.values_mut() {
                    row.generation = 0;
                }
                serde_json::to_vec(&rows).map_err(storage_error)
            }
            1 => serde_json::to_vec(&db.get_frame_elements(frame, None).await?)
                .map_err(storage_error),
            2 => serde_json::to_vec(
                &db.search_elements(
                    "meeting",
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(case % 8 == 2),
                    25,
                    7,
                )
                .await?,
            )
            .map_err(storage_error),
            _ => serde_json::to_vec(
                &db.query_raw_sql(
                    "SELECT id,transcription FROM audio_transcriptions ORDER BY id DESC LIMIT 64",
                )
                .await?,
            )
            .map_err(storage_error),
        }
    }
    let source_path = std::env::var("SCREENPIPE_STORAGE_TEST_CLONE").unwrap();
    let hybrid_path = std::env::var("SCREENPIPE_STORAGE_TEST_HYBRID").unwrap();
    let source = Arc::new(
        DatabaseManager::new_with_storage(&source_path, Default::default(), None, false, false)
            .await
            .unwrap(),
    );
    let ids: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM frames WHERE id % 499=0 ORDER BY id LIMIT 16")
            .fetch_all(&source.pool)
            .await
            .unwrap();
    assert_eq!(ids.len(), 16);
    let mut expected = Vec::new();
    for (case, &id) in ids.iter().enumerate() {
        expected.push(Sha256::digest(query(&source, case, id).await.unwrap()).to_vec());
    }
    source.close().await;
    for concurrency in [1, 4, 8] {
        for (mode, path) in [("sqlite", &source_path), ("hybrid", &hybrid_path)] {
            // Reopen for each measurement. SQLite/OS caches may remain warm;
            // the in-process decoded payload cache starts empty.
            let resolved = super::resolve_database_path(Path::new(path)).unwrap();
            let storage = HybridStorage::for_index(&resolved).unwrap();
            assert_eq!(storage.is_some(), mode == "hybrid");
            let db = Arc::new(
                DatabaseManager::new_with_storage(
                    resolved.to_str().unwrap(),
                    Default::default(),
                    storage,
                    false,
                    false,
                )
                .await
                .unwrap(),
            );
            let started = Instant::now();
            let results: Vec<_> =
                stream::iter(ids.iter().copied().enumerate().map(|(case, id)| {
                    let db = db.clone();
                    async move {
                        let start = Instant::now();
                        let bytes = query(&db, case, id).await?;
                        Ok::<_, sqlx::Error>((
                            case,
                            start.elapsed().as_secs_f64() * 1000.0,
                            Sha256::digest(bytes).to_vec(),
                        ))
                    }
                }))
                .buffer_unordered(concurrency)
                .try_collect()
                .await
                .unwrap();
            let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
            let mut timings = Vec::new();
            for (case, ms, hash) in results {
                assert_eq!(hash, expected[case], "complete query parity at case {case}");
                timings.push(ms);
            }
            db.close().await;
            println!(
                "{}",
                serde_json::json!({"mode": mode, "concurrency": concurrency, "queries": ids.len(), "elapsed_ms": elapsed_ms, "median_ms": percentile(&mut timings,0.5), "p95_ms": percentile(&mut timings,0.95), "parity":true,"cache":"fresh manager, shared SQLite/OS caches"})
            );
        }
    }
}
