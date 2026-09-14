// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use super::{names, TABLE};
use crate::{
    storage::{
        bulk::{codec, Kind, Record, Value, FILE_ROWS},
        storage_error, sync_directory, HybridStorage,
    },
    DatabaseManager,
};
use futures::TryStreamExt;
use screenpipe_sqlite_coordinator::SqliteWritePool;
use sqlx::{Connection, Row, SqlitePool, ValueRef};
use std::{collections::BTreeSet, path::Path, sync::Arc};

struct Encoded {
    id: i64,
    first: i64,
    last: i64,
    rows: usize,
}

fn record(row: &sqlx::sqlite::SqliteRow) -> Result<Record, sqlx::Error> {
    let values = TABLE
        .columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            if row.try_get_raw(i + 2)?.is_null() {
                return Ok(Value::Null);
            }
            Ok(match c.kind {
                Kind::Text => Value::Text(row.try_get(i + 2)?),
                Kind::Integer => Value::Integer(row.try_get(i + 2)?),
                Kind::Real => Value::Real(row.try_get(i + 2)?),
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    Ok(Record {
        id: row.try_get(0)?,
        generation: row.try_get(1)?,
        values,
    })
}

pub(crate) async fn seal(
    storage: &Arc<HybridStorage>,
    pool: &SqlitePool,
    writer: &SqliteWritePool,
) -> Result<usize, sqlx::Error> {
    let id: Option<i64> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT id FROM _bulk_element_rows WHERE _archive_deleted=0 AND ({}) ORDER BY id LIMIT 1",
        TABLE.eligible
    )))
    .fetch_optional(pool)
    .await?;
    let Some(id) = id else {
        return Ok(0);
    };
    let range:Option<(i64,i64,i64)>=sqlx::query_as("SELECT first_id,last_id,file_id FROM _bulk_element_ranges WHERE first_id=(SELECT max(first_id) FROM _bulk_element_ranges WHERE first_id<=?1) AND last_id>=?1").bind(id).fetch_optional(pool).await?;
    rewrite(storage, pool, writer, id, range).await
}

async fn encode(
    storage: &Arc<HybridStorage>,
    writer: &SqliteWritePool,
    rows: Vec<Record>,
) -> Result<Encoded, sqlx::Error> {
    if fs2::available_space(&storage.root)?
        < storage.descriptor.budget.disk_reserve_bytes
            + storage.descriptor.budget.file_bytes as u64 * 2
    {
        return Err(storage_error("element temporary disk reserve reached"));
    }
    let relative = storage
        .descriptor
        .payloads
        .join("bulk-v1/elements")
        .join(format!("{}.parquet", uuid::Uuid::new_v4()));
    let path = storage.payload_path(&relative)?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    let id = {
        let permit = writer.lock().await?;
        sqlx::query(
            "INSERT INTO _bulk_files(table_name,path,state,rows) VALUES('elements',?,'encoding',?)",
        )
        .bind(relative.to_string_lossy().as_ref())
        .bind(rows.len() as i64)
        .execute(permit.pool())
        .await?
        .last_insert_rowid()
    };
    crate::storage::faults::checkpoint("bulk_reserved");
    let frames: BTreeSet<_> = rows
        .iter()
        .filter_map(|r| match r.values[0] {
            Value::Integer(id) => Some(id),
            _ => None,
        })
        .collect();
    let output = Encoded {
        id,
        first: rows.first().unwrap().id,
        last: rows.last().unwrap().id,
        rows: rows.len(),
    };
    let budget = storage.descriptor.budget.clone();
    let file = path.clone();
    let decoder = Arc::clone(&storage.decoder)
        .acquire_owned()
        .await
        .map_err(|_| sqlx::Error::PoolClosed)?;
    let hash = tokio::task::spawn_blocking(move || {
        let _decoder = decoder;
        let hash = codec::write(&file, &TABLE, &rows)?;
        if codec::read(&file, &hash, &TABLE, &budget)? != rows {
            return Err(storage_error("element encoded parity mismatch"));
        }
        Ok::<_, sqlx::Error>(hash)
    })
    .await
    .map_err(storage_error)??;
    let mut directory = path.parent().unwrap();
    loop {
        sync_directory(directory)?;
        if directory == storage.root {
            break;
        }
        directory = directory
            .parent()
            .ok_or_else(|| storage_error("element parent missing"))?;
    }
    let permit = writer.lock().await?;
    let mut tx = permit.pool().begin().await?;
    sqlx::query("UPDATE _bulk_files SET checksum=? WHERE id=?")
        .bind(hash)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO _bulk_element_frames(frame_id,file_id) SELECT value,? FROM json_each(?)",
    )
    .bind(id)
    .bind(serde_json::to_string(&frames).map_err(storage_error)?)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    crate::storage::faults::checkpoint("bulk_files_synced");
    Ok(output)
}

