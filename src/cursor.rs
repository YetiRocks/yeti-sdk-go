//! Scan-cursor exports: open a paged scan over a server-held resource, pull
//! pages, and release it. The cursor is a connection-scoped resource the peer
//! holds; this binding keeps the local half in the [`cursors`] registry and
//! takes it out / puts it back across each `next` (its `next` is `&mut self`).

use std::ffi::c_char;

use crate::ffi::{YetiEntries, YetiStatus};
use crate::registry::{cursors, runtime, store_of};
use crate::util::{bad_handle, fail, str_arg};

/// Open a paged scan cursor over `(table, prefix)`; writes the cursor handle
/// into `out`.
///
/// # Safety
/// String args are NUL-terminated; `out`/`err` valid.
#[no_mangle]
pub unsafe extern "C" fn yeti_store_open_scan(
    client: u64,
    table: *const c_char,
    prefix: *const c_char,
    out: *mut u64,
    err: *mut *mut c_char,
) -> YetiStatus {
    let Some(store) = store_of(client) else {
        return bad_handle(err);
    };
    match runtime().block_on(store.open_scan(str_arg(table), str_arg(prefix))) {
        Ok(cursor) => {
            *out = cursors().insert(cursor);
            YetiStatus::Ok
        }
        Err(e) => fail(&e, err),
    }
}

/// Pull the next page (up to `max` entries) from a cursor. Writes the page
/// into `out`; when the cursor is exhausted `done` is set true and `out` is an
/// empty [`YetiEntries`].
///
/// # Safety
/// `cursor` is a handle from [`yeti_store_open_scan`]; `out`/`done`/`err`
/// valid (or `err` null).
#[no_mangle]
pub unsafe extern "C" fn yeti_cursor_next(
    cursor: u64,
    max: u32,
    out: *mut YetiEntries,
    done: *mut bool,
    err: *mut *mut c_char,
) -> YetiStatus {
    // Take the cursor out so we can mutate it, then put it back (unless the
    // call drives it to exhaustion, in which case we drop it).
    let mut c = match cursors().remove(cursor) {
        Some(c) => c,
        None => return bad_handle(err),
    };
    let result = runtime().block_on(c.next(max));
    match result {
        Ok(Some(page)) => {
            *out = YetiEntries::from_entries(page);
            *done = false;
            // Reinsert under the SAME handle so Go's handle stays valid.
            cursors().map.lock().unwrap().insert(cursor, c);
            YetiStatus::Ok
        }
        Ok(None) => {
            *out = YetiEntries::empty();
            *done = true;
            // Exhausted: server released it; drop our side (c goes out of scope).
            YetiStatus::Ok
        }
        Err(e) => {
            cursors().map.lock().unwrap().insert(cursor, c);
            fail(&e, err)
        }
    }
}

/// Release a cursor's server-side resource early and free its handle
/// (idempotent).
///
/// # Safety
/// `cursor` is a handle from [`yeti_store_open_scan`] (or already freed).
#[no_mangle]
pub unsafe extern "C" fn yeti_cursor_close(cursor: u64, err: *mut *mut c_char) -> YetiStatus {
    let Some(mut c) = cursors().remove(cursor) else {
        return YetiStatus::Ok;
    };
    match runtime().block_on(c.close()) {
        Ok(()) => YetiStatus::Ok,
        Err(e) => fail(&e, err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_next_on_dead_handle_is_bad_handle() {
        let mut out = YetiEntries::empty();
        let mut done = false;
        let mut err: *mut c_char = std::ptr::null_mut();
        let status = unsafe { yeti_cursor_next(424_242, 8, &mut out, &mut done, &mut err) };
        assert!(matches!(status, YetiStatus::BadHandle));
        assert!(!err.is_null());
        unsafe { crate::memory::yeti_string_free(err) };
    }

    #[test]
    fn cursor_close_on_unknown_handle_is_ok() {
        // Closing an already-freed (or never-opened) cursor is a no-op success.
        let status = unsafe { yeti_cursor_close(0, std::ptr::null_mut()) };
        assert!(matches!(status, YetiStatus::Ok));
    }
}
