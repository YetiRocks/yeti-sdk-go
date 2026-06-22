//! Query exports: the FIQL filter builder (open, refine, materialize, close)
//! and the record stream its `each` terminal returns.

use std::ffi::c_char;

use crate::ffi::{YetiBytes, YetiEntries, YetiStatus};
use crate::registry::{queries, record_streams, runtime, store_of};
use crate::util::{bad_handle, fail, str_arg};

/// Open a FIQL filter [`query`](yeti_sdk::remote::Query) over `(table, fiql)`;
/// writes the query handle into `out`. Refine it with `yeti_query_sort_by` /
/// `yeti_query_limit` / `yeti_query_offset`, then materialize with
/// `yeti_query_execute` / `yeti_query_count` / `yeti_query_each`; release it
/// with `yeti_query_close`.
///
/// # Safety
/// String args are NUL-terminated; `out`/`err` valid.
#[no_mangle]
pub unsafe extern "C" fn yeti_store_open_query(
    client: u64,
    table: *const c_char,
    fiql: *const c_char,
    out: *mut u64,
    err: *mut *mut c_char,
) -> YetiStatus {
    let Some(store) = store_of(client) else {
        return bad_handle(err);
    };
    match runtime().block_on(store.query(str_arg(table), str_arg(fiql))) {
        Ok(query) => {
            *out = queries().insert(query);
            YetiStatus::Ok
        }
        Err(e) => fail(&e, err),
    }
}

/// Add a sort key to a query (applied in call order). `ascending` is the sort
/// direction.
///
/// # Safety
/// `query` is a handle from [`yeti_store_open_query`]; `field` NUL-terminated;
/// `err` valid (or null).
#[no_mangle]
pub unsafe extern "C" fn yeti_query_sort_by(
    query: u64,
    field: *const c_char,
    ascending: bool,
    err: *mut *mut c_char,
) -> YetiStatus {
    let field = str_arg(field);
    // `sort_by` borrows the query (`&self`); block on the borrowed future
    // *inside* the registry guard so the borrow never escapes the lock. The
    // handle stays registered (refine ops don't consume it).
    let result = queries().with(query, |q| runtime().block_on(q.sort_by(field, ascending)));
    match result {
        None => bad_handle(err),
        Some(Ok(())) => YetiStatus::Ok,
        Some(Err(e)) => fail(&e, err),
    }
}

/// Cap the number of records a query returns.
///
/// # Safety
/// `query` is a handle from [`yeti_store_open_query`]; `err` valid (or null).
#[no_mangle]
pub unsafe extern "C" fn yeti_query_limit(query: u64, n: u32, err: *mut *mut c_char) -> YetiStatus {
    let result = queries().with(query, |q| runtime().block_on(q.limit(n)));
    match result {
        None => bad_handle(err),
        Some(Ok(())) => YetiStatus::Ok,
        Some(Err(e)) => fail(&e, err),
    }
}

/// Skip the first `n` matched records of a query.
///
/// # Safety
/// `query` is a handle from [`yeti_store_open_query`]; `err` valid (or null).
#[no_mangle]
pub unsafe extern "C" fn yeti_query_offset(
    query: u64,
    n: u32,
    err: *mut *mut c_char,
) -> YetiStatus {
    let result = queries().with(query, |q| runtime().block_on(q.offset(n)));
    match result {
        None => bad_handle(err),
        Some(Ok(())) => YetiStatus::Ok,
        Some(Err(e)) => fail(&e, err),
    }
}

/// Materialize the matched records, each as the JSON bytes the SDK decodes;
/// writes a C-owned [`YetiEntries`] into `out` whose every `key` is empty and
/// whose `value` is one record. (The handle persists; you may `execute` then
/// `count` the same query.)
///
/// # Safety
/// `query` is a handle from [`yeti_store_open_query`]; `out`/`err` valid.
#[no_mangle]
pub unsafe extern "C" fn yeti_query_execute(
    query: u64,
    out: *mut YetiEntries,
    err: *mut *mut c_char,
) -> YetiStatus {
    let result = queries().with(query, |q| runtime().block_on(q.execute()));
    match result {
        None => bad_handle(err),
        Some(Ok(rows)) => {
            *out = YetiEntries::from_records(rows);
            YetiStatus::Ok
        }
        Some(Err(e)) => fail(&e, err),
    }
}

