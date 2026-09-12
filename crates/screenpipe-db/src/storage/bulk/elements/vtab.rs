// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use super::{declaration, names, TABLE};
use crate::storage::{
    bulk::{Record, Value},
    storage_error, HybridStorage,
};
use rusqlite::{
    ffi, params, params_from_iter,
    types::{Value as SqlValue, ValueRef},
    vtab::*,
    Connection, OptionalExtension, Result,
};
use std::{
    collections::VecDeque,
    sync::{Arc, LazyLock},
};

pub(crate) async fn register(
    conn: &mut sqlx::SqliteConnection,
    storage: Arc<HybridStorage>,
) -> Result<(), sqlx::Error> {
    let mut handle = conn.lock_handle().await?;
    // SQLx owns the handle. The module borrows it only on its SQLite worker;
    // from_handle never closes the borrowed database.
    unsafe {
        Connection::from_handle(handle.as_raw_handle().as_ptr())
            .map_err(storage_error)?
            .create_module(
                c"screenpipe_elements",
                update_module_with_tx::<Elements>(),
                Some(storage),
            )
            .map_err(storage_error)?;
    }
    Ok(())
}

fn failure(message: &str) -> rusqlite::Error {
    rusqlite::Error::ModuleError(message.into())
}
fn constraint(message: &str) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        ffi::Error::new(ffi::SQLITE_CONSTRAINT),
        Some(message.into()),
    )
}

trait CachedSql {
    fn cached_execute<P: rusqlite::Params>(&self, sql: &str, params: P) -> Result<usize>;
    fn cached_query_row<T, P: rusqlite::Params, F: FnOnce(&rusqlite::Row<'_>) -> Result<T>>(
        &self,
        sql: &str,
        params: P,
        read: F,
    ) -> Result<T>;
}

impl CachedSql for Connection {
    fn cached_execute<P: rusqlite::Params>(&self, sql: &str, params: P) -> Result<usize> {
        self.prepare_cached(sql)?.execute(params)
    }
    fn cached_query_row<T, P: rusqlite::Params, F: FnOnce(&rusqlite::Row<'_>) -> Result<T>>(
        &self,
        sql: &str,
        params: P,
        read: F,
    ) -> Result<T> {
        self.prepare_cached(sql)?.query_row(params, read)
    }
}

pub(super) fn sql_value(value: &Value) -> SqlValue {
    match value {
        Value::Null => SqlValue::Null,
        Value::Integer(v) => SqlValue::Integer(*v),
        Value::Real(v) => SqlValue::Real(*v),
        Value::Text(v) => SqlValue::Text(v.clone()),
    }
}
fn value(value: SqlValue) -> Result<Value> {
    Ok(match value {
        SqlValue::Null => Value::Null,
        SqlValue::Integer(v) => Value::Integer(v),
        SqlValue::Real(v) => Value::Real(v),
        SqlValue::Text(v) => Value::Text(v),
        SqlValue::Blob(_) => return Err(constraint("element column type mismatch")),
    })
}
fn staged(row: &rusqlite::Row<'_>) -> Result<(Record, bool)> {
    let values = (0..TABLE.columns.len())
        .map(|i| value(row.get(i + 2)?))
        .collect::<Result<Vec<_>>>()?;
    Ok((
        Record {
            id: row.get(0)?,
            generation: row.get(1)?,
            values,
        },
        row.get::<_, i64>(TABLE.columns.len() + 2)? != 0,
    ))
}
struct RowSql {
    projection: String,
    lookup: String,
    upsert: String,
    resident: String,
}

static ROW_SQL: LazyLock<RowSql> = LazyLock::new(|| {
    let columns = names();
    let projection = format!("id,_archive_generation,{columns},_archive_deleted");
    RowSql {
        lookup: format!("SELECT {projection} FROM _bulk_element_rows WHERE id=?"),
        upsert: format!("INSERT OR REPLACE INTO _bulk_element_rows(id,_archive_generation,_archive_deleted,_archive_file,{columns}) VALUES({})", vec!["?";TABLE.columns.len()+4].join(",")),
        resident: format!("SELECT COALESCE((SELECT {} FROM _bulk_element_rows WHERE id=?),0)", TABLE.all_bytes("")),
        projection,
    }
});