async fn rewrite(
    storage: &Arc<HybridStorage>,
    pool: &SqlitePool,
    writer: &SqliteWritePool,
    id: i64,
    range: Option<(i64, i64, i64)>,
) -> Result<usize, sqlx::Error> {
    let _token = storage.read_token(pool).await?;
    let version: i64 = sqlx::query_scalar("SELECT version FROM _bulk_element_state")
        .fetch_one(pool)
        .await?;
    let policy: (String, i64) =
        sqlx::query_as("SELECT policy,required_surfaces FROM storage_metadata")
            .fetch_one(pool)
            .await?;
    let (first, last, limit) = if let Some((first, last, _)) = range {
        let blocked: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT EXISTS(SELECT 1 FROM elements WHERE id BETWEEN ? AND ? AND NOT ({}))",
            TABLE.eligible
        )))
        .bind(first)
        .bind(last)
        .fetch_one(pool)
        .await?;
        if blocked {
            return Ok(0);
        }
        (first, last, String::new())
    } else {
        let next: Option<i64> =
            sqlx::query_scalar("SELECT min(first_id) FROM _bulk_element_ranges WHERE first_id>?")
                .bind(id)
                .fetch_one(pool)
                .await?;
        (
            id,
            next.map_or(i64::MAX, |n| n - 1),
            format!(" LIMIT {FILE_ROWS}"),
        )
    };
    // Release each read cursor before encoding takes the writer. Offline
    // migration has one connection; a recovered resident range can exceed one
    // file budget and must not hold that connection while waiting for itself.
    let mut after = None;
    let mut files = Vec::new();
    loop {
        let lower = after.map_or_else(|| "1".to_owned(), |id| format!("id>{id}"));
        let sql=format!("SELECT id,_archive_generation,{} FROM elements WHERE id BETWEEN ? AND ? AND {lower} AND ({}) ORDER BY id{limit}",names(),TABLE.eligible);
        let mut stream = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(first)
            .bind(last)
            .fetch(pool);
        let mut rows = Vec::new();
        let mut bytes = 0;
        while let Some(row) = stream.try_next().await? {
            let row = record(&row)?;
            if row.bytes() > storage.descriptor.budget.record_bytes {
                return Err(storage_error("element record exceeds sealing budget"));
            }
            if !rows.is_empty()
                && (rows.len() == FILE_ROWS
                    || bytes + row.bytes() > storage.descriptor.budget.file_bytes)
            {
                break;
            }
            bytes += row.bytes();
            rows.push(row);
        }
        drop(stream);
        let Some(last_row) = rows.last() else { break };
        after = Some(last_row.id);
        files.push(encode(storage, writer, rows).await?);
        if range.is_none() {
            break;
        }
    }
    let last = if range.is_some() {
        last
    } else if let Some(file) = files.last() {
        file.last
    } else {
        return Ok(0);
    };
    let permit = writer.lock().await?;
    let mut tx = permit.pool().begin().await?;
    let current: i64 = sqlx::query_scalar("SELECT version FROM _bulk_element_state")
        .fetch_one(&mut *tx)
        .await?;
    let current_policy: (String, i64) =
        sqlx::query_as("SELECT policy,required_surfaces FROM storage_metadata")
            .fetch_one(&mut *tx)
            .await?;
    if current != version || current_policy != policy {
        for file in files {
            sqlx::query("UPDATE _bulk_files SET state='retired' WHERE id=?")
                .bind(file.id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        return Ok(0);
    }
    let selection = if range.is_some() {
        "1".into()
    } else {
        format!("_archive_deleted=1 OR ({})", TABLE.eligible)
    };
    let freed:i64=sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT COALESCE(SUM({}),0) FROM _bulk_element_rows WHERE id BETWEEN ? AND ? AND ({selection})",TABLE.all_bytes("")))).bind(first).bind(last).fetch_one(&mut *tx).await?;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DELETE FROM _bulk_element_rows WHERE id BETWEEN ? AND ? AND ({selection})"
    )))
    .bind(first)
    .bind(last)
    .execute(&mut *tx)
    .await?;
    if let Some((_, _, old)) = range {
        sqlx::query("DELETE FROM _bulk_element_ranges WHERE file_id=?")
            .bind(old)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE _bulk_files SET state='retired' WHERE id=?")
            .bind(old)
            .execute(&mut *tx)
            .await?;
    }
    for file in &files {
        sqlx::query("INSERT INTO _bulk_element_ranges VALUES(?,?,?)")
            .bind(file.first)
            .bind(file.last)
            .bind(file.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE _bulk_files SET state='published' WHERE id=?")
            .bind(file.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE _bulk_element_rows SET _archive_file=? WHERE id BETWEEN ? AND ?")
            .bind(file.id)
            .bind(file.first)
            .bind(file.last)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE storage_metadata SET staging_bytes=staging_bytes-?")
        .bind(freed)
        .execute(&mut *tx)
        .await?;
    crate::storage::faults::checkpoint("bulk_before_commit");
    tx.commit().await?;
    crate::storage::faults::checkpoint("bulk_committed");
    let count = files.iter().map(|f| f.rows).sum();
    tracing::info!(rows = count, files = files.len(), "sealed element records");
    Ok(count)
}

pub(crate) async fn reclaim(
    storage: &Arc<HybridStorage>,
    pool: &SqlitePool,
    writer: &SqliteWritePool,
) -> Result<(), sqlx::Error> {
    let range:Option<(i64,i64,i64)>=sqlx::query_as("SELECT r.first_id,r.last_id,r.file_id FROM _bulk_element_ranges r JOIN _bulk_files p ON p.id=r.file_id WHERE p.state='dirty' ORDER BY r.first_id LIMIT 1").fetch_optional(pool).await?;
    if let Some(range) = range {
        rewrite(storage, pool, writer, range.0, Some(range)).await?;
    }
    let permit = writer.lock().await?;
    sqlx::query(
        "DELETE FROM _bulk_element_rows WHERE _archive_deleted=1 AND _archive_file IS NULL",
    )
    .execute(permit.pool())
    .await?;
    sqlx::query(
        "UPDATE _bulk_files SET state='retired' WHERE table_name='elements' AND state='encoding'",
    )
    .execute(permit.pool())
    .await?;
    sqlx::query("DELETE FROM _bulk_element_frames WHERE file_id IN (SELECT id FROM _bulk_files WHERE state='retired')").execute(permit.pool()).await?;
    Ok(())
}

pub(crate) async fn verify(
    storage: &Arc<HybridStorage>,
    pool: &SqlitePool,
) -> Result<(), sqlx::Error> {
    let bad:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM _bulk_element_ranges r JOIN _bulk_files p ON p.id=r.file_id WHERE r.first_id>r.last_id OR p.table_name!='elements' OR p.state NOT IN ('published','dirty') OR r.first_id<=(SELECT max(last_id) FROM _bulk_element_ranges WHERE first_id<r.first_id))").fetch_one(pool).await?;
    if bad {
        return Err(storage_error("element range catalog is invalid"));
    }
    let files:Vec<(i64,i64,i64,String,String)>=sqlx::query_as("SELECT r.file_id,r.first_id,r.last_id,p.path,p.checksum FROM _bulk_element_ranges r JOIN _bulk_files p ON p.id=r.file_id ORDER BY r.first_id").fetch_all(pool).await?;
    for (id, first, last, path, hash) in files {
        let storage = storage.clone();
        let rows = tokio::task::spawn_blocking(move || {
            codec::read(
                &storage.payload_path(Path::new(&path))?,
                &hash,
                &TABLE,
                &storage.descriptor.budget,
            )
        })
        .await
        .map_err(storage_error)??;
        if rows.first().map(|r| r.id) != Some(first) || rows.last().map(|r| r.id) != Some(last) {
            return Err(storage_error("element range identity mismatch"));
        }
        let expected: BTreeSet<_> = rows
            .iter()
            .filter_map(|r| match r.values[0] {
                Value::Integer(id) => Some(id),
                _ => None,
            })
            .collect();
        let actual: BTreeSet<i64> =
            sqlx::query_scalar("SELECT frame_id FROM _bulk_element_frames WHERE file_id=?")
                .bind(id)
                .fetch_all(pool)
                .await?
                .into_iter()
                .collect();
        if actual != expected {
            return Err(storage_error("element frame catalog mismatch"));
        }
    }
    // Aggregate verification spills to a temporary file. This checked-out
    // connection closes afterward, including cancellation, so its temporary
    // query settings never return to the live pool.
    let mut validation = pool.acquire().await?;
    validation.close_on_drop();
    sqlx::query("PRAGMA temp_store=FILE")
        .execute(&mut *validation)
        .await?;
    for (column, table) in [("parent_id", "_bulk_element_parent_refs")] {
        let bad:bool=sqlx::query_scalar(sqlx::AssertSqlSafe(format!("WITH actual AS (SELECT {column},count(*) AS rows FROM elements WHERE {column} IS NOT NULL GROUP BY {column}), difference AS (SELECT * FROM actual EXCEPT SELECT * FROM {table}), reverse_difference AS (SELECT * FROM {table} EXCEPT SELECT * FROM actual) SELECT EXISTS(SELECT 1 FROM difference UNION ALL SELECT 1 FROM reverse_difference)"))).fetch_one(&mut *validation).await?;
        if bad {
            return Err(storage_error("element relationship counts differ"));
        }
    }
    let groups: bool = sqlx::query_scalar("WITH actual AS (SELECT frame_id,source,role,COALESCE(on_screen,0) AS visibility,on_screen IS NULL AS is_null,count(*) AS rows FROM elements GROUP BY frame_id,source,role,COALESCE(on_screen,0),on_screen IS NULL), expected AS (SELECT g.frame_id,k.source,k.role,g.visibility,g.is_null,g.rows FROM _bulk_element_groups g JOIN _bulk_element_kinds k ON k.id=g.kind_id), difference AS (SELECT * FROM actual EXCEPT SELECT * FROM expected), reverse_difference AS (SELECT * FROM expected EXCEPT SELECT * FROM actual) SELECT EXISTS(SELECT 1 FROM difference UNION ALL SELECT 1 FROM reverse_difference)").fetch_one(&mut *validation).await?;
    if groups {
        return Err(storage_error("element summary counts differ"));
    }
    let missing:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM _bulk_element_parent_refs p WHERE NOT EXISTS(SELECT 1 FROM elements e WHERE e.id=p.parent_id))").fetch_one(&mut *validation).await?;
    if missing {
        return Err(storage_error("element parent is absent"));
    }
    Ok(())
}

