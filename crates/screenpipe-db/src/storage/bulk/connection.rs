// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use super::{Value, TABLES};
use crate::storage::{storage_error, HybridStorage};
use libsqlite3_sys as ffi;
use sqlx::{sqlite::SqlitePoolOptions, SqliteConnection};
use std::{
    collections::HashSet,
    ffi::{c_int, c_void, CStr},
    sync::{atomic::Ordering, Arc, Mutex},
};

#[derive(Default)]
struct Statements {
    active: HashSet<usize>,
    lease: Option<StatementLease>,
}

struct StatementLease(Arc<HybridStorage>);

impl StatementLease {
    fn new(storage: &Arc<HybridStorage>) -> Self {
        storage.bulk.statements.fetch_add(1, Ordering::SeqCst);
        Self(Arc::clone(storage))
    }
}

impl Drop for StatementLease {
    fn drop(&mut self) {
        self.0.bulk.statements.fetch_sub(1, Ordering::SeqCst);
    }
}

impl HybridStorage {
    /// Called after retiring locators under the writer permit. A snapshot
    /// admitted after this cutoff sees current files; existing snapshots pin
    /// their old files through statement completion or transaction end.
    pub(crate) fn sql_readers_active(&self) -> bool {
        self.bulk.statements.load(Ordering::SeqCst) != 0
    }
}

struct Context {
    storage: Arc<HybridStorage>,
    database: usize,
    statements: Mutex<Statements>,
}

pub(crate) fn pool_options(
    storage: Option<Arc<HybridStorage>>,
    logical: bool,
) -> SqlitePoolOptions {
    let options = crate::write_queue::capture_pool_options();
    let Some(storage) = storage.filter(|s| s.has_bulk()) else {
        return options;
    };
    options.after_connect(move |conn, _| {
        let storage = Arc::clone(&storage);
        Box::pin(async move { register(conn, storage, logical).await })
    })
}

async fn register(
    conn: &mut SqliteConnection,
    storage: Arc<HybridStorage>,
    logical: bool,
) -> Result<(), sqlx::Error> {
    register_hash(conn).await?;
    super::elements::register(conn, Arc::clone(&storage)).await?;
    {
        let mut locked = conn.lock_handle().await?;
        let db = locked.as_raw_handle().as_ptr();
        let context = Box::into_raw(Box::new(Context {
            storage,
            database: db as usize,
            statements: Mutex::new(Statements::default()),
        }));
        // SAFETY: SQLite owns the boxed context after registration. Its function
        // destructor disables the trace hook before freeing the shared context.
        unsafe {
            let result = ffi::sqlite3_create_function_v2(
                db,
                c"screenpipe_bulk".as_ptr(),
                5,
                ffi::SQLITE_UTF8,
                context.cast(),
                Some(value),
                None,
                None,
                Some(destroy),
            );
            if result != ffi::SQLITE_OK {
                return Err(storage_error("cannot register Parquet reader"));
            }
            if ffi::sqlite3_trace_v2(
                db,
                (ffi::SQLITE_TRACE_STMT | ffi::SQLITE_TRACE_PROFILE) as u32,
                Some(trace),
                context.cast(),
            ) != ffi::SQLITE_OK
            {
                return Err(storage_error("cannot install payload statement leases"));
            }
        }
    }
    if logical {
        let query_only: i64 = sqlx::query_scalar("PRAGMA query_only")
            .fetch_one(&mut *conn)
            .await?;
        sqlx::query("PRAGMA query_only=OFF")
            .execute(&mut *conn)
            .await?;
        let result = async {
            for table in TABLES.iter().filter(|t| t.name != "elements") {
                // SQLite resolves the target when the view is queried, after
                // bootstrap has created this generation's main-schema views.
                sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                    "CREATE TEMP VIEW {} AS SELECT * FROM main.{}",
                    table.name,
                    table.view()
                )))
                .execute(&mut *conn)
                .await?;
            }
            Ok::<_, sqlx::Error>(())
        }
        .await;
        if query_only != 0 {
            sqlx::query("PRAGMA query_only=ON")
                .execute(&mut *conn)
                .await?;
        }
        result?;
    }
    Ok(())
}

pub(crate) async fn register_hash(conn: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    let mut locked = conn.lock_handle().await?;
    // SAFETY: the callback borrows only SQLite's argument and copies its result.
    let result = unsafe {
        ffi::sqlite3_create_function_v2(
            locked.as_raw_handle().as_ptr(),
            c"screenpipe_payload_sha256".as_ptr(),
            1,
            ffi::SQLITE_UTF8 | ffi::SQLITE_DETERMINISTIC | ffi::SQLITE_INNOCUOUS,
            std::ptr::null_mut(),
            Some(hash),
            None,
            None,
            None,
        )
    };
    if result != ffi::SQLITE_OK {
        return Err(storage_error("cannot register payload identity function"));
    }
    Ok(())
}

