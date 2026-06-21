//! cgo-facing C-ABI binding for the Yeti SDK (ADR-025 generated-binding model).
//!
//! A **thin wrapper** over [`yeti_sdk::remote`] — it adds nothing but a blocking
//! C ABI over the async client, so the Go surface (`sdks/yeti-sdk-go/go`)
//! delegates to the same client implementation the napi (Node) and uniffi
//! (Python/Swift/Kotlin) bindings wrap. One source, no drift.
//!
//! ## ABI shape
//!
//! Go cannot hold Rust objects, so handles are opaque `u64` tokens into
//! process-global registries (one for [`Client`], one for cursors). The async
//! core is driven to completion on a shared multi-thread tokio runtime; every
//! exported call is blocking from Go's view.
//!
//! Fallible calls return a [`YetiStatus`] (0 = ok) and, on error, write an
//! owned C string into the `err` out-param that Go copies then frees with
//! [`yeti_string_free`]. Byte payloads come back as a [`YetiBytes`] the caller
//! frees with [`yeti_bytes_free`]. The widening surface lives in
//! `yeti-sdk`; this binding follows on rebuild.

// The C ABI is inherently unsafe (raw pointers, ownership transfer across the
// boundary). The safety contract for every exported fn is documented at its
// site; callers (the Go package) honor it.
#![allow(clippy::missing_safety_doc)]

use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use tokio::runtime::Runtime;
use yeti_sdk::remote as client;

/// Status code returned by every fallible export. `0` is success; the negative
/// cases mirror [`client::ClientError`] so Go can branch on the kind.
#[repr(i32)]
pub enum YetiStatus {
    /// The call succeeded.
    Ok = 0,
    /// The connection could not be established or was lost.
    Connection = -1,
    /// The remote replied with an error frame (or refused the op).
    Remote = -2,
    /// A payload could not be encoded or decoded.
    Codec = -3,
    /// A handle argument did not refer to a live object.
    BadHandle = -4,
}

impl YetiStatus {
    fn of(e: &client::ClientError) -> Self {
        match e {
            client::ClientError::Connection(_) => YetiStatus::Connection,
            client::ClientError::Remote(_) => YetiStatus::Remote,
            client::ClientError::Codec(_) => YetiStatus::Codec,
        }
    }
}

/// An owned byte buffer handed to Go. `ptr` is heap memory this crate owns
/// until [`yeti_bytes_free`]; `present` distinguishes an empty value (`len`
/// 0, `present` true) from an absent one (`get`/`get-many` miss).
#[repr(C)]
pub struct YetiBytes {
    ptr: *mut u8,
    len: usize,
    present: bool,
}

impl YetiBytes {
    fn absent() -> Self {
        YetiBytes {
            ptr: std::ptr::null_mut(),
            len: 0,
            present: false,
        }
    }

    fn from_vec(v: Vec<u8>) -> Self {
        let mut boxed = v.into_boxed_slice();
        let ptr = boxed.as_mut_ptr();
        let len = boxed.len();
        std::mem::forget(boxed);
        YetiBytes {
            ptr,
            len,
            present: true,
        }
    }
}

/// One `(key, value)` pair in a C-owned [`YetiEntries`] array. `key` is a
/// NUL-terminated C string; `value` is `value_len` bytes at `value`.
#[repr(C)]
pub struct YetiEntry {
    key: *mut c_char,
    value: *mut u8,
    value_len: usize,
}

/// A C-owned array of [`YetiEntry`]. Free the whole array with
/// [`yeti_entries_free`].
#[repr(C)]
pub struct YetiEntries {
    items: *mut YetiEntry,
    len: usize,
}

impl YetiEntries {
    fn empty() -> Self {
        YetiEntries {
            items: std::ptr::null_mut(),
            len: 0,
        }
    }

    fn from_entries(entries: Vec<client::Entry>) -> Self {
        let items: Vec<YetiEntry> = entries
            .into_iter()
            .map(|e| {
                let mut val = e.value.into_boxed_slice();
                let value = val.as_mut_ptr();
                let value_len = val.len();
                std::mem::forget(val);
                YetiEntry {
                    // key has no interior NUL on the wire (it is a store key);
                    // CString::new only fails on interior NUL, so fall back to
                    // an empty string rather than panic across the ABI.
                    key: CString::new(e.key)
                        .unwrap_or_default()
                        .into_raw(),
                    value,
                    value_len,
                }
            })
            .collect();
        let mut boxed = items.into_boxed_slice();
        let ptr = boxed.as_mut_ptr();
        let len = boxed.len();
        std::mem::forget(boxed);
        YetiEntries { items: ptr, len }
    }

