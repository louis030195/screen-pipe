// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use super::{codec, Kind, Record, Table, Value, FILE_ROWS, TABLES};
use crate::{
    storage::{storage_error, sync_directory, HybridStorage},
    DatabaseManager,
};
use screenpipe_sqlite_coordinator::SqliteWritePool;
use sqlx::{Row, SqlitePool, ValueRef};
use std::{path::Path, sync::Arc};

impl HybridStorage {
    pub(crate) async fn seal_bulk(
        self: &Arc<Self>,
        pool: &SqlitePool,
        writer: &SqliteWritePool,
    ) -> Result<usize, sqlx::Error> {
        if !self.has_bulk() {
            return Ok(0);
        }
        let elements = super::elements::seal(self, pool, writer).await?;
        if elements != 0 {
            return Ok(elements);
        }
        for table in TABLES.iter().filter(|t| t.name != "elements") {
            let rows = self.select_bulk(pool, table, None).await?;
            if !rows.is_empty() {
                return self.publish_bulk(pool, writer, table, rows, None).await;
            }
        }
        Ok(0)
    }

    async fn select_bulk(
        self: &Arc<Self>,
        pool: &SqlitePool,
        table: &Table,
        file: Option<i64>,
    ) -> Result<Vec<Record>, sqlx::Error> {
        let _token = self.read_token(pool).await?;
        let condition = if let Some(file) = file {
            format!("pending._archive_file={file}")
        } else {
            "pending._archive_mask!=0".into()
        };
        // Keep the pending index as the outer loop so LIMIT bounds work before
        // materializing IDs from a large history.
        let candidates=sqlx::query(sqlx::AssertSqlSafe(format!("SELECT v.id,({bytes}) AS bytes FROM main.{t} pending CROSS JOIN (SELECT * FROM {view} WHERE {eligible}) v ON v.id=pending.id WHERE {condition} ORDER BY pending.id LIMIT {FILE_ROWS}",bytes=table.all_bytes("v."),view=table.view(),t=table.name,eligible=table.eligible))).fetch_all(pool).await?;
        let mut ids = Vec::new();
        let mut bytes = 0;
        for candidate in candidates {
            let size = candidate.try_get::<i64, _>("bytes")? as usize;
            if size > self.descriptor.budget.record_bytes {
                return Err(storage_error("bulk record exceeds sealing budget"));
            }
            if file.is_none() && !ids.is_empty() && bytes + size > self.descriptor.budget.file_bytes
            {
                break;
            }
            ids.push(candidate.try_get::<i64, _>("id")?);
            bytes += size;
            if bytes > self.descriptor.budget.decode_bytes / 2 {
                return Err(storage_error("bulk batch exceeds sealing budget"));
            }
        }
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let columns = table
            .columns
            .iter()
            .map(|c| format!("v.{}", c.name))
            .collect::<Vec<_>>()
            .join(",");
        let rows=sqlx::query(sqlx::AssertSqlSafe(format!("SELECT v.id,e._archive_generation,{columns} FROM {view} v JOIN main.{t} e ON e.id=v.id WHERE v.id IN (SELECT value FROM json_each(?)) ORDER BY v.id",view=table.view(),t=table.name)))
            .bind(serde_json::to_string(&ids).map_err(storage_error)?).fetch_all(pool).await?;
        rows.iter()
            .map(|r| {
                let values = table
                    .columns
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        if r.try_get_raw(i + 2)?.is_null() {
                            return Ok(Value::Null);
                        }
                        Ok(match c.kind {
                            Kind::Text => Value::Text(r.try_get(i + 2)?),
                            Kind::Integer => Value::Integer(r.try_get(i + 2)?),
                            Kind::Real => Value::Real(r.try_get(i + 2)?),
                        })
                    })
                    .collect::<Result<Vec<_>, sqlx::Error>>()?;
                Ok(Record {
                    id: r.try_get("id")?,
                    generation: r.try_get("_archive_generation")?,
                    values,
                })
            })
            .collect()
    }

    async fn publish_bulk(
        self: &Arc<Self>,
        pool: &SqlitePool,
        writer: &SqliteWritePool,
        table: &'static Table,
        rows: Vec<Record>,
        replaces: Option<i64>,
    ) -> Result<usize, sqlx::Error> {
        tracing::info!(
            table = table.name,
            rows = rows.len(),
            "sealing bulk payload batch"
        );
        let policy: (String, i64) =
            sqlx::query_as("SELECT policy,required_surfaces FROM storage_metadata")
                .fetch_one(pool)
                .await?;
        if fs2::available_space(&self.root)?
            < self.descriptor.budget.disk_reserve_bytes
                + self.descriptor.budget.file_bytes as u64 * 2
        {
            return Err(storage_error("bulk temporary disk reserve reached"));
        }
        let relative = self
            .descriptor
            .payloads
            .join("bulk-v1")
            .join(table.name)
            .join(format!("{}.parquet", uuid::Uuid::new_v4()));
        let file = self.payload_path(&relative)?;
        std::fs::create_dir_all(file.parent().unwrap())?;
        let file_id = {
            let permit = writer.lock().await?;
            sqlx::query(
                "INSERT INTO _bulk_files(table_name,path,state,rows) VALUES(?,?,'encoding',?)",
            )
            .bind(table.name)
            .bind(relative.to_string_lossy().as_ref())
            .bind(rows.len() as i64)
            .execute(permit.pool())
            .await?
            .last_insert_rowid()
        };
        crate::storage::faults::checkpoint("bulk_reserved");
        let copy = rows.clone();
        let budget = self.descriptor.budget.clone();
        let file_for_job = file.clone();
        let decoder = Arc::clone(&self.decoder)
            .acquire_owned()
            .await
            .map_err(|_| sqlx::Error::PoolClosed)?;
        let lease = Arc::clone(&self.leases).read_owned().await;
        let hash = tokio::task::spawn_blocking(move || {
            let (_decoder, _lease) = (decoder, lease);
            let hash = codec::write(&file_for_job, table, &copy)?;
            if codec::read(&file_for_job, &hash, table, &budget)? != copy {
                return Err(storage_error("bulk encoded parity mismatch"));
            }
            Ok::<_, sqlx::Error>(hash)
        })
        .await
        .map_err(storage_error)??;
        let mut directory = file.parent().unwrap();
        loop {
            sync_directory(directory)?;
            if directory == self.root {
                break;
            }
            directory = directory
                .parent()
                .ok_or_else(|| storage_error("bulk parent missing"))?;
        }
        crate::storage::faults::checkpoint("bulk_files_synced");
        let permit = writer.lock().await?;
        let mut tx = permit.pool().begin().await?;
        let current: (String, i64) =
            sqlx::query_as("SELECT policy,required_surfaces FROM storage_metadata")
                .fetch_one(&mut *tx)
                .await?;
        let pairs = serde_json::to_string(
            &rows
                .iter()
                .map(|r| (r.id, r.generation))
                .collect::<Vec<_>>(),
        )
        .map_err(storage_error)?;
        let matches="(id,_archive_generation) IN (SELECT json_extract(value,'$[0]'),json_extract(value,'$[1]') FROM json_each(?))";
        let condition = replaces.map_or_else(
            || "_archive_mask!=0".into(),
            |id| format!("_archive_file={id}"),
        );
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM main.{} WHERE {matches} AND {condition}",
            table.name
        )))
        .bind(&pairs)
        .fetch_one(&mut *tx)
        .await?;
        let eligible:i64=sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {} WHERE id IN (SELECT json_extract(value,'$[0]') FROM json_each(?)) AND ({})",table.view(),table.eligible))).bind(&pairs).fetch_one(&mut *tx).await?;
        if current != policy || count != rows.len() as i64 || eligible != count {
            sqlx::query("UPDATE _bulk_files SET state='retired' WHERE id=?")
                .bind(file_id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            return Ok(0);
        }
        let freed: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT COALESCE(SUM({}),0) FROM main.{} WHERE {matches}",
            table.local_bytes(""),
            table.name
        )))
        .bind(&pairs)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(sqlx::AssertSqlSafe(format!("UPDATE _bulk_files SET state='dirty' WHERE id IN (SELECT _archive_file FROM main.{} WHERE {matches})",table.name))).bind(&pairs).execute(&mut *tx).await?;
        sqlx::query("UPDATE _bulk_files SET checksum=?,state='published' WHERE id=?")
            .bind(hash)
            .bind(file_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE storage_metadata SET maintenance=1")
            .execute(&mut *tx)
            .await?;
        let mut clear = table
            .columns
            .iter()
            .map(|c| format!("{}={}", c.name, c.empty))
            .collect::<Vec<_>>()
            .join(",");
        if table.name == "audio_transcriptions" {
            clear.push_str(",_archive_digest=screenpipe_payload_sha256((SELECT transcription FROM _bulk_logical_audio_transcriptions v WHERE v.id=audio_transcriptions.id))");
        }
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE main.{} SET {clear},_archive_file=?,_archive_mask=0 WHERE {matches}",
            table.name
        )))
        .bind(file_id)
        .bind(&pairs)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE storage_metadata SET maintenance=0,staging_bytes=staging_bytes-?")
            .bind(freed)
            .execute(&mut *tx)
            .await?;
        crate::storage::faults::checkpoint("bulk_before_commit");
        tx.commit().await?;
        crate::storage::faults::checkpoint("bulk_committed");
        Ok(rows.len())
    }

    pub(crate) async fn reclaim_bulk(
        self: &Arc<Self>,
        pool: &SqlitePool,
        writer: &SqliteWritePool,
    ) -> Result<usize, sqlx::Error> {
        if !self.has_bulk() {
            return Ok(0);
        }
        super::elements::reclaim(self, pool, writer).await?;
        let dirty: Option<(i64, String)> = sqlx::query_as(
            "SELECT id,table_name FROM _bulk_files WHERE state='dirty' AND table_name!='elements' ORDER BY id LIMIT 1",
        )
        .fetch_optional(pool)
        .await?;
        if let Some((id, name)) = dirty {
            let table = TABLES
                .iter()
                .find(|t| t.name == name)
                .ok_or_else(|| storage_error("unknown bulk table"))?;
            let rows = self.select_bulk(pool, table, Some(id)).await?;
            if !rows.is_empty() {
                self.publish_bulk(pool, writer, table, rows, Some(id))
                    .await?;
            }
        }
        let permit = writer.lock().await?;
        for table in TABLES.iter().filter(|t| t.name != "elements") {
            sqlx::query(sqlx::AssertSqlSafe(format!("UPDATE _bulk_files SET state='retired' WHERE table_name=? AND state IN ('dirty','encoding') AND NOT EXISTS(SELECT 1 FROM main.{} WHERE _archive_file=_bulk_files.id)",table.name))).bind(table.name).execute(permit.pool()).await?;
        }
        let retired = sqlx::query(
            "SELECT id,path FROM _bulk_files WHERE state='retired' ORDER BY id LIMIT 32",
        )
        .fetch_all(permit.pool())
        .await?;
        {
            let Ok(_leases) = Arc::clone(&self.leases).try_write_owned() else {
                return Ok(0);
            };
            self.bulk.cache.lock().map_err(storage_error)?.clear();
            for row in &retired {
                let path = self.payload_path(Path::new(row.try_get::<&str, _>("path")?))?;
                match std::fs::remove_file(&path) {
                    Ok(()) => sync_directory(path.parent().unwrap())?,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        for row in &retired {
            sqlx::query("DELETE FROM _bulk_files WHERE id=? AND state='retired'")
                .bind(row.try_get::<i64, _>("id")?)
                .execute(permit.pool())
                .await?;
        }
        Ok(retired.len())
    }

    pub(crate) async fn verify_bulk(
        self: &Arc<Self>,
        pool: &SqlitePool,
    ) -> Result<(), sqlx::Error> {
        if !self.has_bulk() {
            return Ok(());
        }
        let _token = self.read_token(pool).await?;
        super::elements::verify(self, pool).await?;
        for (index, table) in TABLES
            .iter()
            .enumerate()
            .filter(|(_, t)| t.name != "elements")
        {
            let bad:i64=sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM main.{} e LEFT JOIN _bulk_files p ON p.id=e._archive_file WHERE e._archive_mask<0 OR e._archive_mask>{} OR (e._archive_mask!={} AND (p.id IS NULL OR p.table_name!=? OR p.state NOT IN ('published','dirty')))",table.name,table.mask(),table.mask()))).bind(table.name).fetch_one(pool).await?;
            if bad != 0 {
                return Err(storage_error("bulk catalog is incomplete"));
            }
            let files=sqlx::query("SELECT id,path,checksum FROM _bulk_files WHERE table_name=? AND state IN ('published','dirty')").bind(table.name).fetch_all(pool).await?;
            for file in files {
                let path: String = file.try_get("path")?;
                let hash: String = file.try_get("checksum")?;
                let storage = Arc::clone(self);
                let records = tokio::task::spawn_blocking(move || {
                    codec::read(
                        &storage.payload_path(Path::new(&path))?,
                        &hash,
                        &TABLES[index],
                        &storage.descriptor.budget,
                    )
                })
                .await
                .map_err(storage_error)??;
                let ids: Vec<(i64, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                    "SELECT id,_archive_generation FROM main.{} WHERE _archive_file=? ORDER BY id",
                    table.name
                )))
                .bind(file.try_get::<i64, _>("id")?)
                .fetch_all(pool)
                .await?;
                for (id, generation) in ids {
                    let row = records
                        .binary_search_by_key(&id, |r| r.id)
                        .map_err(|_| storage_error("bulk catalog row absent"))?;
                    if records[row].generation > generation {
                        return Err(storage_error("bulk generation mismatch"));
                    }
                }
            }
        }
        Ok(())
    }
}

