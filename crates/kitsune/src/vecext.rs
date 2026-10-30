//! sqlite-vec, the vector search that lives inside the same SQLite file (vec0 tables, KNN by
//! cosine). Loaded into each connection the store opens; one C file, compiled in, static-build
//! clean.

use crate::{Error, Result};
use rusqlite::Connection;

type Init = unsafe extern "C" fn(
    *mut rusqlite::ffi::sqlite3,
    *mut *mut std::os::raw::c_char,
    *const rusqlite::ffi::sqlite3_api_routines,
) -> std::os::raw::c_int;

/// Make vec0 and the vec_* functions known to this connection (once; a second call is a no-op).
pub fn load(conn: &Connection) -> Result<()> {
    if conn
        .query_row("SELECT vec_version()", [], |r| r.get::<_, String>(0))
        .is_ok()
    {
        return Ok(());
    }
    // SAFETY: sqlite3_vec_init is sqlite-vec's documented extension entry point, called with this
    // connection's live handle; it only registers functions and the vec0 module on it.
    let rc = unsafe {
        let init: Init = std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ());
        init(conn.handle(), std::ptr::null_mut(), std::ptr::null())
    };
    if rc != 0 {
        return Err(Error::Invalid(format!(
            "sqlite-vec did not load (code {rc})"
        )));
    }
    Ok(())
}
