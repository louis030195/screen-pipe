// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

//! One offline SQLite owner converts resident payloads in the same file. Its
//! committed catalogs, not an external batch journal, own conversion progress.

use super::{bulk, schema, storage_error, HybridStorage, MigrationProgress};
use screenpipe_sqlite_coordinator::SqliteWritePool;
use sqlx::{Connection, Row, SqlitePool};
use std::{fs::OpenOptions, sync::Arc};

// Pool shutdown is asynchronous. Keep the raw fd and manager lease alive even
// if the caller cancels its future while SQLite is finishing a statement.
struct OfflineFile {
    file: Option<std::fs::File>,
    pool: SqlitePool,
    lease: Option<screenpipe_sqlite_coordinator::SqliteManagerLease>,
}

impl Drop for OfflineFile {
    fn drop(&mut self) {
        if self.file.is_none() {
            return;
        }
        let file = self.file.take();
        let lease = self.lease.take();
        let pool = self.pool.clone();
        tokio::spawn(async move {
            pool.close().await;
            drop(file);
            drop(lease);
        });
    }
}

pub(super) fn working_space(budget: &super::StorageBudget) -> u64 {
    budget
        .disk_reserve_bytes
        .saturating_add((budget.file_bytes.max(budget.record_bytes) as u64).saturating_mul(2))
}

pub(super) fn reserve(storage: &HybridStorage) -> Result<(), sqlx::Error> {
    let available = fs2::available_space(&storage.root)?;
    let required = working_space(&storage.descriptor.budget);
    if available < required {
        return Err(storage_error(format!("migration needs {required} free bytes for its next batch and reserve; {available} available. Saved progress will resume after space is available")));
    }
    Ok(())
}

