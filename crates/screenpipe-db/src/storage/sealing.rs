// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use super::{
    checked_path, codec, storage_error, sync_directory, FramePayload, HybridStorage, Projection,
};
use crate::DatabaseManager;
use screenpipe_sqlite_coordinator::SqliteWritePool;
use sqlx::{Row, SqlitePool};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

impl DatabaseManager {
    pub(crate) async fn strip_archived_frame_details(
        &self,
        start: chrono::DateTime<chrono::Utc>,
        end: chrono::DateTime<chrono::Utc>,
    ) -> Result<u64, sqlx::Error> {
        if self.storage.is_none() {
            return Ok(0);
        }
        let mut count = 0;
        loop {
            let rows=sqlx::query("SELECT f.id,p.policy,p.completed_surfaces FROM frames f JOIN frame_payloads p ON p.frame_id=f.id WHERE f.timestamp BETWEEN ? AND ? AND f.payload_detail_present=1 ORDER BY f.id LIMIT 128")
                .bind(start).bind(end).fetch_all(&self.pool).await?;
            if rows.is_empty() {
                break;
            }
            let ids: Vec<i64> = rows.iter().map(|r| r.get("id")).collect();
            let mut payloads = self.frame_payloads(&ids, Projection::All).await?;
            for row in rows {
                let mut payload = payloads
                    .remove(&row.get("id"))
                    .ok_or_else(|| storage_error("retention frame disappeared; retry"))?;
                payload.accessibility_tree_json = None;
                payload.text_json = None;
                if !self
                    .replace_frame_payload(
                        &payload,
                        row.get("policy"),
                        row.get::<i64, _>("completed_surfaces") as u8 | 12,
                        None,
                        None,
                    )
                    .await?
                {
                    return Err(storage_error("retention generation changed; retry"));
                }
                count += 1;
            }
        }
        Ok(count)
    }

    pub async fn set_frame_privacy_policy(
        &self,
        policy: &super::PrivacyPolicy,
    ) -> Result<(), sqlx::Error> {
        let Some(storage) = &self.storage else {
            return Ok(());
        };
        if policy.required_surfaces & !63 != 0
            || (policy.required_surfaces != 0 && policy.identity.is_empty())
        {
            return Err(storage_error("invalid privacy policy"));
        }
        let current: (String, i64) =
            sqlx::query_as("SELECT policy,required_surfaces FROM storage_metadata")
                .fetch_one(&self.pool)
                .await?;
        if current == (policy.identity.clone(), policy.required_surfaces as i64) {
            storage
                .privacy_ready
                .store(true, std::sync::atomic::Ordering::Release);
            return Ok(());
        }
        let mut tx = self.begin_immediate_with_retry().await?;
        sqlx::query("UPDATE storage_metadata SET policy=?,required_surfaces=?,revision=revision+1")
            .bind(&policy.identity)
            .bind(policy.required_surfaces as i64)
            .execute(&mut **tx.conn())
            .await?;
        sqlx::query("UPDATE frame_payloads SET policy=?,completed_surfaces=0,attempts=0,retry_at=NULL,last_error=NULL").bind(&policy.identity).execute(&mut **tx.conn()).await?;
        tx.commit().await?;
        storage
            .privacy_ready
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(())
    }

    pub async fn seal_frame_payloads(&self) -> Result<usize, sqlx::Error> {
        let Some(storage) = &self.storage else {
            return Ok(0);
        };
        storage
            .seal_once(&self.pool, &self.coordinated_writer())
            .await
    }

    pub async fn reclaim_frame_payloads(&self) -> Result<usize, sqlx::Error> {
        let Some(storage) = &self.storage else {
            return Ok(0);
        };
        storage
            .reclaim_once(&self.pool, &self.coordinated_writer())
            .await
    }

