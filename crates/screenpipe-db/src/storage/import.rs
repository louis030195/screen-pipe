// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

//! Offline construction streams the frozen source into the final generation.
//! Only one byte-bounded batch is staged before sealing and page reclamation.

use super::{
    lifecycle::{MigrationProgress, TableParity},
    storage_error, StorageBudget,
};
use crate::DatabaseManager;
use futures::TryStreamExt;
use sha2::{Digest, Sha256};
use sqlx::{sqlite::SqliteRow, Connection, Row, SqliteConnection, TypeInfo, ValueRef};
use std::path::Path;

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn reserve(root: &Path, budget: &StorageBudget) -> Result<(), sqlx::Error> {
    let needed = budget
        .disk_reserve_bytes
        .saturating_add(budget.file_bytes as u64 * 2);
    let available = fs2::available_space(root)?;
    if available < needed {
        return Err(storage_error(format!(
            "migration needs {needed} free bytes for its next batch and disk reserve; {available} available; original database retained"
        )));
    }
    Ok(())
}

/// Reproduce schema and migration provenance in an empty index. Shadow tables
/// are created by their virtual table module; their logical indexes are filled
/// incrementally during import.
pub(super) async fn prepare(
    source: &DatabaseManager,
    index: &Path,
    budget: &StorageBudget,
) -> Result<(), sqlx::Error> {
    reserve(index.parent().unwrap(), budget)?;
    let objects: Vec<String> = sqlx::query_scalar(
        "SELECT m.sql FROM sqlite_master m WHERE m.sql IS NOT NULL AND m.name NOT LIKE 'sqlite_%' AND NOT EXISTS(SELECT 1 FROM pragma_table_list t WHERE t.schema='main' AND t.name=m.tbl_name AND t.type='shadow') ORDER BY CASE m.type WHEN 'table' THEN 0 WHEN 'index' THEN 1 WHEN 'view' THEN 2 ELSE 3 END,m.rowid",
    ).fetch_all(&source.pool).await?;
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(index)
        .create_if_missing(true)
        .pragma("auto_vacuum", "INCREMENTAL")
        .pragma("temp_store", "FILE")
        .foreign_keys(false);
    let mut conn = SqliteConnection::connect_with(&options).await?;
    for sql in objects {
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(&mut conn)
            .await?;
    }
    let columns = columns(source, "_sqlx_migrations").await?;
    let mut after = None;
    loop {
        let rows = batch(source, "_sqlx_migrations", &columns, after, budget).await?;
        if rows.is_empty() {
            break;
        }
        after = Some(rows.last().unwrap().try_get::<i64, _>(0)?);
        insert(&mut conn, "_sqlx_migrations", &columns, &rows, false).await?;
    }
    conn.close().await?;
    super::faults::checkpoint("migration_schema_ready");
    Ok(())
}

async fn columns(source: &DatabaseManager, table: &str) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT name FROM pragma_table_info(?) ORDER BY cid")
        .bind(table)
        .fetch_all(&source.pool)
        .await
}

async fn batch(
    source: &DatabaseManager,
    table: &str,
    columns: &[String],
    after: Option<i64>,
    budget: &StorageBudget,
) -> Result<Vec<SqliteRow>, sqlx::Error> {
    let sql = format!(
        "SELECT rowid,{} FROM {} WHERE rowid{}? ORDER BY rowid LIMIT {}",
        columns
            .iter()
            .map(|c| quote(c))
            .collect::<Vec<_>>()
            .join(","),
        quote(table),
        if after.is_some() { ">" } else { ">=" },
        if table == "frames" {
            budget.file_rows
        } else {
            super::bulk::FILE_ROWS
        }
    );
    let mut stream = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(after.unwrap_or(i64::MIN))
        .fetch(&source.pool);
    let mut rows = Vec::new();
    let mut bytes = 0;
    while let Some(row) = stream.try_next().await? {
        let mut size = 0;
        for i in 0..row.len() {
            let raw = row.try_get_raw(i)?;
            size += if raw.is_null() {
                0
            } else {
                match raw.type_info().name() {
                    "INTEGER" | "REAL" => 8,
                    _ => row.try_get::<Vec<u8>, _>(i)?.len(),
                }
            };
        }
        if size > budget.decode_bytes / 2 {
            return Err(storage_error(format!(
                "migration record in {table} exceeds decode budget"
            )));
        }
        if !rows.is_empty() && bytes + size > budget.file_bytes {
            break;
        }
        bytes += size;
        rows.push(row);
    }
    Ok(rows)
}

