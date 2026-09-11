// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use super::storage_error;
use std::ffi::{c_char, c_int, c_void, CStr, CString};

unsafe extern "C" fn resident_columns(
    context: *mut c_void,
    action: c_int,
    table: *const c_char,
    column: *const c_char,
    _: *const c_char,
    _: *const c_char,
) -> c_int {
    if action == libsqlite3_sys::SQLITE_READ && !table.is_null() && !column.is_null() {
        // SAFETY: SQLite owns both null-terminated names for this callback.
        let (table, column) = unsafe {
            (
                CStr::from_ptr(table).to_bytes(),
                CStr::from_ptr(column).to_bytes(),
            )
        };
        if table.eq_ignore_ascii_case(b"frames")
            && [
                b"full_text".as_slice(),
                b"accessibility_text",
                b"accessibility_tree_json",
                b"text_json",
            ]
            .iter()
            .any(|name| column.eq_ignore_ascii_case(name))
        {
            unsafe {
                *context.cast::<bool>() = true;
            }
            return libsqlite3_sys::SQLITE_DENY;
        }
    }
    libsqlite3_sys::SQLITE_OK
}

/// Preparing with SQLite's authorizer resolves aliases, wildcard expansion,
/// subqueries and views against the actual resident schema.
pub(crate) async fn validate_resident_query(
    pool: &sqlx::SqlitePool,
    query: &str,
) -> Result<(), sqlx::Error> {
    let mut conn = pool.acquire().await?;
    let mut handle = conn.lock_handle().await?;
    let query = CString::new(query).map_err(storage_error)?;
    // SAFETY: the connection is exclusively locked throughout registration,
    // preparation, finalization and callback removal. No callback escapes.
    unsafe {
        let db = handle.as_raw_handle().as_ptr();
        let mut denied = false;
        libsqlite3_sys::sqlite3_set_authorizer(
            db,
            Some(resident_columns),
            (&mut denied as *mut bool).cast(),
        );
        let mut next = query.as_ptr();
        while *next != 0 {
            let mut statement = std::ptr::null_mut();
            let mut tail = std::ptr::null();
            let result =
                libsqlite3_sys::sqlite3_prepare_v2(db, next, -1, &mut statement, &mut tail);
            libsqlite3_sys::sqlite3_finalize(statement);
            if result != libsqlite3_sys::SQLITE_OK || tail == next {
                break;
            }
            next = tail;
        }
        libsqlite3_sys::sqlite3_set_authorizer(db, None, std::ptr::null_mut());
        if denied {
            return Err(storage_error("unsupported-storage-query: archived frame payloads are available through typed search and frame endpoints"));
        }
        // Normal execution reports syntax and schema errors in the existing format.
    }
    Ok(())
}