pub(super) async fn convert(
    storage: Arc<HybridStorage>,
    original_allocated: u64,
    total_records: u64,
    progress: &(impl Fn(MigrationProgress) + Send + Sync),
) -> Result<(), sqlx::Error> {
    let index = storage.root.join(&storage.descriptor.index);
    let lease = screenpipe_sqlite_coordinator::acquire_sqlite_manager_lease(&index)
        .map_err(storage_error)?;
    crate::recovery::verify_database_before_reopen(&index).await?;
    crate::db::register_sqlite_extensions()?;
    // Do not close this fd while SQLite holds its process-wide Unix locks.
    let file = OpenOptions::new().read(true).write(true).open(&index)?;
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&index)
        .create_if_missing(false)
        .foreign_keys(false)
        .pragma("journal_mode", "WAL")
        .pragma("locking_mode", "EXCLUSIVE")
        .pragma("synchronous", "FULL")
        .pragma("temp_store", "FILE")
        .pragma("secure_delete", "OFF")
        .busy_timeout(std::time::Duration::from_secs(5));
    let pool = bulk::pool_options(Some(storage.clone()), false)
        .max_connections(1)
        .min_connections(1)
        .connect_lazy_with(options);
    let mut owner = OfflineFile {
        file: Some(file),
        pool: pool.clone(),
        lease: Some(lease),
    };
    let file = owner.file.as_mut().unwrap();
    let writer = SqliteWritePool::new(
        pool.clone(),
        screenpipe_sqlite_coordinator::sqlite_write_lock(&index),
    );
    let result = async {
        reserve(&storage)?;
        {
            let permit = writer.lock().await?;
            let mut conn = permit.pool().acquire().await?;
            sqlx::raw_sql("BEGIN EXCLUSIVE; COMMIT; CREATE TABLE IF NOT EXISTS _storage_conversion_steps(step TEXT PRIMARY KEY,last_id INTEGER);").execute(&mut *conn).await?;
            if !schema::converted_step(&mut conn, "original-triggers").await? {
                let mut tx=conn.begin().await?;
                sqlx::raw_sql("CREATE TABLE _storage_conversion_triggers(name TEXT PRIMARY KEY,sql TEXT NOT NULL,restore INTEGER NOT NULL DEFAULT 0); INSERT INTO _storage_conversion_triggers(name,sql) SELECT name,sql FROM sqlite_master WHERE type='trigger';").execute(&mut *tx).await?;
                schema::finish_step(&mut tx,"original-triggers").await?;
                tx.commit().await?;
            }
            schema::bootstrap_in_place(&mut conn, &storage.descriptor).await?;
            super::read_schema::upgrade(&mut conn, &storage).await?;
            if !schema::converted_step(&mut conn, "suspend-original-triggers").await? {
                let mut tx=conn.begin().await?;
                let names: Vec<String> = sqlx::query_scalar("SELECT t.name FROM _storage_conversion_triggers t JOIN sqlite_master m ON m.name=t.name AND m.type='trigger' AND m.sql=t.sql").fetch_all(&mut *tx).await?;
                for name in names {
                    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP TRIGGER \"{}\"",name.replace('"',"\"\"")))).execute(&mut *tx).await?;
                    sqlx::query("UPDATE _storage_conversion_triggers SET restore=1 WHERE name=?").bind(name).execute(&mut *tx).await?;
                }
                schema::finish_step(&mut tx,"suspend-original-triggers").await?;
                tx.commit().await?;
            }
            if schema::converted_step(&mut conn,"conversion-complete").await? { return Ok(()); }
            schema::construction_checkpoint(&mut conn).await?;
        }
        // Encoding reservations left by an interrupted attempt are handled by
        // the existing catalog reclamation; published files are never discarded.
        storage.reclaim_once(&pool, &writer).await?;
        storage.reclaim_bulk(&pool, &writer).await?;
        reclaim(&storage, &writer, file).await?;
        report(&storage, &pool, original_allocated, total_records, progress).await?;
        loop {
            reserve(&storage)?;
            if storage.seal_once(&pool, &writer).await? != 0 {
                reclaim(&storage, &writer, file).await?;
                report(&storage, &pool, original_allocated, total_records, progress).await?;
                continue;
            }
            let last: Option<i64> = sqlx::query_scalar("SELECT max(frame_id) FROM frame_payloads").fetch_one(&pool).await?;
            let columns = super::import::columns(&pool, "frames").await?;
            let columns: Vec<_> = columns.into_iter().filter(|c| !c.starts_with("payload_")).collect();
            let rows = super::import::batch(&pool, "frames", &columns, last, &storage.descriptor.budget).await?;
            if rows.is_empty() { break; }
            let first = rows.first().unwrap().get::<i64,_>(0);
            let last = rows.last().unwrap().get::<i64,_>(0);
            let permit = writer.lock().await?;
            let mut conn = permit.pool().acquire().await?;
            let mut tx = conn.begin().await?;
            schema::stage_frames(&mut tx, first, last).await?;
            tx.commit().await?;
            schema::construction_checkpoint(&mut conn).await?;
            super::faults::checkpoint("migration_batch_staged");
        }
        let source_exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='_bulk_elements_source')").fetch_one(&pool).await?;
        if source_exists {
            let columns = super::import::columns(&pool, "_bulk_elements_source").await?;
            loop {
                reserve(&storage)?;
                if bulk::elements::seal(&storage, &pool, &writer).await? != 0 {
                    reclaim(&storage, &writer, file).await?;
                    report(&storage, &pool, original_allocated, total_records, progress).await?;
                    continue;
                }
                let rows = super::import::batch(&pool, "_bulk_elements_source", &columns, None, &storage.descriptor.budget).await?;
                if rows.is_empty() { break; }
                let first = rows.first().unwrap().get::<i64,_>(0);
                let last = rows.last().unwrap().get::<i64,_>(0);
                let permit = writer.lock().await?;
                let mut conn = permit.pool().acquire().await?;
                let mut tx = conn.begin().await?;
                super::import::insert(&mut tx, "elements", &columns, &rows, true).await?;
                sqlx::query("DELETE FROM _bulk_elements_source WHERE id BETWEEN ? AND ?").bind(first).bind(last).execute(&mut *tx).await?;
                tx.commit().await?;
                schema::construction_checkpoint(&mut conn).await?;
                super::faults::checkpoint("migration_batch_staged");
                drop(conn);
                drop(permit);
                reclaim(&storage, &writer, file).await?;
            }
            let permit = writer.lock().await?;
            sqlx::query("DROP TABLE _bulk_elements_source").execute(permit.pool()).await?;
        }
        for table in bulk::TABLES.iter().filter(|t| t.name != "elements") {
            loop {
                reserve(&storage)?;
                let rows = storage.select_bulk(&pool, table, None).await?;
                if rows.is_empty() { break; }
                if !table.fts.is_empty() {
                    let ids = serde_json::to_string(&rows.iter().map(|r| r.id).collect::<Vec<_>>()).map_err(storage_error)?;
                    let permit = writer.lock().await?;
                    let mut conn = permit.pool().acquire().await?;
                    let mut tx = conn.begin().await?;
                    for sql in [format!("DELETE FROM {t}_fts WHERE rowid IN (SELECT value FROM json_each(?))",t=table.name), format!("INSERT INTO {t}_fts(rowid,{columns}) SELECT id,{columns} FROM {view} WHERE id IN (SELECT value FROM json_each(?)) AND ({condition})",t=table.name,columns=table.fts_columns(),view=table.view(),condition=table.fts_condition)] {
                        sqlx::query(sqlx::AssertSqlSafe(sql)).bind(&ids).execute(&mut *tx).await?;
                    }
                    tx.commit().await?;
                }
                storage.publish_bulk(&pool, &writer, table, rows, None).await?;
                reclaim(&storage, &writer, file).await?;
                report(&storage, &pool, original_allocated, total_records, progress).await?;
            }
            backfill_resident_fts(&storage, &pool, &writer, table, file).await?;
        }
        {
            let permit = writer.lock().await?;
            let mut conn = permit.pool().acquire().await?;
            bulk::finish_indexes(&mut conn).await?;
            let mut tx=conn.begin().await?;
            let original: Vec<String> = sqlx::query_scalar("SELECT sql FROM _storage_conversion_triggers WHERE restore=1").fetch_all(&mut *tx).await?;
            for sql in original { sqlx::raw_sql(sqlx::AssertSqlSafe(sql)).execute(&mut *tx).await?; }
            schema::finish_step(&mut tx,"conversion-complete").await?;
            tx.commit().await?;
        }
        reclaim(&storage, &writer, file).await?;
        progress(MigrationProgress { message: "conversion complete", completed_records: Some(total_records), total_records: Some(total_records),
            bytes_saved: Some(original_allocated.saturating_sub(super::reclaim::footprint(&storage.root)?)), available_bytes: Some(fs2::available_space(&storage.root)?) });
        Ok(())
    }.await;
    if let Err(ref error) = result {
        if let Some(code) = crate::sqlite_error::sqlite_hard_fault_code(error) {
            screenpipe_sqlite_coordinator::latch_sqlite_hard_fault(&index, code);
        }
    }
    pool.close().await;
    drop(owner.file.take());
    drop(owner.lease.take());
    result
}