    /// Pack a list of keyless records (a `Query::execute` result — each row is
    /// just value bytes) into a [`YetiEntries`] where every `key` is an empty
    /// C string and each `value` is one record.
    fn from_records(records: Vec<Vec<u8>>) -> Self {
        let items: Vec<YetiEntry> = records
            .into_iter()
            .map(|rec| {
                let mut val = rec.into_boxed_slice();
                let value = val.as_mut_ptr();
                let value_len = val.len();
                std::mem::forget(val);
                YetiEntry {
                    key: CString::default().into_raw(),
                    value,
                    value_len,
                }
            })
            .collect();
        let mut boxed = items.into_boxed_slice();
        let ptr = boxed.as_mut_ptr();
        let len = boxed.len();
        std::mem::forget(boxed);
        YetiEntries { items: ptr, len }
    }
}

/// The shared multi-thread runtime that drives the async client. Created on
/// first use.
fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime for the Yeti Go binding")
    })
}

/// Process-global registry mapping opaque `u64` handles to live `T`. Handles
/// are never reused within a process run.
struct Registry<T> {
    next: AtomicU64,
    map: Mutex<HashMap<u64, T>>,
}

impl<T> Registry<T> {
    fn new() -> Self {
        Registry {
            next: AtomicU64::new(1),
            map: Mutex::new(HashMap::new()),
        }
    }

    fn insert(&self, value: T) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.map.lock().unwrap().insert(id, value);
        id
    }

    fn remove(&self, id: u64) -> Option<T> {
        self.map.lock().unwrap().remove(&id)
    }

    /// Run `f` against the live `T` for `id` while holding the registry lock,
    /// returning `None` if the handle is dead. Used for the `&self` handle ops
    /// (a `Query`'s refine/terminal calls) that neither mutate nor consume the
    /// registered value. `f` borrows the value and the lock is held for its
    /// whole duration, so a blocking await on the borrowed future is driven to
    /// completion inside `f` (the borrow never escapes the guard).
    fn with<R>(&self, id: u64, f: impl FnOnce(&T) -> R) -> Option<R> {
        self.map.lock().unwrap().get(&id).map(f)
    }
}

fn clients() -> &'static Registry<client::Client> {
    static R: OnceLock<Registry<client::Client>> = OnceLock::new();
    R.get_or_init(Registry::new)
}

fn cursors() -> &'static Registry<client::ScanCursor> {
    static R: OnceLock<Registry<client::ScanCursor>> = OnceLock::new();
    R.get_or_init(Registry::new)
}

fn queries() -> &'static Registry<client::Query> {
    static R: OnceLock<Registry<client::Query>> = OnceLock::new();
    R.get_or_init(Registry::new)
}

fn record_streams() -> &'static Registry<client::RecordStream> {
    static R: OnceLock<Registry<client::RecordStream>> = OnceLock::new();
    R.get_or_init(Registry::new)
}

fn subscriptions() -> &'static Registry<client::Subscription> {
    static R: OnceLock<Registry<client::Subscription>> = OnceLock::new();
    R.get_or_init(Registry::new)
}

/// Write an owned C error string into `err` (if non-null) and return the
/// status for `e`.
unsafe fn fail(e: &client::ClientError, err: *mut *mut c_char) -> YetiStatus {
    if !err.is_null() {
        *err = CString::new(e.to_string()).unwrap_or_default().into_raw();
    }
    YetiStatus::of(e)
}

unsafe fn str_arg<'a>(p: *const c_char) -> &'a str {
    if p.is_null() {
        return "";
    }
    CStr::from_ptr(p).to_str().unwrap_or("")
}

unsafe fn bytes_arg<'a>(p: *const u8, len: usize) -> &'a [u8] {
    if p.is_null() || len == 0 {
        return &[];
    }
    std::slice::from_raw_parts(p, len)
}

// ── Connection lifecycle ────────────────────────────────────────────────────