fn location(db: &Connection, id: i64) -> Result<Option<(i64, String, String)>> {
    db.cached_query_row("SELECT r.file_id,p.path,p.checksum FROM _bulk_element_ranges r JOIN _bulk_files p ON p.id=r.file_id WHERE r.first_id=(SELECT max(first_id) FROM _bulk_element_ranges WHERE first_id<=?1) AND r.last_id>=?1", [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()
}
fn lookup(db: &Connection, storage: &HybridStorage, id: i64) -> Result<Option<Record>> {
    if let Some((row, deleted)) = db
        .cached_query_row(&ROW_SQL.lookup, [id], staged)
        .optional()?
    {
        return Ok((!deleted).then_some(row));
    }
    let Some((_, path, hash)) = location(db, id)? else {
        return Ok(None);
    };
    let rows = storage
        .bulk_records(&path, &hash, 0)
        .map_err(|_| failure("element archive unavailable"))?;
    Ok(rows
        .binary_search_by_key(&id, |r| r.id)
        .ok()
        .map(|i| rows[i].clone()))
}

#[repr(C)]
struct Elements {
    base: ffi::sqlite3_vtab,
    database: Connection,
    storage: Arc<HybridStorage>,
}

unsafe impl<'v> VTab<'v> for Elements {
    type Aux = Arc<HybridStorage>;
    type Cursor = Cursor;
    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Self::Aux>,
        _: &[&[u8]],
    ) -> Result<(String, Self)> {
        // Writes touch SQLite shadow state in the same transaction. SQLite
        // rolls these writes back with a failed outer statement/savepoint.
        let storage = aux
            .ok_or_else(|| failure("element storage missing"))?
            .clone();
        // SQLx owns the connection. Cached helper statements live for the
        // writer transaction and are finalized at its commit/rollback boundary.
        let database = unsafe { Connection::from_handle(db.handle())? };
        database.set_prepared_statement_cache_capacity(32);
        Ok((
            format!(
                "CREATE TABLE x(id INTEGER PRIMARY KEY,{},_archive_generation INTEGER HIDDEN)",
                declaration()
            ),
            Self {
                base: Default::default(),
                database,
                storage,
            },
        ))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<()> {
        let mut constraints = Vec::new();
        let mut point = false;
        let mut frame = false;
        for (c, mut usage) in info.constraints_and_usages() {
            if !c.is_usable() {
                continue;
            }
            let column = c.column();
            let op = match c.operator() {
                IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ => "=",
                IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_GT => ">",
                IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_GE => ">=",
                IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_LT => "<",
                IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_LE => "<=",
                _ => continue,
            };
            if column == -1 || column == 0 || (column == 1 && op == "=") {
                point |= column <= 0 && op == "=";
                frame |= column == 1;
                constraints.push(format!("{}:{op}", if column == 1 { "frame" } else { "id" }));
                usage.set_argv_index(constraints.len() as i32);
            }
        }
        let ordering: Vec<_> = info
            .order_bys()
            .map(|o| (o.column(), o.is_order_by_desc()))
            .collect();
        let desc = ordering.len() == 1 && ordering[0].0 <= 0 && ordering[0].1;
        if ordering.len() == 1 && ordering[0].0 <= 0 {
            info.set_order_by_consumed(true);
        }
        info.set_idx_num(i32::from(desc));
        info.set_idx_str(&constraints.join(","));
        info.set_estimated_rows(if point {
            1
        } else if frame {
            200
        } else {
            20_000_000
        });
        info.set_estimated_cost(if point {
            10.0
        } else if frame {
            200.0
        } else {
            20_000_000.0
        });
        if point {
            info.set_idx_flags(IndexFlags::SQLITE_INDEX_SCAN_UNIQUE);
        }
        Ok(())
    }
    fn open(&mut self) -> Result<Cursor> {
        Ok(Cursor {
            base: Default::default(),
            db: unsafe { Connection::from_handle(self.database.handle())? },
            storage: self.storage.clone(),
            files_sql: String::new(),
            files_args: Vec::new(),
            files_after: None,
            files_done: false,
            archive: None,
            archive_location: None,
            archive_position: 0,
            archive_indices: Vec::new(),
            lower: i64::MIN,
            upper: i64::MAX,
            frame: None,
            pending: VecDeque::new(),
            pending_sql: String::new(),
            pending_args: Vec::new(),
            pending_after: None,
            pending_done: false,
            descending: false,
            current: None,
        })
    }
}
impl CreateVTab<'_> for Elements {
    const KIND: VTabKind = VTabKind::Default;
}

impl Elements {
    fn write(&self, id: i64, mut row: Option<Record>, inserting: bool) -> Result<i64> {
        let db = &self.database;
        let old = lookup(&db, &self.storage, id)?;
        if inserting && old.is_some() {
            return Err(constraint("element ID already exists"));
        }
        if !inserting && old.is_none() {
            return Err(constraint("element ID is absent"));
        }
        let file = location(&db, id)?.map(|r| r.0);
        if let Some(new) = row.as_mut() {
            // Affinities are explicit for virtual tables; identity, required
            // fields, and relationships have the same accepted value types.
            for (i, column) in TABLE.columns.iter().enumerate() {
                use crate::storage::bulk::Kind;
                let current = std::mem::replace(&mut new.values[i], Value::Null);
                new.values[i] = match (column.kind, current) {
                    (_, Value::Null) => Value::Null,
                    (Kind::Text, value @ Value::Text(_))
                    | (Kind::Integer, value @ Value::Integer(_))
                    | (Kind::Real, value @ Value::Real(_)) => value,
                    (Kind::Text, Value::Integer(value)) => Value::Text(value.to_string()),
                    (Kind::Text, Value::Real(value)) => Value::Text(db.cached_query_row(
                        "SELECT CAST(? AS TEXT)",
                        [value],
                        |r| r.get(0),
                    )?),
                    (Kind::Real, Value::Integer(value)) => Value::Real(value as f64),
                    (Kind::Integer, Value::Text(value)) => Value::Integer(
                        value
                            .trim()
                            .parse()
                            .map_err(|_| constraint("element integer affinity mismatch"))?,
                    ),
                    (Kind::Real, Value::Text(value)) => Value::Real(
                        value
                            .trim()
                            .parse()
                            .map_err(|_| constraint("element real affinity mismatch"))?,
                    ),
                    (Kind::Integer, Value::Real(value))
                        if value.is_finite()
                            && value.fract() == 0.0
                            && value >= i64::MIN as f64
                            && value < -(i64::MIN as f64) =>
                    {
                        Value::Integer(value as i64)
                    }
                    _ => return Err(constraint("element column type mismatch")),
                };
            }
            for index in [0, 1, 2, 5, 11] {
                if new.values[index] == Value::Null {
                    return Err(constraint("required element column is NULL"));
                }
            }
            if new.bytes() > self.storage.descriptor.budget.record_bytes {
                return Err(constraint("element record budget exceeded"));
            }
            let Value::Integer(frame) = new.values[0] else {
                return Err(constraint("invalid element frame"));
            };
            if !db.cached_query_row(
                "SELECT EXISTS(SELECT 1 FROM frames WHERE id=?)",
                [frame],
                |r| r.get::<_, bool>(0),
            )? {
                return Err(constraint("element frame is absent"));
            }
            if let Value::Integer(parent) = new.values[4] {
                db.cached_execute(
                    "INSERT OR IGNORE INTO _bulk_element_checks VALUES(?)",
                    [parent],
                )?;
            }
            new.generation = old.as_ref().map_or(1, |r| r.generation + 1);
        }
        if row.is_none() {
            db.cached_execute("INSERT OR IGNORE INTO _bulk_element_checks VALUES(?)", [id])?;
        }
        let resident: i64 = if inserting {
            0
        } else {
            db.cached_query_row(&ROW_SQL.resident, [id], |r| r.get(0))?
        };
        // Frame references enforce the ordinary frame FK. Parent references
        // validate the complete transaction, including multi-row tree deletes.
        for (record, delta) in [(old.as_ref(), -1), (row.as_ref(), 1)] {
            if let Some(record) = record {
                let visibility = match record.values[13] {
                    Value::Integer(value) => value,
                    _ => 0,
                };
                let is_null = i64::from(record.values[13] == Value::Null);
                db.cached_execute(
                    "INSERT OR IGNORE INTO _bulk_element_kinds(source,role) VALUES(?,?)",
                    params![sql_value(&record.values[1]), sql_value(&record.values[2])],
                )?;
                let kind: i64 = db.cached_query_row(
                    "SELECT id FROM _bulk_element_kinds WHERE source=? AND role=?",
                    params![sql_value(&record.values[1]), sql_value(&record.values[2])],
                    |r| r.get(0),
                )?;
                db.cached_execute("INSERT INTO _bulk_element_groups VALUES(?1,?2,?3,?4,?5) ON CONFLICT(frame_id,kind_id,visibility,is_null) DO UPDATE SET rows=rows+excluded.rows",params![sql_value(&record.values[0]),kind,visibility,is_null,delta])?;
                if delta < 0 {
                    db.cached_execute("DELETE FROM _bulk_element_groups WHERE frame_id=?1 AND kind_id=?2 AND visibility=?3 AND is_null=?4 AND rows=0",params![sql_value(&record.values[0]),kind,visibility,is_null])?;
                }
                for (index, table, key) in [(4, "_bulk_element_parent_refs", "parent_id")] {
                    if let Value::Integer(value) = record.values[index] {
                        db.cached_execute(&format!("INSERT INTO {table}({key},rows) VALUES(?1,?2) ON CONFLICT({key}) DO UPDATE SET rows=rows+excluded.rows"),params![value,delta])?;
                        if delta < 0 {
                            db.cached_execute(
                                &format!("DELETE FROM {table} WHERE {key}=? AND rows=0"),
                                [value],
                            )?;
                        }
                    }
                }
            }
        }
        if let Some(old) = &old {
            if matches!(&old.values[3], Value::Text(text) if !text.is_empty()) {
                db.cached_execute("INSERT INTO elements_fts(elements_fts,rowid,text,role,frame_id) VALUES('delete',?,?,?,?)", params![id,sql_value(&old.values[3]),sql_value(&old.values[2]),sql_value(&old.values[0])])?;
            }
        }
        let new_bytes = row.as_ref().map_or(0, Record::bytes) as i64;
        let generation = row
            .as_ref()
            .map_or_else(|| old.as_ref().unwrap().generation + 1, |r| r.generation);
        let deleted = row.is_none();
        let mut args = vec![
            SqlValue::Integer(id),
            SqlValue::Integer(generation),
            SqlValue::Integer(i64::from(deleted)),
            file.map_or(SqlValue::Null, SqlValue::Integer),
        ];
        if let Some(row) = &row {
            args.extend(row.values.iter().map(sql_value));
        } else {
            args.resize(TABLE.columns.len() + 4, SqlValue::Null);
        }
        db.cached_execute(&ROW_SQL.upsert, params_from_iter(args))?;
        if let Some(row) = row {
            if matches!(&row.values[3],Value::Text(t) if !t.is_empty()) {
                db.cached_execute(
                    "INSERT INTO elements_fts(rowid,text,role,frame_id) VALUES(?,?,?,?)",
                    params![
                        id,
                        sql_value(&row.values[3]),
                        sql_value(&row.values[2]),
                        sql_value(&row.values[0])
                    ],
                )?;
            }
        }
        db.cached_execute(
            "UPDATE storage_metadata SET staging_bytes=staging_bytes-?1+?2,revision=revision+1",
            params![resident, new_bytes],
        )?;
        db.cached_execute("UPDATE _bulk_element_state SET version=version+1", [])?;
        if let Some(file) = file {
            db.cached_execute("UPDATE _bulk_files SET state='dirty' WHERE id=?", [file])?;
        }
        if inserting {
            db.cached_execute("INSERT INTO sqlite_sequence(name,seq) SELECT 'elements',0 WHERE NOT EXISTS(SELECT 1 FROM sqlite_sequence WHERE name='elements')",[])?;
            db.cached_execute(
                "UPDATE sqlite_sequence SET seq=max(seq,?) WHERE name='elements'",
                [id],
            )?;
        }
        Ok(id)
    }
    fn record(&self, args: &Values<'_>, id: i64) -> Result<Record> {
        Ok(Record {
            id,
            generation: 1,
            values: (0..TABLE.columns.len())
                .map(|i| value(args.get(i + 3)?))
                .collect::<Result<Vec<_>>>()?,
        })
    }
}
impl UpdateVTab<'_> for Elements {
    fn delete(&mut self, arg: ValueRef<'_>) -> Result<()> {
        self.write(arg.as_i64()?, None, false).map(|_| ())
    }
    fn insert(&mut self, args: &Inserts<'_>) -> Result<i64> {
        let db = &self.database;
        let explicit: Option<i64> = args.get(2)?;
        let rowid: Option<i64> = args.get(1)?;
        if explicit.is_some() && rowid.is_some() && explicit != rowid {
            return Err(constraint("element rowid and ID differ"));
        }
        let id = if let Some(id) = explicit.or(rowid) {
            id
        } else {
            let seq: i64 = db.cached_query_row(
                "SELECT COALESCE((SELECT seq FROM sqlite_sequence WHERE name='elements'),0)",
                [],
                |r| r.get(0),
            )?;
            seq.checked_add(1)
                .ok_or_else(|| constraint("element IDs exhausted"))?
        };
        let mut row = self.record(args, id)?;
        for index in [5, 11] {
            if row.values[index] == Value::Null {
                row.values[index] = Value::Integer(0);
            }
        }
        self.write(id, Some(row), true)
    }
    fn update(&mut self, args: &Updates<'_>) -> Result<()> {
        let id: i64 = args.get(0)?;
        if args.get::<i64>(1)? != id || args.get::<i64>(2)? != id {
            return Err(constraint("element IDs are immutable"));
        }
        self.write(id, Some(self.record(args, id)?), false)
            .map(|_| ())
    }
}
impl TransactionVTab<'_> for Elements {
    fn sync(&mut self) -> Result<()> {
        let result = (|| {
            let db = &self.database;
            {
                let mut checks = db.prepare("SELECT c.parent_id FROM _bulk_element_checks c JOIN _bulk_element_parent_refs p ON p.parent_id=c.parent_id WHERE p.rows>0")?;
                let mut rows = checks.query([])?;
                while let Some(row) = rows.next()? {
                    if lookup(&db, &self.storage, row.get(0)?)?.is_none() {
                        return Err(constraint("element parent is absent"));
                    }
                }
            }
            db.cached_execute("DELETE FROM _bulk_element_checks", [])?;
            Ok(())
        })();
        self.database.flush_prepared_statement_cache();
        result
    }
    fn commit(&mut self) -> Result<()> {
        self.database.flush_prepared_statement_cache();
        Ok(())
    }
    fn rollback(&mut self) -> Result<()> {
        self.database.flush_prepared_statement_cache();
        Ok(())
    }
}