async fn reclaim(
    storage: &HybridStorage,
    writer: &SqliteWritePool,
    file: &mut std::fs::File,
) -> Result<(), sqlx::Error> {
    let permit = writer.lock().await?;
    let mut conn = permit.pool().acquire().await?;
    super::reclaim::free_leaves(&mut conn, file, storage.descriptor.budget.decode_bytes).await?;
    super::faults::checkpoint("migration_batch_sealed");
    Ok(())
}

// Privacy-ineligible records remain resident. Their search entries still need
// the same bounded, committed backfill as records that were sealed above.
async fn backfill_resident_fts(
    storage: &HybridStorage,
    pool: &SqlitePool,
    writer: &SqliteWritePool,
    table: &bulk::Table,
    file: &mut std::fs::File,
) -> Result<(), sqlx::Error> {
    use futures::TryStreamExt;
    if table.fts.is_empty() {
        return Ok(());
    }
    let step = format!("resident-fts-{}", table.name);
    let size = table
        .fts
        .iter()
        .map(|c| {
            format!(
                "COALESCE(length(CAST({} AS BLOB)),0)",
                c.split_whitespace().next().unwrap()
            )
        })
        .collect::<Vec<_>>()
        .join("+");
    loop {
        reserve(storage)?;
        let after: Option<i64> =
            sqlx::query_scalar("SELECT last_id FROM _storage_conversion_steps WHERE step=?")
                .bind(&step)
                .fetch_optional(pool)
                .await?
                .flatten();
        let sql = format!("SELECT id,{size} FROM {t} WHERE _archive_file IS NULL AND NOT ({eligible}) AND id {comparison} ? ORDER BY id LIMIT {rows}",t=table.name,eligible=table.eligible,comparison=if after.is_some(){">"}else{">="},rows=bulk::FILE_ROWS);
        let mut stream = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(after.unwrap_or(i64::MIN))
            .fetch(pool);
        let mut ids = Vec::new();
        let mut bytes = 0_usize;
        while let Some(row) = stream.try_next().await? {
            let size = row.get::<i64, _>(1) as usize;
            if size > storage.descriptor.budget.record_bytes {
                return Err(storage_error(
                    "resident search record exceeds migration budget",
                ));
            }
            if !ids.is_empty() && bytes + size > storage.descriptor.budget.file_bytes {
                break;
            }
            ids.push(row.get::<i64, _>(0));
            bytes += size;
        }
        drop(stream);
        let Some(last) = ids.last().copied() else {
            break;
        };
        let ids = serde_json::to_string(&ids).map_err(storage_error)?;
        let permit = writer.lock().await?;
        let mut conn = permit.pool().acquire().await?;
        let mut tx = conn.begin().await?;
        for sql in [format!("DELETE FROM {t}_fts WHERE rowid IN (SELECT value FROM json_each(?))",t=table.name),format!("INSERT INTO {t}_fts(rowid,{columns}) SELECT id,{columns} FROM {t} WHERE id IN (SELECT value FROM json_each(?)) AND ({condition})",t=table.name,columns=table.fts_columns(),condition=table.fts_condition)] {
            sqlx::query(sqlx::AssertSqlSafe(sql)).bind(&ids).execute(&mut *tx).await?;
        }
        sqlx::query("INSERT INTO _storage_conversion_steps(step,last_id) VALUES(?,?) ON CONFLICT(step) DO UPDATE SET last_id=excluded.last_id").bind(&step).bind(last).execute(&mut *tx).await?;
        tx.commit().await?;
        super::reclaim::free_leaves(&mut conn, file, storage.descriptor.budget.decode_bytes)
            .await?;
    }
    Ok(())
}

async fn report(
    storage: &HybridStorage,
    pool: &SqlitePool,
    original: u64,
    total: u64,
    progress: &(impl Fn(MigrationProgress) + Send + Sync),
) -> Result<(), sqlx::Error> {
    let frames: i64 =
        sqlx::query_scalar("SELECT count(*) FROM frame_payloads WHERE state='sealed'")
            .fetch_one(pool)
            .await?;
    let bulk: i64 =
        sqlx::query_scalar("SELECT COALESCE(sum(rows),0) FROM _bulk_files WHERE state='published'")
            .fetch_one(pool)
            .await?;
    progress(MigrationProgress {
        message: "converting and freeing disk space",
        completed_records: Some((frames + bulk) as u64),
        total_records: Some(total),
        bytes_saved: Some(original.saturating_sub(super::reclaim::footprint(&storage.root)?)),
        available_bytes: Some(fs2::available_space(&storage.root)?),
    });
    Ok(())
}