/// Count the matched records; writes the count into `out`.
///
/// # Safety
/// `query` is a handle from [`yeti_store_open_query`]; `out`/`err` valid.
#[no_mangle]
pub unsafe extern "C" fn yeti_query_count(
    query: u64,
    out: *mut u32,
    err: *mut *mut c_char,
) -> YetiStatus {
    let result = queries().with(query, |q| runtime().block_on(q.count()));
    match result {
        None => bad_handle(err),
        Some(Ok(n)) => {
            *out = n;
            YetiStatus::Ok
        }
        Some(Err(e)) => fail(&e, err),
    }
}

/// Open the streaming variant of `execute`; writes a record-stream handle into
/// `out`. Drain it with `yeti_record_stream_next`, free it with
/// `yeti_record_stream_close`.
///
/// # Safety
/// `query` is a handle from [`yeti_store_open_query`]; `out`/`err` valid.
#[no_mangle]
pub unsafe extern "C" fn yeti_query_each(
    query: u64,
    out: *mut u64,
    err: *mut *mut c_char,
) -> YetiStatus {
    let result = queries().with(query, |q| runtime().block_on(q.each()));
    match result {
        None => bad_handle(err),
        Some(Ok(stream)) => {
            *out = record_streams().insert(stream);
            YetiStatus::Ok
        }
        Some(Err(e)) => fail(&e, err),
    }
}

/// Release a query's peer-side resource and free its handle (idempotent).
///
/// # Safety
/// `query` is a handle from [`yeti_store_open_query`] (or already freed).
#[no_mangle]
pub unsafe extern "C" fn yeti_query_close(query: u64, err: *mut *mut c_char) -> YetiStatus {
    let Some(q) = queries().remove(query) else {
        return YetiStatus::Ok;
    };
    // `Query::close` consumes by value, so we own the value we removed.
    match runtime().block_on(q.close()) {
        Ok(()) => YetiStatus::Ok,
        Err(e) => fail(&e, err),
    }
}

/// Pull the next record (JSON bytes) from a record stream. On success writes the
/// record into `out`; when the stream ends `done` is set true and `out` is an
/// absent [`YetiBytes`].
///
/// # Safety
/// `stream` is a handle from [`yeti_query_each`]; `out`/`done`/`err` valid (or
/// `err` null).
#[no_mangle]
pub unsafe extern "C" fn yeti_record_stream_next(
    stream: u64,
    out: *mut YetiBytes,
    done: *mut bool,
    err: *mut *mut c_char,
) -> YetiStatus {
    // Take the stream out so we can poll it (`next` is `&mut self`), then put it
    // back unless it ended — exactly the cursor pattern.
    let mut s = match record_streams().remove(stream) {
        Some(s) => s,
        None => return bad_handle(err),
    };
    match runtime().block_on(s.next()) {
        Ok(Some(rec)) => {
            *out = YetiBytes::from_vec(rec);
            *done = false;
            record_streams().map.lock().unwrap().insert(stream, s);
            YetiStatus::Ok
        }
        Ok(None) => {
            *out = YetiBytes::absent();
            *done = true;
            // Stream ended: drop our side (s goes out of scope).
            YetiStatus::Ok
        }
        Err(e) => {
            record_streams().map.lock().unwrap().insert(stream, s);
            fail(&e, err)
        }
    }
}

/// Free a record-stream handle (idempotent). The stream has no peer-side
/// teardown call of its own; dropping it releases the local receiver.
///
/// # Safety
/// `stream` is a handle from [`yeti_query_each`] (or already freed).
#[no_mangle]
pub unsafe extern "C" fn yeti_record_stream_close(stream: u64) {
    record_streams().remove(stream);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn refine_ops_on_dead_query_report_bad_handle() {
        let mut err: *mut c_char = std::ptr::null_mut();
        let status = unsafe { yeti_query_limit(777, 5, &mut err) };
        assert!(matches!(status, YetiStatus::BadHandle));
        let msg = unsafe { CString::from_raw(err) };
        assert_eq!(msg.to_str().unwrap(), "invalid client handle");
    }

    #[test]
    fn query_close_on_unknown_handle_is_ok() {
        let status = unsafe { yeti_query_close(0, std::ptr::null_mut()) };
        assert!(matches!(status, YetiStatus::Ok));
    }

    #[test]
    fn record_stream_close_on_unknown_handle_is_a_noop() {
        unsafe { yeti_record_stream_close(0) };
        unsafe { yeti_record_stream_close(98_765) };
    }
}
