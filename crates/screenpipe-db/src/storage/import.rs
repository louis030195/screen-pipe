// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

//! Bounded resident-row batches shared by the in-place frame and element conversion.

use super::{storage_error, StorageBudget};
use futures::TryStreamExt;
use sqlx::{sqlite::SqliteRow, Row, SqliteConnection, TypeInfo, ValueRef};

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

pub(super) async fn columns(
    source: &sqlx::SqlitePool,
    table: &str,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT name FROM pragma_table_info(?) ORDER BY cid")
        .bind(table)
        .fetch_all(source)
        .await
}

pub(super) async fn batch(
    source: &sqlx::SqlitePool,
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
        .fetch(source);
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

pub(super) async fn insert(
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
