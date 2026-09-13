// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use super::{storage_error, FramePayload, HybridStorage, Projection};
use crate::DatabaseManager;
use sqlx::{Row, SqlitePool};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tokio::sync::{OwnedMutexGuard, OwnedRwLockReadGuard};

impl Projection {
    fn sqlite_columns(self) -> &'static str {
        match self {
            Self::Search => {
                "f.full_text,f.accessibility_text,NULL AS accessibility_tree_json,NULL AS text_json"
            }
            Self::Detail => {
                "NULL AS full_text,NULL AS accessibility_text,f.accessibility_tree_json,f.text_json"
            }
            Self::All => "f.full_text,f.accessibility_text,f.accessibility_tree_json,f.text_json",
        }
    }
}

/// Carries the read revision through response assembly, caches and export.
pub struct StorageReadToken {
    pub revision: i64,
    revocation: Option<i64>,
    storage: Option<Arc<HybridStorage>>,
    _lease: Option<Arc<OwnedRwLockReadGuard<()>>>,
}

impl std::fmt::Debug for StorageReadToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageReadToken")
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

impl StorageReadToken {
    /// Keep this guard until the prepared response is handed to its transport.
    pub async fn admit(
        &self,
        pool: &SqlitePool,
    ) -> Result<Option<OwnedMutexGuard<()>>, sqlx::Error> {
        let Some(storage) = &self.storage else {
            return Ok(None);
        };
        let gate = Arc::clone(&storage.gate).lock_owned().await;
        if storage.closing.is_cancelled() {
            return Err(sqlx::Error::PoolClosed);
        }
        if let Some(expected) = self.revocation {
            let current: i64 =
                sqlx::query_scalar("SELECT revision FROM _storage_revocation WHERE id=1")
                    .fetch_one(pool)
                    .await?;
            if current != expected {
                return Err(storage_error(
                    "read revision changed; retry the complete query",
                ));
            }
            return Ok(Some(gate));
        }
        let current: i64 =
            sqlx::query_scalar("SELECT revision FROM storage_metadata WHERE singleton=1")
                .fetch_one(pool)
                .await?;
        if current != self.revision {
            return Err(storage_error(
                "read revision changed; retry the complete query",
            ));
        }
        Ok(Some(gate))
    }
}

impl DatabaseManager {
    pub(crate) async fn consistent_read<T, F, Fut>(&self, mut read: F) -> Result<T, sqlx::Error>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, sqlx::Error>>,
    {
        let Some(storage) = &self.storage else {
            return read().await;
        };
        if super::snapshot::in_snapshot(&self.pool) {
            return Box::pin(read()).await;
        }
        let mut last = storage_error("read revision changed; retry the complete query");
        for _ in 0..storage.descriptor.budget.read_attempts {
            let result = self
                .read_snapshot(Box::pin(read()))
                .await
                .and_then(|result| result);
            match result {
                Err(error) if is_revision_change(&error) => last = error,
                result => return result,
            }
        }
        Err(last)
    }

    pub async fn admit_storage_revision(
        &self,
        revision: i64,
    ) -> Result<Option<OwnedMutexGuard<()>>, sqlx::Error> {
        StorageReadToken {
            revision,
            revocation: None,
            storage: self.storage.clone(),
            _lease: None,
        }
        .admit(&self.pool)
        .await
    }

    pub fn storage_mode(&self) -> super::StorageMode {
        if self.storage.is_some() {
            super::StorageMode::HybridParquetV1
        } else {
            super::StorageMode::Sqlite
        }
    }

    pub async fn storage_read_token(&self) -> Result<StorageReadToken, sqlx::Error> {
        let Some(storage) = &self.storage else {
            return Ok(StorageReadToken {
                revision: 0,
                revocation: None,
                storage: None,
                _lease: None,
            });
        };
        storage.read_token(&self.pool).await
    }

    pub async fn frame_payloads(
        &self,
        ids: &[i64],
        projection: Projection,
    ) -> Result<BTreeMap<i64, FramePayload>, sqlx::Error> {
        if let Some(storage) = &self.storage {
            self.consistent_read(|| storage.read(&self.pool, ids, projection))
                .await
        } else {
            let json = serde_json::to_string(ids).map_err(storage_error)?;
            let sql = format!(
                "SELECT f.id,{} FROM frames f WHERE f.id IN (SELECT value FROM json_each(?))",
                projection.sqlite_columns()
            );
            let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
                .bind(json)
                .fetch_all(&self.pool)
                .await?;
            rows.iter()
                .map(|r| {
                    let id = r.try_get("id")?;
                    Ok((
                        id,
                        FramePayload {
                            id,
                            generation: 0,
                            full_text: r.try_get("full_text")?,
                            accessibility_text: r.try_get("accessibility_text")?,
                            accessibility_tree_json: r.try_get("accessibility_tree_json")?,
                            text_json: r.try_get("text_json")?,
                        },
                    ))
                })
                .collect()
        }
    }

