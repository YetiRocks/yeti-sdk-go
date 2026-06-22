//! Store exports: single-key ops, prefix scan/count, and batch ops. Every call
//! resolves a [`Store`](yeti_sdk::remote::Store) facade for the client handle,
//! then drives one async op to completion. The paged scan cursor lives in
//! [`crate::cursor`]; the FIQL query builder in [`crate::query`].

use std::ffi::{c_char, CString};

use yeti_sdk::remote as client;

use crate::ffi::{YetiBytes, YetiEntries, YetiEntry, YetiStatus};
use crate::registry::{runtime, store_of};
use crate::util::{bad_handle, bytes_arg, fail, str_arg};

// ── Store: single-key ops ───────────────────────────────────────────────────

/// Store `value` under `(table, key)`.
///
/// # Safety
/// String args are NUL-terminated; `value`/`value_len` describe a valid slice.
#[no_mangle]
pub unsafe extern "C" fn yeti_store_put(
    client: u64,
    table: *const c_char,
    key: *const c_char,
    value: *const u8,
    value_len: usize,
    err: *mut *mut c_char,
) -> YetiStatus {
    let Some(store) = store_of(client) else {
        return bad_handle(err);
    };
    let (t, k, v) = (str_arg(table), str_arg(key), bytes_arg(value, value_len));
    match runtime().block_on(store.put(t, k, v)) {
        Ok(()) => YetiStatus::Ok,
        Err(e) => fail(&e, err),
    }
}

/// Fetch the bytes under `(table, key)`. On success writes the value (or an
/// absent [`YetiBytes`]) into `out`.
///
/// # Safety
/// String args are NUL-terminated; `out`/`err` are valid (or `err` null).
#[no_mangle]
pub unsafe extern "C" fn yeti_store_get(
    client: u64,
    table: *const c_char,
    key: *const c_char,
    out: *mut YetiBytes,
    err: *mut *mut c_char,
) -> YetiStatus {
    let Some(store) = store_of(client) else {
        return bad_handle(err);
    };
    match runtime().block_on(store.get(str_arg(table), str_arg(key))) {
        Ok(Some(v)) => {
            *out = YetiBytes::from_vec(v);
            YetiStatus::Ok
        }
        Ok(None) => {
            *out = YetiBytes::absent();
            YetiStatus::Ok
        }
        Err(e) => fail(&e, err),
    }
}

/// Store `value` under `(table, key)` only if absent; writes whether it was
/// inserted into `out`.
///
/// # Safety
/// As [`yeti_store_put`]; `out` is a valid writable bool pointer.
#[no_mangle]
pub unsafe extern "C" fn yeti_store_put_if_absent(
    client: u64,
    table: *const c_char,
    key: *const c_char,
    value: *const u8,
    value_len: usize,
    out: *mut bool,
    err: *mut *mut c_char,
) -> YetiStatus {
    let Some(store) = store_of(client) else {
        return bad_handle(err);
    };
    let (t, k, v) = (str_arg(table), str_arg(key), bytes_arg(value, value_len));
    match runtime().block_on(store.put_if_absent(t, k, v)) {
        Ok(b) => {
            *out = b;
            YetiStatus::Ok
        }
        Err(e) => fail(&e, err),
    }
}

/// Delete `(table, key)`; writes whether the key existed into `out`.
///
/// # Safety
/// String args are NUL-terminated; `out` is a valid writable bool pointer.
#[no_mangle]
pub unsafe extern "C" fn yeti_store_delete(
    client: u64,
    table: *const c_char,
    key: *const c_char,
    out: *mut bool,
    err: *mut *mut c_char,
) -> YetiStatus {
    let Some(store) = store_of(client) else {
        return bad_handle(err);
    };
    match runtime().block_on(store.delete(str_arg(table), str_arg(key))) {
        Ok(b) => {
            *out = b;
            YetiStatus::Ok
        }
        Err(e) => fail(&e, err),
    }
}

/// Whether `(table, key)` is present; writes the answer into `out`.
///
/// # Safety
/// String args are NUL-terminated; `out` is a valid writable bool pointer.
#[no_mangle]
pub unsafe extern "C" fn yeti_store_exists(
    client: u64,
    table: *const c_char,
    key: *const c_char,
    out: *mut bool,
    err: *mut *mut c_char,
) -> YetiStatus {
    let Some(store) = store_of(client) else {
        return bad_handle(err);
    };
    match runtime().block_on(store.exists(str_arg(table), str_arg(key))) {
        Ok(b) => {
            *out = b;
            YetiStatus::Ok
        }
        Err(e) => fail(&e, err),
    }
}

// ── Store: scan / count ─────────────────────────────────────────────────────

/// List `(key, value)` entries under `table` starting with `prefix`; writes a
/// C-owned [`YetiEntries`] into `out`. (`scan` for keys-only is built on this
/// in the Go layer.)
///
/// # Safety
/// String args are NUL-terminated; `out`/`err` valid.
#[no_mangle]
pub unsafe extern "C" fn yeti_store_scan_prefix(
    client: u64,
    table: *const c_char,
    prefix: *const c_char,
    out: *mut YetiEntries,
    err: *mut *mut c_char,
) -> YetiStatus {
    let Some(store) = store_of(client) else {
        return bad_handle(err);
    };
    match runtime().block_on(store.scan_prefix(str_arg(table), str_arg(prefix))) {
        Ok(entries) => {
            *out = YetiEntries::from_entries(entries);
            YetiStatus::Ok
        }
        Err(e) => fail(&e, err),
    }
}