    /// Generation compare-and-swap for late OCR, privacy and detail retention.
    /// Detection and payload reading complete before this acquires the writer.
    pub async fn replace_frame_payload(
        &self,
        payload: &FramePayload,
        policy: &str,
        completed_surfaces: u8,
        window_name: Option<&str>,
        browser_url: Option<&str>,
    ) -> Result<bool, sqlx::Error> {
        let Some(storage) = &self.storage else {
            return Err(storage_error("replacement requires hybrid storage"));
        };
        if payload.bytes() > storage.descriptor.budget.record_bytes {
            return Err(storage_error("record budget exceeded"));
        }
        let mut tx = self.begin_immediate_with_retry().await?;
        let row = sqlx::query("SELECT p.generation,p.state,p.file_id,p.bytes,m.policy,m.staging_bytes,m.staging_limit FROM frame_payloads p CROSS JOIN storage_metadata m WHERE p.frame_id=?")
            .bind(payload.id).fetch_optional(&mut **tx.conn()).await?;
        let Some(row) = row else {
            tx.rollback().await?;
            return Ok(false);
        };
        if row.get::<i64, _>("generation") != payload.generation
            || row.get::<&str, _>("policy") != policy
        {
            tx.rollback().await?;
            return Ok(false);
        }
        let prior_bytes = if row.get::<&str, _>("state") == "staged" {
            row.get::<i64, _>("bytes")
        } else {
            0
        };
        let next_bytes = row.get::<i64, _>("staging_bytes") - prior_bytes + payload.bytes() as i64;
        if next_bytes > row.get::<i64, _>("staging_limit") {
            return Err(storage_error("staging budget reached"));
        }
        let old_file: Option<String> = row.try_get("file_id")?;
        sqlx::query(
            "UPDATE storage_metadata SET maintenance=1,revision=revision+1,staging_bytes=?",
        )
        .bind(next_bytes)
        .execute(&mut **tx.conn())
        .await?;
        sqlx::query("UPDATE frames SET full_text=?,accessibility_text=?,accessibility_tree_json=?,text_json=?,window_name=COALESCE(?,window_name),browser_url=COALESCE(?,browser_url),payload_full_text_length=length(COALESCE(?,'')),payload_accessibility_length=length(COALESCE(?,'')),payload_full_text_present=? IS NOT NULL,payload_accessibility_present=? IS NOT NULL,payload_detail_present=? IS NOT NULL OR ? IS NOT NULL WHERE id=?")
            .bind(&payload.full_text).bind(&payload.accessibility_text).bind(&payload.accessibility_tree_json).bind(&payload.text_json)
            .bind(window_name).bind(browser_url).bind(&payload.full_text).bind(&payload.accessibility_text)
            .bind(&payload.full_text).bind(&payload.accessibility_text).bind(&payload.accessibility_tree_json).bind(&payload.text_json).bind(payload.id)
            .execute(&mut **tx.conn()).await?;
        sqlx::query("UPDATE frame_payloads SET generation=generation+1,state='staged',file_id=NULL,bytes=?,policy=?,completed_surfaces=?,attempts=0,retry_at=NULL,last_error=NULL WHERE frame_id=?")
            .bind(payload.bytes() as i64).bind(policy).bind(completed_surfaces as i64).bind(payload.id).execute(&mut **tx.conn()).await?;
        sqlx::query("UPDATE frames SET full_text_redacted_at=CASE WHEN (?1&1)!=0 THEN strftime('%s','now') END,accessibility_redacted_at=CASE WHEN (?1&2)!=0 THEN strftime('%s','now') END,accessibility_tree_redacted_at=CASE WHEN (?1&4)!=0 THEN strftime('%s','now') END,text_json_redacted_at=CASE WHEN (?1&8)!=0 THEN strftime('%s','now') END,window_name_redacted_at=CASE WHEN (?1&16)!=0 THEN strftime('%s','now') END,browser_url_redacted_at=CASE WHEN (?1&32)!=0 THEN strftime('%s','now') END WHERE id=?2")
            .bind(completed_surfaces as i64).bind(payload.id).execute(&mut **tx.conn()).await?;
        sqlx::query("DELETE FROM frames_fts WHERE rowid=?")
            .bind(payload.id)
            .execute(&mut **tx.conn())
            .await?;
        sqlx::query("INSERT INTO frames_fts(rowid,full_text,app_name,window_name,browser_url) SELECT id,full_text,COALESCE(app_name,''),COALESCE(window_name,''),COALESCE(browser_url,'') FROM frames WHERE id=? AND full_text IS NOT NULL AND full_text != ''")
            .bind(payload.id).execute(&mut **tx.conn()).await?;
        sqlx::query("UPDATE payload_files SET state='dirty' WHERE id=?")
            .bind(old_file)
            .execute(&mut **tx.conn())
            .await?;
        sqlx::query("UPDATE storage_metadata SET maintenance=0")
            .execute(&mut **tx.conn())
            .await?;
        tx.commit().await?;
        Ok(true)
    }
}