    pub(crate) async fn hydrate_ocr_rows(
        &self,
        rows: &mut [crate::OCRResultRaw],
        detail: bool,
    ) -> Result<(), sqlx::Error> {
        if self.storage.is_none() {
            return Ok(());
        }
        let ids: Vec<_> = rows.iter().map(|r| r.frame_id).collect();
        let mut payloads = self
            .frame_payloads(
                &ids,
                if detail {
                    Projection::All
                } else {
                    Projection::Search
                },
            )
            .await?;
        for row in rows {
            let payload = payloads
                .remove(&row.frame_id)
                .ok_or_else(|| storage_error("selected frame disappeared"))?;
            row.ocr_text = payload.text().to_owned();
            if detail {
                row.text_json = payload.text_json.unwrap_or_default();
            }
        }
        Ok(())
    }

    pub(crate) async fn hydrate_frame_rows(
        &self,
        rows: &mut [crate::FrameRow],
    ) -> Result<(), sqlx::Error> {
        if self.storage.is_none() {
            return Ok(());
        }
        let ids: Vec<_> = rows.iter().map(|r| r.id).collect();
        let mut payloads = self.frame_payloads(&ids, Projection::All).await?;
        for row in rows {
            let payload = payloads
                .remove(&row.id)
                .ok_or_else(|| storage_error("selected frame disappeared"))?;
            row.ocr_text = payload.text().to_owned();
            row.text_json = payload.text_json.unwrap_or_default();
            row.accessibility_tree_json = payload.accessibility_tree_json;
        }
        Ok(())
    }

    pub(crate) async fn hydrate_accessibility_rows(
        &self,
        rows: &mut [crate::UiContent],
    ) -> Result<(), sqlx::Error> {
        if self.storage.is_none() {
            return Ok(());
        }
        let ids: Vec<_> = rows.iter().map(|r| r.id).collect();
        let payloads = self.frame_payloads(&ids, Projection::Search).await?;
        for row in rows {
            row.text = payloads
                .get(&row.id)
                .ok_or_else(|| storage_error("selected frame disappeared"))?
                .text()
                .to_owned();
        }
        Ok(())
    }
}

fn is_revision_change(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Protocol(message) if message.starts_with("frame storage: read revision changed;"))
}

impl HybridStorage {
    pub async fn read_token(
        self: &Arc<Self>,
        pool: &SqlitePool,
    ) -> Result<StorageReadToken, sqlx::Error> {
        // Cleanup can hold the exclusive file lease while committing through
        // the gate. Acquire the file lease before entering that gate.
        let lease = if let Some(lease) = super::snapshot::lease(pool) {
            lease
        } else {
            tokio::select! {
                biased;
                _ = self.closing.cancelled() => return Err(sqlx::Error::PoolClosed),
                lease = Arc::clone(&self.leases).read_owned() => Arc::new(lease),
            }
        };
        if self.closing.is_cancelled() {
            return Err(sqlx::Error::PoolClosed);
        }
        let (revision, revocation) = if super::snapshot::in_snapshot(pool) {
            let (revision, revocation) = super::snapshot::revision(pool).await?;
            (revision, Some(revocation))
        } else {
            let _gate = self.gate.lock().await;
            (
                sqlx::query_scalar("SELECT revision FROM storage_metadata WHERE singleton=1")
                    .fetch_one(pool)
                    .await?,
                None,
            )
        };
        Ok(StorageReadToken {
            revision,
            revocation,
            storage: Some(Arc::clone(self)),
            _lease: Some(lease),
        })
    }