pub(crate) async fn export(
    source: &DatabaseManager,
    output: &mut sqlx::SqliteConnection,
) -> Result<(), sqlx::Error> {
    super::register(output, source.storage.as_ref().unwrap().clone()).await?;
    let sql: String = sqlx::query_scalar("SELECT sql FROM _bulk_legacy_sql WHERE name='elements'")
        .fetch_one(&mut *output)
        .await?;
    sqlx::raw_sql(
        "DROP VIEW _bulk_logical_elements; DROP VIEW _bulk_element_search_content; DROP TABLE elements; DROP TABLE elements_fts;",
    )
    .execute(&mut *output)
    .await?;
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
        .execute(&mut *output)
        .await?;
    // Only the unpublished export copy suspends FK checks while rows are
    // restored in ID order. Full verification precedes its publication.
    sqlx::query("PRAGMA foreign_keys=OFF")
        .execute(&mut *output)
        .await?;
    let mut stream = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT id,_archive_generation,{} FROM elements ORDER BY id",
        names()
    )))
    .fetch(&source.pool);
    let mut batch = Vec::new();
    let mut bytes = 0;
    while let Some(row) = stream.try_next().await? {
        let row = record(&row)?;
        if !batch.is_empty()
            && (batch.len() == 128
                || bytes + row.bytes()
                    > source
                        .storage
                        .as_ref()
                        .unwrap()
                        .descriptor
                        .budget
                        .file_bytes)
        {
            export_batch(output, std::mem::take(&mut batch)).await?;
            bytes = 0;
        }
        bytes += row.bytes();
        batch.push(row);
    }
    drop(stream);
    if !batch.is_empty() {
        export_batch(output, batch).await?;
    }
    sqlx::query("PRAGMA foreign_keys=ON")
        .execute(&mut *output)
        .await?;
    sqlx::raw_sql("DROP TABLE _bulk_element_rows; DROP TABLE _bulk_element_ranges; DROP TABLE _bulk_element_frames; DROP VIEW _bulk_element_counts; DROP TABLE _bulk_element_groups; DROP TABLE _bulk_element_parent_refs; DROP TABLE _bulk_element_checks; DROP TABLE _bulk_element_state; DROP TABLE _bulk_element_kinds;").execute(&mut *output).await?;
    Ok(())
}

async fn export_batch(
    output: &mut sqlx::SqliteConnection,
    rows: Vec<Record>,
) -> Result<(), sqlx::Error> {
    let mut tx = output.begin().await?;
    for row in rows {
        let mut query = sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO elements(id,{}) VALUES({})",
            names(),
            vec!["?"; TABLE.columns.len() + 1].join(",")
        )))
        .bind(row.id);
        for value in row.values {
            query = match value {
                Value::Null => query.bind(Option::<String>::None),
                Value::Integer(v) => query.bind(v),
                Value::Real(v) => query.bind(v),
                Value::Text(v) => query.bind(v),
            };
        }
        query.execute(&mut *tx).await?;
    }
    tx.commit().await
}