/// Count the keys under `table` starting with `prefix`; writes the count into
/// `out`.
///
/// # Safety
/// String args are NUL-terminated; `out` is a valid writable u32 pointer.
#[no_mangle]
pub unsafe extern "C" fn yeti_store_count_prefix(
    client: u64,
    table: *const c_char,
    prefix: *const c_char,
    out: *mut u32,
    err: *mut *mut c_char,
) -> YetiStatus {
    let Some(store) = store_of(client) else {
        return bad_handle(err);
    };
    match runtime().block_on(store.count_prefix(str_arg(table), str_arg(prefix))) {
        Ok(n) => {
            *out = n;
            YetiStatus::Ok
        }
        Err(e) => fail(&e, err),
    }
}

// ── Store: batch ops ────────────────────────────────────────────────────────

/// Fetch many keys at once. `keys` is `keys_len` NUL-terminated C strings;
/// writes a [`YetiEntries`] into `out` with one slot per requested key, in
/// order — an absent key has a null `value` and `value_len` 0 (its `key` slot
/// still carries the requested key).
///
/// # Safety
/// `keys` points to `keys_len` valid C string pointers; `out`/`err` valid.
#[no_mangle]
pub unsafe extern "C" fn yeti_store_get_many(
    client: u64,
    table: *const c_char,
    keys: *const *const c_char,
    keys_len: usize,
    out: *mut YetiEntries,
    err: *mut *mut c_char,
) -> YetiStatus {
    let Some(store) = store_of(client) else {
        return bad_handle(err);
    };
    let key_strs: Vec<String> = (0..keys_len)
        .map(|i| str_arg(*keys.add(i)).to_owned())
        .collect();
    match runtime().block_on(store.get_many(str_arg(table), &key_strs)) {
        Ok(vals) => {
            // Pair each requested key with its slot so Go can map by key.
            let items: Vec<YetiEntry> = key_strs
                .into_iter()
                .zip(vals)
                .map(|(k, v)| {
                    let (value, value_len) = match v {
                        Some(bytes) => {
                            let mut b = bytes.into_boxed_slice();
                            let p = b.as_mut_ptr();
                            let l = b.len();
                            std::mem::forget(b);
                            (p, l)
                        }
                        None => (std::ptr::null_mut(), 0),
                    };
                    YetiEntry {
                        key: CString::new(k).unwrap_or_default().into_raw(),
                        value,
                        value_len,
                    }
                })
                .collect();
            *out = YetiEntries::from_items(items);
            YetiStatus::Ok
        }
        Err(e) => fail(&e, err),
    }
}

/// Store many `(key, value)` entries under `table` in one call. The three
/// parallel arrays each have `n` elements: `keys[i]` is a C string, the value
/// is `value_lens[i]` bytes at `values[i]`.
///
/// # Safety
/// The parallel arrays each have `n` valid elements; string/byte pointers are
/// valid.
#[no_mangle]
pub unsafe extern "C" fn yeti_store_put_many(
    client: u64,
    table: *const c_char,
    keys: *const *const c_char,
    values: *const *const u8,
    value_lens: *const usize,
    n: usize,
    err: *mut *mut c_char,
) -> YetiStatus {
    let Some(store) = store_of(client) else {
        return bad_handle(err);
    };
    let entries: Vec<client::Entry> = (0..n)
        .map(|i| client::Entry {
            key: str_arg(*keys.add(i)).to_owned(),
            value: bytes_arg(*values.add(i), *value_lens.add(i)).to_vec(),
        })
        .collect();
    match runtime().block_on(store.put_many(str_arg(table), &entries)) {
        Ok(()) => YetiStatus::Ok,
        Err(e) => fail(&e, err),
    }
}

/// Delete many keys under `table`; writes how many existed into `out`.
///
/// # Safety
/// `keys` points to `keys_len` valid C string pointers; `out`/`err` valid.
#[no_mangle]
pub unsafe extern "C" fn yeti_store_delete_many(
    client: u64,
    table: *const c_char,
    keys: *const *const c_char,
    keys_len: usize,
    out: *mut u32,
    err: *mut *mut c_char,
) -> YetiStatus {
    let Some(store) = store_of(client) else {
        return bad_handle(err);
    };
    let key_strs: Vec<String> = (0..keys_len)
        .map(|i| str_arg(*keys.add(i)).to_owned())
        .collect();
    match runtime().block_on(store.delete_many(str_arg(table), &key_strs)) {
        Ok(n) => {
            *out = n;
            YetiStatus::Ok
        }
        Err(e) => fail(&e, err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ops_on_a_dead_client_handle_report_bad_handle() {
        // Every store op resolves a facade first; a fabricated handle has no
        // client, so the call must surface BadHandle with the canonical string
        // rather than dialing or panicking.
        let mut err: *mut c_char = std::ptr::null_mut();
        let table = CString::new("notes").unwrap();
        let key = CString::new("k").unwrap();
        let status = unsafe {
            yeti_store_put(
                0,
                table.as_ptr(),
                key.as_ptr(),
                std::ptr::null(),
                0,
                &mut err,
            )
        };
        assert!(matches!(status, YetiStatus::BadHandle));
        let msg = unsafe { CString::from_raw(err) };
        assert_eq!(msg.to_str().unwrap(), "invalid client handle");
    }
}