    pub async fn read(
        self: &Arc<Self>,
        pool: &SqlitePool,
        ids: &[i64],
        projection: Projection,
    ) -> Result<BTreeMap<i64, FramePayload>, sqlx::Error> {
        if ids.is_empty() {
            return Ok(BTreeMap::new());
        }
        let token = self.read_token(pool).await?;
        let mut snapshot = super::snapshot::frame_snapshot(pool).await?;
        let columns = projection.sqlite_columns();
        let ids_json = serde_json::to_string(ids).map_err(storage_error)?;
        let size = match projection {
            Projection::Search => "CASE WHEN p.state='staged' THEN COALESCE(length(CAST(f.full_text AS BLOB)),0)+COALESCE(length(CAST(f.accessibility_text AS BLOB)),0) ELSE MIN(p.bytes,4*(f.payload_full_text_length+f.payload_accessibility_length)) END",
            Projection::Detail | Projection::All => "p.bytes",
        };
        let preflight=format!("SELECT COALESCE(SUM({size}),0) FROM frames f JOIN frame_payloads p ON p.frame_id=f.id WHERE f.id IN (SELECT value FROM json_each(?))");
        let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(preflight))
            .bind(&ids_json)
            .fetch_one(&mut *snapshot)
            .await?;
        if total < 0 || total as usize > self.descriptor.budget.response_bytes {
            return Err(storage_error("response payload budget exceeded"));
        }
        let sql = format!("SELECT f.id,p.generation,p.state,p.file_id,p.bytes,{columns},pf.search_path,pf.detail_path,pf.search_checksum,pf.detail_checksum FROM frames f JOIN frame_payloads p ON p.frame_id=f.id LEFT JOIN payload_files pf ON pf.id=p.file_id WHERE f.id IN (SELECT value FROM json_each(?))");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(ids_json)
            .fetch_all(&mut *snapshot)
            .await?;
        let mut out = BTreeMap::new();
        let mut files: HashMap<String, (String, String, String, String, Vec<(i64, i64)>)> =
            HashMap::new();
        for row in rows {
            let id: i64 = row.try_get("id")?;
            let generation: i64 = row.try_get("generation")?;
            if row.try_get::<&str, _>("state")? == "staged" {
                out.insert(
                    id,
                    FramePayload {
                        id,
                        generation,
                        full_text: row.try_get("full_text")?,
                        accessibility_text: row.try_get("accessibility_text")?,
                        accessibility_tree_json: row.try_get("accessibility_tree_json")?,
                        text_json: row.try_get("text_json")?,
                    },
                );
            } else {
                let key: String = row.try_get("file_id")?;
                let entry = files.entry(key).or_insert((
                    row.try_get("search_path")?,
                    row.try_get("detail_path")?,
                    row.try_get("search_checksum")?,
                    row.try_get("detail_checksum")?,
                    Vec::new(),
                ));
                entry.4.push((id, generation));
            }
        }
        snapshot.commit().await?;
        for (_, (search, detail, search_hash, detail_hash, requested)) in files {
            let search = self.payload_path(Path::new(&search))?;
            let detail = self.payload_path(Path::new(&detail))?;
            let storage = Arc::clone(self);
            // A blocking decoder shares its admitted pin. Acquiring another
            // read lease behind shutdown's queued writer would deadlock.
            let lease = token._lease.clone();
            let budget = self.descriptor.budget.clone();
            let decoded = tokio::task::spawn_blocking(move || {
                let _lease = lease;
                let selected = requested.iter().map(|(id, _)| *id).collect();
                let mut records = BTreeMap::<i64, FramePayload>::new();
                if projection != Projection::Detail {
                    for r in read_projection(
                        &storage,
                        &search,
                        Projection::Search,
                        &search_hash,
                        &budget,
                        Some(&selected),
                    )? {
                        records.insert(r.id, r);
                    }
                }
                if projection != Projection::Search {
                    for r in read_projection(
                        &storage,
                        &detail,
                        Projection::Detail,
                        &detail_hash,
                        &budget,
                        Some(&selected),
                    )? {
                        let entry = records.entry(r.id).or_insert_with(|| FramePayload {
                            id: r.id,
                            generation: r.generation,
                            ..Default::default()
                        });
                        if entry.generation != r.generation {
                            return Err(storage_error("Parquet projection generation mismatch"));
                        }
                        entry.accessibility_tree_json = r.accessibility_tree_json;
                        entry.text_json = r.text_json;
                    }
                }
                requested
                    .into_iter()
                    .map(|(id, generation)| {
                        let record = records
                            .remove(&id)
                            .ok_or_else(|| storage_error("Parquet locator missing"))?;
                        if record.generation != generation {
                            return Err(storage_error("Parquet locator generation mismatch"));
                        }
                        Ok((id, record))
                    })
                    .collect::<Result<BTreeMap<_, _>, sqlx::Error>>()
            })
            .await
            .map_err(storage_error)??;
            let bytes: usize = out
                .values()
                .chain(decoded.values())
                .map(FramePayload::bytes)
                .sum();
            if bytes > self.descriptor.budget.response_bytes {
                return Err(storage_error("response payload budget exceeded"));
            }
            out.extend(decoded);
        }
        if !super::snapshot::in_snapshot(pool) {
            let _admission = token.admit(pool).await?;
        }
        Ok(out)
    }
}

use std::path::Path;