/// Connect to a Yeti host-rpc endpoint over TLS. `server_name` is the hostname
/// verified against the peer's leaf certificate; the dial is TLS-only (the SDK
/// `connect` has no plaintext path), trusting the public WebPKI root store so
/// the peer must present a certificate chaining to a public CA — mirroring
/// `yeti-sdk-node`'s minimal `connect(addr, serverName)` surface. On success
/// writes the client handle into `out` and returns [`YetiStatus::Ok`]; on
/// failure writes an owned error string into `err`.
///
/// # Safety
/// `addr` and `server_name` are valid NUL-terminated C strings; `out` and `err`
/// are valid writable pointers (or `err` may be null).
#[no_mangle]
pub unsafe extern "C" fn yeti_connect(
    addr: *const c_char,
    server_name: *const c_char,
    out: *mut u64,
    err: *mut *mut c_char,
) -> YetiStatus {
    let addr = str_arg(addr).to_owned();
    let server_name = str_arg(server_name).to_owned();
    let tls = client::TlsConfig::webpki(server_name);
    match runtime().block_on(client::connect(&addr, tls)) {
        Ok(c) => {
            *out = clients().insert(c);
            YetiStatus::Ok
        }
        Err(e) => fail(&e, err),
    }
}

/// Connect to a Yeti host-rpc endpoint over TLS, trusting exactly the CA
/// cert(s) in `ca_pem` instead of the public WebPKI root store — the dev /
/// self-host path where the node presents a certificate signed by a private CA
/// (e.g. an mkcert root) that the public roots reject. `server_name` is the
/// hostname verified against the peer's leaf certificate; the dial stays
/// TLS-only (no plaintext, no skip-verify path). On success writes the client
/// handle into `out` and returns [`YetiStatus::Ok`]; on failure (a bad CA PEM,
/// or a connection/handshake error) writes an owned error string into `err`.
///
/// # Safety
/// `addr`, `server_name`, and `ca_pem` are valid NUL-terminated C strings;
/// `out` and `err` are valid writable pointers (or `err` may be null).
#[no_mangle]
pub unsafe extern "C" fn yeti_connect_pinned(
    addr: *const c_char,
    server_name: *const c_char,
    ca_pem: *const c_char,
    out: *mut u64,
    err: *mut *mut c_char,
) -> YetiStatus {
    let addr = str_arg(addr).to_owned();
    let server_name = str_arg(server_name).to_owned();
    let ca_pem = str_arg(ca_pem).to_owned();
    let tls = match client::TlsConfig::pinned_roots(&ca_pem, server_name) {
        Ok(tls) => tls,
        Err(e) => {
            if !err.is_null() {
                *err = CString::new(e.to_string()).unwrap_or_default().into_raw();
            }
            return YetiStatus::Connection;
        }
    };
    match runtime().block_on(client::connect(&addr, tls)) {
        Ok(c) => {
            *out = clients().insert(c);
            YetiStatus::Ok
        }
        Err(e) => fail(&e, err),
    }
}

/// Close a client connection and release its handle (idempotent).
///
/// # Safety
/// `client` is a handle previously returned by [`yeti_connect`] (or 0).
#[no_mangle]
pub unsafe extern "C" fn yeti_client_close(client: u64) {
    clients().remove(client);
}

/// Resolve a live `Store` facade for `client`, or `None` if the handle is
/// dead.
fn store_of(client: u64) -> Option<client::Store> {
    clients()
        .map
        .lock()
        .unwrap()
        .get(&client)
        .map(client::Client::store)
}

fn log_of(client: u64) -> Option<client::Log> {
    clients()
        .map
        .lock()
        .unwrap()
        .get(&client)
        .map(client::Client::log)
}

fn publisher_of(client: u64) -> Option<client::Publisher> {
    clients()
        .map
        .lock()
        .unwrap()
        .get(&client)
        .map(client::Client::publisher)
}

fn subscriber_of(client: u64) -> Option<client::Subscriber> {
    clients()
        .map
        .lock()
        .unwrap()
        .get(&client)
        .map(client::Client::subscriber)
}

fn rate_limit_of(client: u64) -> Option<client::RateLimit> {
    clients()
        .map
        .lock()
        .unwrap()
        .get(&client)
        .map(client::Client::rate_limit)
}

unsafe fn bad_handle(err: *mut *mut c_char) -> YetiStatus {
    if !err.is_null() {
        *err = CString::new("invalid client handle")
            .unwrap_or_default()
            .into_raw();
    }
    YetiStatus::BadHandle
}

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
            let mut boxed = items.into_boxed_slice();
            *out = YetiEntries {
                items: boxed.as_mut_ptr(),
                len: boxed.len(),
            };
            std::mem::forget(boxed);
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

// ── Store: scan cursor ──────────────────────────────────────────────────────

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

// ── Query: FIQL filter builder ──────────────────────────────────────────────