impl HybridStorage {
    pub fn spawn_maintenance(
        self: &Arc<Self>,
        pool: SqlitePool,
        writer: SqliteWritePool,
        shutdown: tokio_util::sync::CancellationToken,
    ) {
        let storage = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                }
                // An in-flight file job retains ownership until it either
                // publishes or leaves its durable reservation for restart.
                if storage
                    .privacy_ready
                    .load(std::sync::atomic::Ordering::Acquire)
                {
                    if let Err(error) = storage.seal_once(&pool, &writer).await {
                        tracing::warn!(%error,"frame storage sealing deferred");
                    }
                }
                if shutdown.is_cancelled() {
                    break;
                }
                if let Err(error) = storage.reclaim_once(&pool, &writer).await {
                    tracing::warn!(%error,"frame storage reclamation deferred");
                }
            }
        });
    }

    pub async fn seal_once(
        self: &Arc<Self>,
        pool: &SqlitePool,
        writer: &SqliteWritePool,
    ) -> Result<usize, sqlx::Error> {
        let _job = self.file_job.lock().await;
        let candidates = sqlx::query("SELECT p.frame_id,p.bytes FROM frame_payloads p CROSS JOIN storage_metadata m WHERE p.state='staged' AND p.policy=m.policy AND (p.completed_surfaces & m.required_surfaces)=m.required_surfaces ORDER BY p.frame_id LIMIT ?")
            .bind(self.descriptor.budget.file_rows as i64).fetch_all(pool).await?;
        let mut ids = Vec::new();
        let mut bytes = 0;
        for row in candidates {
            let size = row.get::<i64, _>("bytes") as usize;
            if size > self.descriptor.budget.record_bytes {
                return Err(storage_error("record exceeds sealing budget"));
            }
            if !ids.is_empty() && bytes + size > self.descriptor.budget.file_bytes {
                break;
            }
            ids.push(row.get("frame_id"));
            bytes += size;
        }
        if ids.is_empty() {
            return self.seal_bulk(pool, writer).await;
        }
        let payloads: Vec<_> = self
            .read(pool, &ids, Projection::All)
            .await?
            .into_values()
            .collect();
        self.publish(pool, writer, payloads, None).await
    }

    async fn publish(
        self: &Arc<Self>,
        pool: &SqlitePool,
        writer: &SqliteWritePool,
        rows: Vec<FramePayload>,
        replaces: Option<&str>,
    ) -> Result<usize, sqlx::Error> {
        if rows.is_empty() {
            return Ok(0);
        }
        let required: (String, i64) =
            sqlx::query_as("SELECT policy,required_surfaces FROM storage_metadata")
                .fetch_one(pool)
                .await?;
        if fs2::available_space(&self.root)?
            < self.descriptor.budget.disk_reserve_bytes
                + self.descriptor.budget.file_bytes as u64 * 2
        {
            return Err(storage_error("temporary disk reserve reached"));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let base = self
            .descriptor
            .payloads
            .join(codec::SCHEMA_VERSION.to_string())
            .join(chrono::Utc::now().format("%Y-%m-%d").to_string())
            .join(&id);
        let search = base.join("search.parquet");
        let detail = base.join("detail.parquet");
        {
            let permit = writer.lock().await?;
            sqlx::query("INSERT INTO payload_files(id,search_path,detail_path,schema_version,state,row_count) VALUES(?,?,?,?,'encoding',?)")
                .bind(&id).bind(search.to_string_lossy().as_ref()).bind(detail.to_string_lossy().as_ref()).bind(codec::SCHEMA_VERSION as i64).bind(rows.len() as i64).execute(permit.pool()).await?;
        }
        super::faults::checkpoint("seal_reserved");
        let directory = checked_path(&self.root, &base)?;
        std::fs::create_dir_all(&directory)?;
        let search_file = checked_path(&self.root, &search)?;
        let detail_file = checked_path(&self.root, &detail)?;
        let encode_rows = rows.clone();
        let budget = self.descriptor.budget.clone();
        let decoder = Arc::clone(&self.decoder)
            .acquire_owned()
            .await
            .map_err(|_| sqlx::Error::PoolClosed)?;
        let job_lease = Arc::clone(&self.leases).read_owned().await;
        let (search_hash, detail_hash) = tokio::task::spawn_blocking(move || {
            let _job_lease = job_lease;
            let _decoder = decoder;
            let a = codec::write(&search_file, &encode_rows, Projection::Search, &budget)?;
            let b = codec::write(&detail_file, &encode_rows, Projection::Detail, &budget)?;
            let search_rows = codec::read(&search_file, Projection::Search, &a, &budget)?;
            let detail_rows = codec::read(&detail_file, Projection::Detail, &b, &budget)?;
            if search_rows.len() != encode_rows.len() || detail_rows.len() != encode_rows.len() {
                return Err(storage_error("encoded row count mismatch"));
            }
            for ((source, text), detail) in encode_rows.iter().zip(search_rows).zip(detail_rows) {
                let combined = FramePayload {
                    accessibility_tree_json: detail.accessibility_tree_json,
                    text_json: detail.text_json,
                    ..text
                };
                if &combined != source {
                    return Err(storage_error("encoded record parity mismatch"));
                }
            }
            Ok::<_, sqlx::Error>((a, b))
        })
        .await
        .map_err(storage_error)??;
        let mut ancestor = directory.as_path();
        loop {
            sync_directory(ancestor)?;
            if ancestor == self.root {
                break;
            }
            ancestor = ancestor
                .parent()
                .ok_or_else(|| storage_error("invalid payload directory"))?;
        }
        super::faults::checkpoint("seal_files_synced");
        let permit = writer.lock().await?;
        let mut tx = permit.pool().begin().await?;
        let current: (String, i64) =
            sqlx::query_as("SELECT policy,required_surfaces FROM storage_metadata")
                .fetch_one(&mut *tx)
                .await?;
        let mut valid = current == required;
        for row in &rows {
            let current: Option<(i64,String,Option<String>,String,i64)> = sqlx::query_as("SELECT generation,state,file_id,policy,completed_surfaces FROM frame_payloads WHERE frame_id=?")
                .bind(row.id).fetch_optional(&mut *tx).await?;
            valid &= current.is_some_and(|(generation, state, file, policy, completed)| {
                generation == row.generation
                    && policy == required.0
                    && completed & required.1 == required.1
                    && match replaces {
                        Some(old) => state == "sealed" && file.as_deref() == Some(old),
                        None => state == "staged",
                    }
            });
        }
        if !valid {
            sqlx::query("UPDATE payload_files SET state='retired' WHERE id=?")
                .bind(&id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            return Ok(0);
        }
        sqlx::query("UPDATE payload_files SET state='published',search_checksum=?,detail_checksum=? WHERE id=?")
            .bind(search_hash).bind(detail_hash).bind(&id).execute(&mut *tx).await?;
        sqlx::query("UPDATE storage_metadata SET maintenance=1")
            .execute(&mut *tx)
            .await?;
        for row in &rows {
            sqlx::query("UPDATE frame_payloads SET state='sealed',file_id=?,archive_writer_version=? WHERE frame_id=?")
                .bind(&id).bind(env!("CARGO_PKG_VERSION")).bind(row.id).execute(&mut *tx).await?;
            if replaces.is_none() {
                sqlx::query("UPDATE frames SET full_text=NULL,accessibility_text=NULL,accessibility_tree_json=NULL,text_json=NULL WHERE id=?")
                    .bind(row.id).execute(&mut *tx).await?;
            }
        }
        let freed = if replaces.is_none() {
            rows.iter().map(|r| r.bytes() as i64).sum()
        } else {
            0
        };
        sqlx::query("UPDATE storage_metadata SET maintenance=0,staging_bytes=staging_bytes-?")
            .bind(freed)
            .execute(&mut *tx)
            .await?;
        if let Some(old) = replaces {
            sqlx::query("UPDATE payload_files SET state='retired' WHERE id=? AND NOT EXISTS(SELECT 1 FROM frame_payloads WHERE file_id=?)")
                .bind(old).bind(old).execute(&mut *tx).await?;
        }
        super::faults::checkpoint("seal_before_commit");
        tx.commit().await?;
        super::faults::checkpoint("seal_committed");
        Ok(rows.len())
    }

    pub async fn reclaim_once(
        self: &Arc<Self>,
        pool: &SqlitePool,
        writer: &SqliteWritePool,
    ) -> Result<usize, sqlx::Error> {
        let _job = self.file_job.lock().await;
        let bulk_removed = self.reclaim_bulk(pool, writer).await?;
        let dirty: Option<String> =
            sqlx::query_scalar("SELECT id FROM payload_files WHERE state='dirty' LIMIT 1")
                .fetch_optional(pool)
                .await?;
        if let Some(id) = dirty {
            let ids: Vec<i64> = sqlx::query_scalar(
                "SELECT frame_id FROM frame_payloads WHERE file_id=? ORDER BY frame_id",
            )
            .bind(&id)
            .fetch_all(pool)
            .await?;
            if !ids.is_empty() {
                let rows = self
                    .read(pool, &ids, Projection::All)
                    .await?
                    .into_values()
                    .collect();
                self.publish(pool, writer, rows, Some(&id)).await?;
            }
        }
        {
            let permit = writer.lock().await?;
            sqlx::query("UPDATE payload_files SET state='retired' WHERE state IN ('dirty','encoding') AND NOT EXISTS(SELECT 1 FROM frame_payloads WHERE file_id=payload_files.id)")
                .execute(permit.pool()).await?;
        }
        // No waiter is queued: a reader can acquire another pin while holding
        // its existing pin. Cleanup tries again after those operations drain.
        let permit = writer.lock().await?;
        let retired = sqlx::query(
            "SELECT id,search_path,detail_path FROM payload_files WHERE state='retired' AND NOT EXISTS(SELECT 1 FROM frame_payloads WHERE file_id=payload_files.id) LIMIT 32",
        )
        .fetch_all(permit.pool())
        .await?;
        let mut removed = bulk_removed;
        {
            let Ok(_leases) = Arc::clone(&self.leases).try_write_owned() else {
                return Ok(removed);
            };
            super::faults::checkpoint("files_retired");
            for row in &retired {
                for column in ["search_path", "detail_path"] {
                    let relative: &str = row.try_get(column)?;
                    let path = self.payload_path(Path::new(relative))?;
                    match std::fs::remove_file(&path) {
                        Ok(()) => sync_directory(path.parent().unwrap())?,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(e.into()),
                    }
                }
            }
        }
        for row in retired {
            super::faults::checkpoint("files_unlinked");
            sqlx::query("DELETE FROM payload_files WHERE id=? AND state='retired'")
                .bind(row.get::<&str, _>("id"))
                .execute(permit.pool())
                .await?;
            removed += 1;
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_stale_member_retires_the_entire_file_pair() {
        let root = tempfile::tempdir().unwrap();
        let db = DatabaseManager::new_hybrid(root.path(), Default::default(), Default::default())
            .await
            .unwrap();
        db.execute_raw_sql_write("INSERT INTO frames(id,timestamp,full_text) VALUES(1,'2026-09-11','first'),(2,'2026-09-11','second')").await.unwrap();
        let rows = db
            .frame_payloads(&[1, 2], Projection::All)
            .await
            .unwrap()
            .into_values()
            .collect();
        db.execute_raw_sql_write("UPDATE frames SET full_text='current' WHERE id=1")
            .await
            .unwrap();
        let count = db
            .storage
            .as_ref()
            .unwrap()
            .publish(&db.pool, &db.coordinated_writer(), rows, None)
            .await
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM frame_payloads WHERE state='staged'"
            )
            .fetch_one(&db.pool)
            .await
            .unwrap(),
            2
        );
        assert_eq!(db.reclaim_frame_payloads().await.unwrap(), 1);
        assert_eq!(
            db.frame_payloads(&[1], Projection::Search).await.unwrap()[&1].text(),
            "current"
        );
        db.close().await;
    }
}