fn read_projection(
    storage: &HybridStorage,
    path: &Path,
    projection: Projection,
    hash: &str,
    _budget: &super::StorageBudget,
    selected: Option<&std::collections::BTreeSet<i64>>,
) -> Result<Vec<FramePayload>, sqlx::Error> {
    Ok(storage
        .cached_frame_projection(
            path,
            hash,
            projection,
            selected.expect("selected frame IDs"),
        )?
        .as_ref()
        .clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn cleanup_lease_leaves_writer_admission_available() {
        let root = tempfile::tempdir().unwrap();
        let db = Arc::new(
            DatabaseManager::new_hybrid(root.path(), Default::default(), Default::default())
                .await
                .unwrap(),
        );
        let storage = db.storage.as_ref().unwrap();
        let cleanup = Arc::clone(&storage.leases).write_owned().await;
        let (started, ready) = tokio::sync::oneshot::channel();
        let reader = Arc::clone(&db);
        let waiting = tokio::spawn(async move {
            started.send(()).unwrap();
            reader.storage_read_token().await
        });
        ready.await.unwrap();
        // The reader is blocked by cleanup's file lease. Cleanup still needs
        // to commit the retired job through the ordinary writer and gate.
        let writer = db.coordinated_writer();
        let permit = tokio::time::timeout(std::time::Duration::from_secs(1), writer.lock())
            .await
            .unwrap()
            .unwrap();
        drop(permit);
        drop(cleanup);
        drop(waiting.await.unwrap().unwrap());
        db.close().await;
    }

    #[tokio::test]
    async fn shutdown_cancels_queued_decode_and_drains_admitted_pins() {
        let root = tempfile::tempdir().unwrap();
        let db = Arc::new(
            DatabaseManager::new_hybrid(root.path(), Default::default(), Default::default())
                .await
                .unwrap(),
        );
        db.execute_raw_sql_write(
            "INSERT INTO frames(id,timestamp,full_text) VALUES(1,'2026-09-11','complete')",
        )
        .await
        .unwrap();
        db.seal_frame_payloads().await.unwrap();
        let storage = db.storage.as_ref().unwrap();
        let admitted = db.storage_read_token().await.unwrap();
        let blocker = Arc::clone(&storage.decoder)
            .acquire_many_owned(storage.descriptor.budget.concurrent_decodes as u32)
            .await
            .unwrap();
        let reader = Arc::clone(&db);
        let reading =
            tokio::spawn(async move { reader.frame_payloads(&[1], Projection::All).await });
        tokio::task::yield_now().await;
        let owner = Arc::clone(&db);
        let closing = tokio::spawn(async move { owner.close().await });
        storage.closing.cancelled().await;
        assert!(db.pool.is_closed());
        assert!(db
            .execute_raw_sql_write("INSERT INTO frames(id,timestamp) VALUES(2,'2026-09-11')")
            .await
            .is_err());
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(2), reading)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert!(!closing.is_finished());
        drop(blocker);
        drop(admitted);
        tokio::time::timeout(std::time::Duration::from_secs(2), closing)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn revision_conflicts_retry_selection_and_stop_at_the_budget() {
        let root = tempfile::tempdir().unwrap();
        let db = DatabaseManager::new_hybrid(root.path(), Default::default(), Default::default())
            .await
            .unwrap();
        let calls = AtomicUsize::new(0);
        let selected=db.consistent_read(||async {
            let attempt=calls.fetch_add(1,Ordering::SeqCst);
            if attempt==0 {db.execute_raw_sql_write("INSERT INTO frames(id,timestamp,full_text) VALUES(1,'2026-09-11','new capture')").await?;}
            Ok::<_,sqlx::Error>(attempt)
        }).await.unwrap();
        assert_eq!(selected, 1);
        calls.store(0, Ordering::SeqCst);
        let error = db
            .consistent_read(|| async {
                calls.fetch_add(1, Ordering::SeqCst);
                db.execute_raw_sql_write("UPDATE frames SET window_name='changed' WHERE id=1")
                    .await?;
                Ok::<_, sqlx::Error>(())
            })
            .await
            .unwrap_err();
        assert!(is_revision_change(&error));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        db.close().await;
    }

    #[tokio::test]
    async fn cancelled_decode_admission_releases_its_file_leases() {
        let root = tempfile::tempdir().unwrap();
        let db = Arc::new(
            DatabaseManager::new_hybrid(root.path(), Default::default(), Default::default())
                .await
                .unwrap(),
        );
        db.execute_raw_sql_write(
            "INSERT INTO frames(id,timestamp,full_text) VALUES(1,'2026-09-11','complete')",
        )
        .await
        .unwrap();
        db.seal_frame_payloads().await.unwrap();
        let storage = db.storage.as_ref().unwrap();
        let blocker = Arc::clone(&storage.decoder)
            .acquire_many_owned(storage.descriptor.budget.concurrent_decodes as u32)
            .await
            .unwrap();
        let reader = Arc::clone(&db);
        let task = tokio::spawn(async move { reader.frame_payloads(&[1], Projection::All).await });
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if storage.leases.try_write().is_err() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        drop(blocker);
        db.execute_raw_sql_write("DELETE FROM frames WHERE id=1")
            .await
            .unwrap();
        assert_eq!(db.reclaim_frame_payloads().await.unwrap(), 1);
        db.close().await;
    }
}