#[repr(C)]
struct Cursor {
    base: ffi::sqlite3_vtab_cursor,
    db: Connection,
    storage: Arc<HybridStorage>,
    files_sql: String,
    files_args: Vec<SqlValue>,
    files_after: Option<i64>,
    files_done: bool,
    archive: Option<Arc<crate::storage::bulk::element_index::ElementIndex>>,
    archive_location: Option<(String, String)>,
    archive_position: usize,
    archive_indices: Vec<usize>,
    lower: i64,
    upper: i64,
    frame: Option<i64>,
    pending: VecDeque<(Record, bool)>,
    pending_sql: String,
    pending_args: Vec<SqlValue>,
    pending_after: Option<i64>,
    pending_done: bool,
    descending: bool,
    current: Option<CursorRow>,
}

enum CursorRow {
    Staged(Record),
    Archived(usize),
}
impl Cursor {
    fn current_id(&self) -> Result<i64> {
        match self.current.as_ref() {
            Some(CursorRow::Staged(row)) => Ok(row.id),
            Some(CursorRow::Archived(index)) => Ok(self.archive.as_ref().unwrap().rows[*index].id),
            None => Err(failure("element cursor exhausted")),
        }
    }
    fn fill_pending(&mut self) -> Result<()> {
        if !self.pending.is_empty() || self.pending_done {
            return Ok(());
        }
        let after = self.pending_after.map_or(String::new(), |_| {
            format!(" AND id{}?", if self.descending { "<" } else { ">" })
        });
        let mut args = self.pending_args.clone();
        if let Some(id) = self.pending_after {
            args.push(SqlValue::Integer(id));
        }
        let sql = format!(
            "SELECT {} FROM _bulk_element_rows WHERE {}{after} ORDER BY id {} LIMIT 128",
            ROW_SQL.projection,
            self.pending_sql,
            if self.descending { "DESC" } else { "ASC" }
        );
        let mut stmt = self.db.prepare_cached(&sql)?;
        let mut rows = stmt.query(params_from_iter(args))?;
        let mut bytes = 0;
        while let Some(row) = rows.next()? {
            let row = staged(row)?;
            let size = row.0.bytes();
            if !self.pending.is_empty() && bytes + size > self.storage.descriptor.budget.file_bytes
            {
                break;
            }
            bytes += size;
            self.pending_after = Some(row.0.id);
            self.pending.push_back(row);
        }
        if self.pending.is_empty() {
            self.pending_done = true;
        }
        Ok(())
    }
    fn fill_archive(&mut self) -> Result<()> {
        if self
            .archive
            .as_ref()
            .is_some_and(|_| self.archive_position < self.archive_indices.len())
        {
            return Ok(());
        }
        self.archive = None;
        while !self.files_done {
            let mut args = self.files_args.clone();
            let after = if let Some(id) = self.files_after {
                args.push(SqlValue::Integer(id));
                format!(
                    " AND r.first_id{}?",
                    if self.descending { "<" } else { ">" }
                )
            } else {
                String::new()
            };
            let sql = format!(
                "{}{after} ORDER BY r.first_id {} LIMIT 1",
                self.files_sql,
                if self.descending { "DESC" } else { "ASC" }
            );
            let file: Option<(i64, String, String)> = self
                .db
                .prepare_cached(&sql)?
                .query_row(params_from_iter(args), |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .optional()?;
            let Some((first_id, path, hash)) = file else {
                self.files_done = true;
                break;
            };
            self.files_after = Some(first_id);
            let rows = self
                .storage
                .element_index(&path, &hash)
                .map_err(|_| failure("element archive unavailable"))?;
            self.archive_position = 0;
            let begin = rows.rows.partition_point(|r| r.id < self.lower);
            let end = rows.rows.partition_point(|r| r.id <= self.upper);
            self.archive_indices = if let Some(frame) = self.frame {
                let frames = self
                    .storage
                    .element_frame_rows(format!("{path}:{hash}"), &rows.rows)
                    .map_err(|_| failure("element frame index unavailable"))?;
                frames
                    .get(&frame)
                    .map(|ids| {
                        ids.iter()
                            .copied()
                            .filter(|i| *i >= begin && *i < end)
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                (begin..end).collect()
            };
            if self.descending {
                self.archive_indices.reverse();
            }
            if !self.archive_indices.is_empty() {
                self.archive = Some(rows);
                self.archive_location = Some((path, hash));
                break;
            }
        }
        Ok(())
    }
    fn advance(&mut self) -> Result<()> {
        self.current = None;
        loop {
            self.fill_pending()?;
            self.fill_archive()?;
            let archived = self
                .archive
                .as_ref()
                .map(|r| &r.rows[self.archive_indices[self.archive_position]]);
            let pending = self.pending.front();
            let take_pending = match (pending, archived) {
                (None, None) => return Ok(()),
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (Some((p, _)), Some(a)) => {
                    if self.descending {
                        p.id >= a.id
                    } else {
                        p.id <= a.id
                    }
                }
            };
            if take_pending {
                let (row, deleted) = self.pending.pop_front().unwrap();
                if archived.is_some_and(|a| a.id == row.id) {
                    self.archive_position += 1;
                }
                if deleted {
                    continue;
                }
                self.current = Some(CursorRow::Staged(row));
            } else {
                self.current = Some(CursorRow::Archived(
                    self.archive_indices[self.archive_position],
                ));
                self.archive_position += 1;
            }
            return Ok(());
        }
    }
}
unsafe impl VTabCursor for Cursor {
    fn filter(&mut self, idx: i32, description: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.descending = idx != 0;
        self.lower = i64::MIN;
        self.upper = i64::MAX;
        self.frame = None;
        self.pending.clear();
        self.pending_after = None;
        self.pending_done = false;
        self.files_after = None;
        self.files_done = false;
        self.archive = None;
        self.archive_location = None;
        self.current = None;
        let mut range = Vec::new();
        let mut pending = Vec::new();
        let mut range_args = Vec::new();
        let mut pending_args = Vec::new();
        let mut frame = None;
        for (i, item) in description
            .unwrap_or("")
            .split(',')
            .filter(|s| !s.is_empty())
            .enumerate()
        {
            let (column, op) = item
                .split_once(':')
                .ok_or_else(|| failure("invalid element query plan"))?;
            let value: SqlValue = args.get(i)?;
            if column == "frame" {
                if let SqlValue::Integer(id) = &value {
                    self.frame = Some(*id);
                }
                frame = Some(value);
                continue;
            }
            if let SqlValue::Integer(id) = &value {
                match op {
                    "=" => {
                        self.lower = self.lower.max(*id);
                        self.upper = self.upper.min(*id);
                    }
                    ">" => self.lower = self.lower.max(id.saturating_add(1)),
                    ">=" => self.lower = self.lower.max(*id),
                    "<" => self.upper = self.upper.min(id.saturating_sub(1)),
                    "<=" => self.upper = self.upper.min(*id),
                    _ => (),
                }
            }
            let condition = match op {
                "=" => "r.first_id=(SELECT max(first_id) FROM _bulk_element_ranges WHERE first_id<=?) AND r.last_id>=?",
                ">" | ">=" => "r.first_id>=COALESCE((SELECT max(first_id) FROM _bulk_element_ranges WHERE first_id<=?),?) AND r.last_id>=?",
                "<" | "<=" => "r.first_id<=?",
                _ => return Err(failure("invalid element range")),
            };
            range.push(condition.to_owned());
            range_args.push(value.clone());
            if op == "=" {
                range_args.push(value.clone());
            } else if matches!(op, ">" | ">=") {
                range_args.extend([value.clone(), value.clone()]);
            }
            pending.push(format!("id{op}?"));
            pending_args.push(value);
        }
        if let Some(frame) = &frame {
            range.push("f.frame_id=?".into());
            range_args.push(frame.clone());
        }
        let where_sql = if range.is_empty() {
            "1".into()
        } else {
            range.join(" AND ")
        };
        let frame_join = if frame.is_some() {
            "JOIN _bulk_element_frames f ON f.file_id=r.file_id"
        } else {
            ""
        };
        self.files_sql=format!("SELECT r.first_id,p.path,p.checksum FROM _bulk_element_ranges r JOIN _bulk_files p ON p.id=r.file_id {frame_join} WHERE {where_sql}");
        self.files_args = range_args;
        if let Some(frame) = frame {
            pending.push("(frame_id=? OR _archive_file IN (SELECT file_id FROM _bulk_element_frames WHERE frame_id=?))".into());
            pending_args.extend([frame.clone(), frame]);
        }
        self.pending_sql = if pending.is_empty() {
            "1".into()
        } else {
            pending.join(" AND ")
        };
        self.pending_args = pending_args;
        self.advance()
    }
    fn next(&mut self) -> Result<()> {
        self.advance()
    }
    fn eof(&self) -> bool {
        self.current.is_none()
    }
    fn column(&self, ctx: &mut Context, index: i32) -> Result<()> {
        let current = self
            .current
            .as_ref()
            .ok_or_else(|| failure("element cursor exhausted"))?;
        let full;
        let row = match current {
            CursorRow::Staged(row) => row,
            CursorRow::Archived(position) => {
                let archive = self.archive.as_ref().unwrap();
                let row = &archive.rows[*position];
                match index {
                    0 => return ctx.set_result(&row.id),
                    1 => return ctx.set_result(&row.frame),
                    2 => return ctx.set_result(&archive.sources[row.source as usize]),
                    3 => return ctx.set_result(&archive.roles[row.role as usize]),
                    12 => return ctx.set_result(&row.sort),
                    14 => return ctx.set_result(&(row.present & 1 != 0).then_some(row.visible)),
                    15 => return ctx.set_result(&(row.present & 2 != 0).then_some(row.redacted)),
                    16 => return ctx.set_result(&row.generation),
                    _ => (),
                }
                let (path, hash) = self.archive_location.as_ref().unwrap();
                full = self
                    .storage
                    .bulk_records(path, hash, 0)
                    .map_err(|_| failure("element archive unavailable"))?;
                let position = full
                    .binary_search_by_key(&row.id, |r| r.id)
                    .map_err(|_| failure("element row is absent"))?;
                &full[position]
            }
        };
        if index == 0 {
            ctx.set_result(&row.id)
        } else if index as usize == TABLE.columns.len() + 1 {
            ctx.set_result(&row.generation)
        } else {
            ctx.set_result(&sql_value(&row.values[index as usize - 1]))
        }
    }
    fn rowid(&self) -> Result<i64> {
        self.current_id()
    }
}