async fn insert(
    conn: &mut SqliteConnection,
    table: &str,
    columns: &[String],
    rows: &[SqliteRow],
    elements: bool,
) -> Result<(), sqlx::Error> {
    let destination = if elements {
        "_bulk_element_rows"
    } else {
        table
    };
    let prefix = format!(
        "INSERT INTO {}(rowid,{}{}) ",
        quote(destination),
        columns
            .iter()
            .map(|c| quote(c))
            .collect::<Vec<_>>()
            .join(","),
        if elements { ",_archive_generation" } else { "" }
    );
    // SQLite's parameter limit bounds statement size independently of payload
    // bytes. Values retain their original SQLite storage class.
    for chunk in rows.chunks(32766 / (columns.len() + 2)) {
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(&prefix);
        let mut error = None;
        query.push_values(chunk, |mut values, row| {
            for i in 0..row.len() {
                let value = (|| -> Result<(), sqlx::Error> {
                    let raw = row.try_get_raw(i)?;
                    if raw.is_null() {
                        values.push_bind(None::<i64>);
                    } else {
                        match raw.type_info().name() {
                            "INTEGER" => {
                                values.push_bind(row.try_get::<i64, _>(i)?);
                            }
                            "REAL" => {
                                values.push_bind(row.try_get::<f64, _>(i)?);
                            }
                            "TEXT" => {
                                values.push_bind(row.try_get::<String, _>(i)?);
                            }
                            _ => {
                                values.push_bind(row.try_get::<Vec<u8>, _>(i)?);
                            }
                        }
                    }
                    Ok(())
                })();
                if let Err(e) = value {
                    error = Some(e);
                }
            }
            if elements {
                values.push_bind(1_i64);
            }
        });
        if let Some(error) = error {
            return Err(error);
        }
        query.build().execute(&mut *conn).await?;
    }
    if elements {
        super::bulk::elements::import_batch(
            conn,
            rows.first().unwrap().try_get(0)?,
            rows.last().unwrap().try_get(0)?,
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn records(
    source: &DatabaseManager,
    candidate: &DatabaseManager,
    progress: &(impl Fn(MigrationProgress) + Send + Sync),
) -> Result<Vec<TableParity>, sqlx::Error> {
    let storage = candidate.storage.as_ref().unwrap();
    let budget = &storage.descriptor.budget;
    // Historical rows already reflect application triggers. Preserve those
    // definitions for subsequent writes without replaying their side effects.
    // Hybrid insert triggers alone construct new postings and payload catalogs.
    let original: Vec<(String, String)> =
        sqlx::query_as("SELECT name,sql FROM sqlite_master WHERE type='trigger'")
            .fetch_all(&source.pool)
            .await?;
    let mut restore = Vec::new();
    {
        let writer = candidate.coordinated_writer();
        let permit = writer.lock().await?;
        for (name, sql) in original {
            let current: Option<String> =
                sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?")
                    .bind(&name)
                    .fetch_optional(permit.pool())
                    .await?;
            if current.as_deref() == Some(&sql) {
                sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                    "DROP TRIGGER {}",
                    quote(&name)
                )))
                .execute(permit.pool())
                .await?;
                restore.push(sql);
            }
        }
    }
    let logical = super::lifecycle::logical_tables(source).await?;
    let mut tables: Vec<String> = logical
        .iter()
        .filter(|t| t.as_str() != "sqlite_sequence")
        .cloned()
        .collect();
    // Unchanged virtual search tables retain their existing indexed contents.
    let virtuals: Vec<(String, String)> = sqlx::query_as("SELECT name,sql FROM sqlite_master WHERE type='table' AND sql LIKE 'CREATE VIRTUAL TABLE%'")
        .fetch_all(&source.pool).await?;
    for (name, sql) in virtuals {
        let current: Option<String> =
            sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE name=?")
                .bind(&name)
                .fetch_optional(&candidate.pool)
                .await?;
        if current.as_deref() == Some(&sql) && !tables.contains(&name) {
            tables.push(name);
        }
    }
    tables.push("sqlite_sequence".into());
    let mut total_records = 0_u64;
    for table in &tables {
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM {}",
            quote(table)
        )))
        .fetch_one(&source.pool)
        .await?;
        total_records += count as u64;
    }
    let mut completed_records = 0_u64;
    let report = |completed_records| {
        progress(MigrationProgress {
            message: "converting and compressing recordings",
            completed_records: Some(completed_records),
            total_records: Some(total_records),
        })
    };
    report(completed_records);
    let mut receipts = Vec::new();
    for table in tables {
        tracing::info!(%table, "streaming migration records");
        let columns = columns(source, &table).await?;
        let elements = table == "elements" && storage.has_bulk();
        let mut after = None;
        let mut original_hash = Sha256::new();
        let mut converted_hash = Sha256::new();
        let mut count = 0;
        loop {
            reserve(&storage.root, budget)?;
            let rows = batch(source, &table, &columns, after, budget).await?;
            if rows.is_empty() {
                break;
            }
            let last = rows.last().unwrap().try_get::<i64, _>(0)?;
            {
                let writer = candidate.coordinated_writer();
                let permit = writer.lock().await?;
                let mut conn = permit.pool().acquire().await?;
                // The unpublished candidate may contain forward relationships
                // across batches/tables. Batch comparison preserves their source IDs.
                conn.close_on_drop();
                sqlx::query("PRAGMA foreign_keys=OFF")
                    .execute(&mut *conn)
                    .await?;
                let mut tx = conn.begin().await?;
                if table == "sqlite_sequence" && after.is_none() {
                    sqlx::query("DELETE FROM sqlite_sequence")
                        .execute(&mut *tx)
                        .await?;
                }
                if table != "_sqlx_migrations" {
                    insert(&mut tx, &table, &columns, &rows, elements).await?;
                }
                verify_batch(
                    &mut tx,
                    &table,
                    &columns,
                    &rows,
                    elements,
                    &mut original_hash,
                    &mut converted_hash,
                )
                .await?;
                if table == "frames" {
                    sqlx::query("UPDATE frame_payloads SET capture_version=NULL WHERE frame_id BETWEEN ? AND ?")
                        .bind(rows.first().unwrap().try_get::<i64, _>(0)?)
                        .bind(last).execute(&mut *tx).await?;
                }
                tx.commit().await?;
            }
            let batch_count = rows.len() as u64;
            count += batch_count;
            drop(rows);
            super::faults::checkpoint("migration_batch_staged");
            while candidate.seal_frame_payloads().await? != 0 {}
            reclaim(candidate).await?;
            super::faults::checkpoint("migration_batch_sealed");
            completed_records += batch_count;
            report(completed_records);
            after = Some(last);
        }
        if logical.contains(&table) {
            receipts.push(TableParity {
                table,
                rows: count,
                sha256: format!("{:x}", original_hash.finalize()),
            });
        }
    }
    let writer = candidate.coordinated_writer();
    let permit = writer.lock().await?;
    for sql in restore {
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(permit.pool())
            .await?;
    }
    receipts.sort_by(|a, b| a.table.cmp(&b.table));
    Ok(receipts)
}