/// Open a FIQL filter [`query`](client::Query) over `(table, fiql)`; writes the
/// query handle into `out`. Refine it with `yeti_query_sort_by` /
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
pub unsafe extern "C" fn yeti_query_limit(
    query: u64,
    n: u32,
    err: *mut *mut c_char,
) -> YetiStatus {
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

// ── Messaging: publish / subscribe ──────────────────────────────────────────

/// Fire a change out to a table's subscribers on the peer (fire-and-forget).
/// `op` is the `pubsub-op` u8: `0` create, `1` update, `2` delete.
///
/// # Safety
/// String args are NUL-terminated; `value`/`value_len` describe a valid slice.
#[no_mangle]
pub unsafe extern "C" fn yeti_publish(
    client: u64,
    table: *const c_char,
    op: u8,
    key: *const c_char,
    value: *const u8,
    value_len: usize,
    err: *mut *mut c_char,
) -> YetiStatus {
    let Some(publisher) = publisher_of(client) else {
        return bad_handle(err);
    };
    let (t, k, v) = (str_arg(table), str_arg(key), bytes_arg(value, value_len));
    match runtime().block_on(publisher.publish(t, op, k, v)) {
        Ok(()) => YetiStatus::Ok,
        Err(e) => fail(&e, err),
    }
}

/// Open a table's update stream on the peer; writes a subscription handle into
/// `out`. `filter` is an optional server-side FIQL expression — pass null to
/// subscribe to every change. Drain it with `yeti_subscription_next`, free it
/// with `yeti_subscription_close`.
///
/// # Safety
/// `table` is NUL-terminated; `filter` is NUL-terminated or null; `out`/`err`
/// valid.
#[no_mangle]
pub unsafe extern "C" fn yeti_subscribe(
    client: u64,
    table: *const c_char,
    filter: *const c_char,
    out: *mut u64,
    err: *mut *mut c_char,
) -> YetiStatus {
    let Some(subscriber) = subscriber_of(client) else {
        return bad_handle(err);
    };
    // A null `filter` is `None` (subscribe to all); a non-null one is the FIQL.
    let filter = if filter.is_null() {
        None
    } else {
        Some(str_arg(filter))
    };
    match runtime().block_on(subscriber.subscribe(str_arg(table), filter)) {
        Ok(subscription) => {
            *out = subscriptions().insert(subscription);
            YetiStatus::Ok
        }
        Err(e) => fail(&e, err),
    }
}

/// Pull the next event from a subscription. On success writes the event fields
/// into the out-params; when the peer closes the stream `done` is set true and
/// the other out-params are left untouched.
///
/// `op` is the `pubsub-op` u8 (`0` create, `1` update, `2` delete); `key` is an
/// owned C string the caller frees with [`yeti_string_free`]; `value` is a
/// [`YetiBytes`] the caller frees with [`yeti_bytes_free`]; `timestamp_ms` is
/// the event time in epoch milliseconds.
///
/// # Safety
/// `subscription` is a handle from [`yeti_subscribe`]; every out-param is a
/// valid writable pointer (or `err` null).
#[no_mangle]
pub unsafe extern "C" fn yeti_subscription_next(
    subscription: u64,
    op: *mut u8,
    key: *mut *mut c_char,
    value: *mut YetiBytes,
    timestamp_ms: *mut u64,
    done: *mut bool,
    err: *mut *mut c_char,
) -> YetiStatus {
    // Take the subscription out so we can poll it (`next` is `&mut self`), then
    // put it back unless the peer closed it — exactly the cursor pattern.
    let mut sub = match subscriptions().remove(subscription) {
        Some(s) => s,
        None => return bad_handle(err),
    };
    match runtime().block_on(sub.next()) {
        Ok(Some(event)) => {
            *op = event.op;
            *key = CString::new(event.key).unwrap_or_default().into_raw();
            *value = YetiBytes::from_vec(event.value);
            *timestamp_ms = event.timestamp_ms;
            *done = false;
            subscriptions()
                .map
                .lock()
                .unwrap()
                .insert(subscription, sub);
            YetiStatus::Ok
        }
        Ok(None) => {
            *done = true;
            // Closed: drop our side (sub goes out of scope).
            YetiStatus::Ok
        }
        Err(e) => {
            subscriptions()
                .map
                .lock()
                .unwrap()
                .insert(subscription, sub);
            fail(&e, err)
        }
    }
}

/// Free a subscription handle (idempotent). Dropping it releases the local
/// receiver; the peer reaps its side when the connection closes.
///
/// # Safety
/// `subscription` is a handle from [`yeti_subscribe`] (or already freed).
#[no_mangle]
pub unsafe extern "C" fn yeti_subscription_close(subscription: u64) {
    subscriptions().remove(subscription);
}

// ── Ingress: rate-limit ─────────────────────────────────────────────────────

/// The outcome of a rate-limit `check`, mirroring
/// [`RateDecision`](client::wire::ratelimit_codec::RateDecision). `allowed` is
/// whether a token was available and consumed; `remaining` is the tokens left
/// after this check; `retry_after_ms` is the suggested wait before retrying
/// when denied (`0` when allowed).
#[repr(C)]
pub struct YetiRateDecision {
    allowed: bool,
    remaining: u32,
    retry_after_ms: u32,
}

/// Consume one token from the peer's `(bucket, key)` slot; writes the
/// [`YetiRateDecision`] into `out`.
///
/// # Safety
/// String args are NUL-terminated; `out`/`err` valid.
#[no_mangle]
pub unsafe extern "C" fn yeti_ratelimit_check(
    client: u64,
    bucket: *const c_char,
    key: *const c_char,
    out: *mut YetiRateDecision,
    err: *mut *mut c_char,
) -> YetiStatus {
    let Some(rate_limit) = rate_limit_of(client) else {
        return bad_handle(err);
    };
    match runtime().block_on(rate_limit.check(str_arg(bucket), str_arg(key))) {
        Ok(decision) => {
            *out = YetiRateDecision {
                allowed: decision.allowed,
                remaining: decision.remaining,
                retry_after_ms: decision.retry_after_ms,
            };
            YetiStatus::Ok
        }
        Err(e) => fail(&e, err),
    }
}

// ── Telemetry / logs ────────────────────────────────────────────────────────

/// Ship one log record (fire-and-acknowledge). `severity` is the wire integer
/// (0=trace .. 4=error). Fields are `n` parallel `(field_keys[i],
/// field_values[i])` C strings.
///
/// # Safety
/// String args NUL-terminated; the field arrays each have `n` valid C string
/// pointers.
#[no_mangle]
pub unsafe extern "C" fn yeti_log_emit(
    client: u64,
    severity: u8,
    target: *const c_char,
    message: *const c_char,
    field_keys: *const *const c_char,
    field_values: *const *const c_char,
    n: usize,
    err: *mut *mut c_char,
) -> YetiStatus {
    let Some(log) = log_of(client) else {
        return bad_handle(err);
    };
    let sev = match severity {
        0 => client::Severity::Trace,
        1 => client::Severity::Debug,
        3 => client::Severity::Warn,
        4 => client::Severity::Error,
        _ => client::Severity::Info,
    };
    let fields: Vec<(String, String)> = (0..n)
        .map(|i| {
            (
                str_arg(*field_keys.add(i)).to_owned(),
                str_arg(*field_values.add(i)).to_owned(),
            )
        })
        .collect();
    match runtime().block_on(log.emit(sev, str_arg(target), str_arg(message), fields)) {
        Ok(()) => YetiStatus::Ok,
        Err(e) => fail(&e, err),
    }
}

// ── Memory release ──────────────────────────────────────────────────────────

/// Free a C string previously written into an `err` out-param.
///
/// # Safety
/// `s` was produced by this library (or null).
#[no_mangle]
pub unsafe extern "C" fn yeti_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

/// Free a [`YetiBytes`] produced by this library.
///
/// # Safety
/// `b` was produced by this library; call at most once.
#[no_mangle]
pub unsafe extern "C" fn yeti_bytes_free(b: YetiBytes) {
    if !b.ptr.is_null() {
        drop(Vec::from_raw_parts(b.ptr, b.len, b.len));
    }
}

/// Free a [`YetiEntries`] (and every key/value it owns) produced by this
/// library.
///
/// # Safety
/// `e` was produced by this library; call at most once.
#[no_mangle]
pub unsafe extern "C" fn yeti_entries_free(e: YetiEntries) {
    if e.items.is_null() {
        return;
    }
    let items = Vec::from_raw_parts(e.items, e.len, e.len);
    for item in items {
        if !item.key.is_null() {
            drop(CString::from_raw(item.key));
        }
        if !item.value.is_null() {
            drop(Vec::from_raw_parts(item.value, item.value_len, item.value_len));
        }
    }
}

// A zero-sized anchor so the staticlib always exports at least one symbol even
// if the linker is aggressive; also lets a smoke test confirm the lib loads.
/// Returns the ABI version of this binding (bump on breaking C-ABI changes).
#[no_mangle]
pub extern "C" fn yeti_abi_version() -> u32 {
    1
}

// Silence "unused" for the c_void import kept for header completeness.
#[allow(dead_code)]
fn _abi_void_anchor(_: *mut c_void) {}