impl DatabaseManager {
    pub async fn seal_payloads(&self) -> Result<usize, sqlx::Error> {
        self.seal_frame_payloads().await
    }
}

pub(crate) async fn export(
    source: &DatabaseManager,
    output: &mut sqlx::SqliteConnection,
) -> Result<(), sqlx::Error> {
    if !source.storage.as_ref().is_some_and(|s| s.has_bulk()) {
        return Ok(());
    }
    let legacy: Vec<(String, String, String)> =
        sqlx::query_as("SELECT kind,name,sql FROM _bulk_legacy_sql ORDER BY kind,name")
            .fetch_all(&mut *output)
            .await?;
    let indexes: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type='index' AND name LIKE '_bulk_%'",
    )
    .fetch_all(&mut *output)
    .await?;
    for name in indexes {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP INDEX {name}")))
            .execute(&mut *output)
            .await?;
    }
    super::elements::export(source, output).await?;
    for table in TABLES.iter().filter(|t| t.name != "elements") {
        let columns = table
            .columns
            .iter()
            .map(|c| (c.name, c.kind))
            .collect::<Vec<_>>();
        let selected = columns
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(",");
        let mut after = None;
        loop {
            let candidates = sqlx::query(sqlx::AssertSqlSafe(format!(
                "SELECT id,({}) AS bytes FROM {} WHERE id{}? ORDER BY id LIMIT 512",
                table.all_bytes(""),
                table.name,
                if after.is_some() { ">" } else { ">=" }
            )))
            .bind(after.unwrap_or(i64::MIN))
            .fetch_all(&source.pool)
            .await?;
            if candidates.is_empty() {
                break;
            }
            let mut ids = Vec::new();
            let mut bytes = 0;
            for row in candidates {
                let size = row.try_get::<i64, _>("bytes")? as usize;
                if !ids.is_empty()
                    && bytes + size
                        > source
                            .storage
                            .as_ref()
                            .unwrap()
                            .descriptor
                            .budget
                            .file_bytes
                {
                    break;
                }
                bytes += size;
                ids.push(row.try_get::<i64, _>("id")?);
            }
            after = ids.last().copied();
            let rows=sqlx::query(sqlx::AssertSqlSafe(format!("SELECT id,{selected} FROM {} WHERE id IN (SELECT value FROM json_each(?)) ORDER BY id",table.name)))
                .bind(serde_json::to_string(&ids).map_err(storage_error)?).fetch_all(&source.pool).await?;
            let values = rows
                .iter()
                .map(|row| {
                    let mut values = vec![serde_json::Value::from(row.try_get::<i64, _>("id")?)];
                    for (i, (_, kind)) in columns.iter().enumerate() {
                        values.push(if row.try_get_raw(i + 1)?.is_null() {
                            serde_json::Value::Null
                        } else {
                            match kind {
                                Kind::Text => {
                                    serde_json::Value::from(row.try_get::<String, _>(i + 1)?)
                                }
                                Kind::Integer => {
                                    serde_json::Value::from(row.try_get::<i64, _>(i + 1)?)
                                }
                                Kind::Real => {
                                    serde_json::Value::from(row.try_get::<f64, _>(i + 1)?)
                                }
                            }
                        });
                    }
                    Ok(values)
                })
                .collect::<Result<Vec<_>, sqlx::Error>>()?;
            let projection = (0..columns.len() + 1)
                .map(|i| format!("json_extract(value,'$[{i}]') AS c{i}"))
                .collect::<Vec<_>>()
                .join(",");
            let assignments = columns
                .iter()
                .enumerate()
                .map(|(i, (name, _))| format!("{name}=patch.c{}", i + 1))
                .collect::<Vec<_>>()
                .join(",");
            sqlx::query(sqlx::AssertSqlSafe(format!("WITH patch AS (SELECT {projection} FROM json_each(?)) UPDATE {t} SET {assignments} FROM patch WHERE {t}.id=patch.c0",t=table.name)))
                .bind(serde_json::to_string(&values).map_err(storage_error)?).execute(&mut *output).await?;
        }
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP VIEW {};", table.view())))
            .execute(&mut *output)
            .await?;
        if !table.fts.is_empty() {
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "DROP TABLE {}_fts",
                table.name
            )))
            .execute(&mut *output)
            .await?;
        }
        let internals: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM pragma_table_info(?) WHERE name LIKE '_archive_%'",
        )
        .bind(table.name)
        .fetch_all(&mut *output)
        .await?;
        for column in internals {
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "ALTER TABLE {} DROP COLUMN {column}",
                table.name
            )))
            .execute(&mut *output)
            .await?;
        }
    }
    sqlx::raw_sql("DROP TABLE _bulk_files; DROP TABLE _bulk_legacy_sql;")
        .execute(&mut *output)
        .await?;
    for (kind, name, sql) in &legacy {
        if kind == "table" && name != "elements" {
            sqlx::raw_sql(sqlx::AssertSqlSafe(sql.as_str()))
                .execute(&mut *output)
                .await?;
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "INSERT INTO {name}({name}) VALUES('rebuild')"
            )))
            .execute(&mut *output)
            .await?;
        }
    }
    for (kind, _, sql) in &legacy {
        if kind != "table" {
            sqlx::raw_sql(sqlx::AssertSqlSafe(sql.as_str()))
                .execute(&mut *output)
                .await?;
        }
    }
    Ok(())
}