async fn verify_batch(
    conn: &mut SqliteConnection,
    table: &str,
    columns: &[String],
    rows: &[SqliteRow],
    elements: bool,
    original: &mut Sha256,
    converted: &mut Sha256,
) -> Result<(), sqlx::Error> {
    let destination = if elements {
        "_bulk_element_rows"
    } else {
        table
    };
    let sql = format!(
        "SELECT rowid,{} FROM {} WHERE rowid BETWEEN ? AND ? ORDER BY rowid",
        columns
            .iter()
            .map(|c| quote(c))
            .collect::<Vec<_>>()
            .join(","),
        quote(destination)
    );
    let mut actual = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(rows.first().unwrap().try_get::<i64, _>(0)?)
        .bind(rows.last().unwrap().try_get::<i64, _>(0)?)
        .fetch(&mut *conn);
    for row in rows {
        let copied = actual
            .try_next()
            .await?
            .ok_or_else(|| storage_error("migration batch row missing"))?;
        if row.try_get::<i64, _>(0)? != copied.try_get::<i64, _>(0)? {
            return Err(storage_error("migration batch row identity differs"));
        }
        for index in 1..row.len() {
            super::lifecycle::hash_value(original, row, index, None)?;
            super::lifecycle::hash_value(converted, &copied, index, None)?;
        }
        original.update(b"E");
        converted.update(b"E");
    }
    if actual.try_next().await?.is_some()
        || original.clone().finalize() != converted.clone().finalize()
    {
        return Err(storage_error(format!(
            "migration batch parity failed: {table}"
        )));
    }
    Ok(())
}

async fn reclaim(candidate: &DatabaseManager) -> Result<(), sqlx::Error> {
    let writer = candidate.coordinated_writer();
    let permit = writer.lock().await?;
    let mut conn = permit.pool().acquire().await?;
    // The fresh index enables incremental vacuum before schema creation. Freed
    // batch pages are reclaimed in place; each step bounds the construction WAL.
    loop {
        let free: i64 = sqlx::query_scalar("PRAGMA freelist_count")
            .fetch_one(&mut *conn)
            .await?;
        if free == 0 {
            break;
        }
        sqlx::query("PRAGMA incremental_vacuum(256)")
            .execute(&mut *conn)
            .await?;
        super::schema::construction_checkpoint(&mut conn).await?;
    }
    super::schema::construction_checkpoint(&mut conn).await
}