unsafe extern "C" fn hash(
    output: *mut ffi::sqlite3_context,
    _: c_int,
    args: *mut *mut ffi::sqlite3_value,
) {
    use sha2::{Digest, Sha256};
    let bytes = ffi::sqlite3_value_blob(*args);
    let len = ffi::sqlite3_value_bytes(*args);
    let bytes = if len == 0 {
        &[]
    } else if bytes.is_null() {
        ffi::sqlite3_result_error_nomem(output);
        return;
    } else {
        std::slice::from_raw_parts(bytes.cast::<u8>(), len as usize)
    };
    let digest = Sha256::digest(bytes);
    ffi::sqlite3_result_blob(
        output,
        digest.as_ptr().cast(),
        digest.len() as c_int,
        ffi::SQLITE_TRANSIENT(),
    );
}

unsafe extern "C" fn destroy(pointer: *mut c_void) {
    let context = Box::from_raw(pointer.cast::<Context>());
    ffi::sqlite3_trace_v2(
        context.database as *mut ffi::sqlite3,
        0,
        None,
        std::ptr::null_mut(),
    );
    drop(context);
}

unsafe extern "C" fn trace(
    event: u32,
    pointer: *mut c_void,
    statement: *mut c_void,
    _: *mut c_void,
) -> c_int {
    let context = &*pointer.cast::<Context>();
    // Pin before SQLite opens a snapshot, including nested statements during
    // preparation. Admission is independent of file unlinking: new snapshots
    // see committed current locators while older snapshots defer reclamation.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut state = context.statements.lock().map_err(storage_error)?;
        if event == ffi::SQLITE_TRACE_STMT as u32 {
            if state.lease.is_none() {
                state.lease = Some(StatementLease::new(&context.storage));
            }
            state.active.insert(statement as usize);
        } else if event == ffi::SQLITE_TRACE_PROFILE as u32 {
            state.active.remove(&(statement as usize));
            if state.active.is_empty()
                && ffi::sqlite3_get_autocommit(context.database as *mut ffi::sqlite3) != 0
            {
                state.lease = None;
            }
        }
        Ok::<_, sqlx::Error>(())
    }));
    if !matches!(result, Ok(Ok(()))) {
        ffi::sqlite3_interrupt(context.database as *mut ffi::sqlite3);
    }
    0
}

unsafe extern "C" fn value(
    output: *mut ffi::sqlite3_context,
    count: c_int,
    arguments: *mut *mut ffi::sqlite3_value,
) {
    let context = &*ffi::sqlite3_user_data(output).cast::<Context>();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if count != 5 {
            return Err(storage_error("invalid bulk reader arguments"));
        }
        let args = std::slice::from_raw_parts(arguments, 5);
        let string = |index: usize| -> Result<&str, sqlx::Error> {
            let text = ffi::sqlite3_value_text(args[index]);
            if text.is_null() {
                return Err(storage_error("missing bulk file location"));
            }
            CStr::from_ptr(text.cast()).to_str().map_err(storage_error)
        };
        let path = string(0)?;
        let hash = string(1)?;
        let table = usize::try_from(ffi::sqlite3_value_int64(args[2])).map_err(storage_error)?;
        let id = ffi::sqlite3_value_int64(args[3]);
        let column = usize::try_from(ffi::sqlite3_value_int64(args[4])).map_err(storage_error)?;
        let rows = context.storage.bulk_records(path, hash, table)?;
        let position = rows
            .binary_search_by_key(&id, |r| r.id)
            .map_err(|_| storage_error("bulk row is absent"))?;
        let value = rows[position]
            .values
            .get(column)
            .ok_or_else(|| storage_error("bulk column is absent"))?;
        match value {
            Value::Null => ffi::sqlite3_result_null(output),
            Value::Text(s) => ffi::sqlite3_result_text64(
                output,
                s.as_ptr().cast(),
                s.len() as u64,
                ffi::SQLITE_TRANSIENT(),
                ffi::SQLITE_UTF8 as u8,
            ),
            Value::Integer(i) => ffi::sqlite3_result_int64(output, *i),
            Value::Real(f) => ffi::sqlite3_result_double(output, *f),
        }
        Ok::<_, sqlx::Error>(())
    }));
    if !matches!(result, Ok(Ok(()))) {
        // Captured strings and paths never enter SQLite diagnostics.
        ffi::sqlite3_result_error(
            output,
            c"Parquet payload unavailable or invalid".as_ptr(),
            -1,
        );
    }
}
