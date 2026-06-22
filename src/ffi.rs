//! The `#[repr(C)]` value types that cross the cgo boundary, plus their
//! owned-buffer constructors. Every type here is laid out for C: Go reads the
//! fields directly and frees the owned buffers through the `*_free` exports in
//! [`crate::memory`].

use std::ffi::{c_char, CString};

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
    pub(crate) fn of(e: &client::ClientError) -> Self {
        match e {
            client::ClientError::Connection(_) => YetiStatus::Connection,
            client::ClientError::Remote(_) => YetiStatus::Remote,
            client::ClientError::Codec(_) => YetiStatus::Codec,
        }
    }
}

/// An owned byte buffer handed to Go. `ptr` is heap memory this crate owns
/// until [`crate::memory::yeti_bytes_free`]; `present` distinguishes an empty
/// value (`len` 0, `present` true) from an absent one (`get`/`get-many` miss).
#[repr(C)]
pub struct YetiBytes {
    pub(crate) ptr: *mut u8,
    pub(crate) len: usize,
    pub(crate) present: bool,
}

impl YetiBytes {
    pub(crate) fn absent() -> Self {
        YetiBytes {
            ptr: std::ptr::null_mut(),
            len: 0,
            present: false,
        }
    }

    pub(crate) fn from_vec(v: Vec<u8>) -> Self {
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
    pub(crate) key: *mut c_char,
    pub(crate) value: *mut u8,
    pub(crate) value_len: usize,
}

/// A C-owned array of [`YetiEntry`]. Free the whole array with
/// [`crate::memory::yeti_entries_free`].
#[repr(C)]
pub struct YetiEntries {
    pub(crate) items: *mut YetiEntry,
    pub(crate) len: usize,
}

impl YetiEntries {
    pub(crate) fn empty() -> Self {
        YetiEntries {
            items: std::ptr::null_mut(),
            len: 0,
        }
    }

    pub(crate) fn from_entries(entries: Vec<client::Entry>) -> Self {
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
                    key: CString::new(e.key).unwrap_or_default().into_raw(),
                    value,
                    value_len,
                }
            })
            .collect();
        Self::from_items(items)
    }

    /// Pack a list of keyless records (a `Query::execute` result — each row is
    /// just value bytes) into a [`YetiEntries`] where every `key` is an empty
    /// C string and each `value` is one record.
    pub(crate) fn from_records(records: Vec<Vec<u8>>) -> Self {
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
        Self::from_items(items)
    }

    /// Leak a `Vec<YetiEntry>` into a C-owned array. The single place that
    /// forgets the backing allocation, so [`crate::memory::yeti_entries_free`]
    /// is the only one that reclaims it.
    pub(crate) fn from_items(items: Vec<YetiEntry>) -> Self {
        let mut boxed = items.into_boxed_slice();
        let ptr = boxed.as_mut_ptr();
        let len = boxed.len();
        std::mem::forget(boxed);
        YetiEntries { items: ptr, len }
    }
}

/// The outcome of a rate-limit `check`, mirroring
/// [`RateDecision`](client::wire::ratelimit_codec::RateDecision). `allowed` is
/// whether a token was available and consumed; `remaining` is the tokens left
/// after this check; `retry_after_ms` is the suggested wait before retrying
/// when denied (`0` when allowed).
#[repr(C)]
pub struct YetiRateDecision {
    pub(crate) allowed: bool,
    pub(crate) remaining: u32,
    pub(crate) retry_after_ms: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_absent_is_not_present() {
        let b = YetiBytes::absent();
        assert!(b.ptr.is_null());
        assert_eq!(b.len, 0);
        assert!(!b.present);
    }

    #[test]
    fn bytes_from_vec_round_trips_through_free() {
        let b = YetiBytes::from_vec(vec![1, 2, 3, 4]);
        assert!(b.present);
        assert_eq!(b.len, 4);
        assert!(!b.ptr.is_null());
        let slice = unsafe { std::slice::from_raw_parts(b.ptr, b.len) };
        assert_eq!(slice, &[1, 2, 3, 4]);
        // Reclaim through the public free path (no leak under miri/asan).
        unsafe { crate::memory::yeti_bytes_free(b) };
    }

    #[test]
    fn bytes_from_empty_vec_is_present_with_len_zero() {
        let b = YetiBytes::from_vec(Vec::new());
        assert!(b.present, "an empty value is present, not absent");
        assert_eq!(b.len, 0);
        unsafe { crate::memory::yeti_bytes_free(b) };
    }

    #[test]
    fn entries_empty_has_no_items() {
        let e = YetiEntries::empty();
        assert!(e.items.is_null());
        assert_eq!(e.len, 0);
        unsafe { crate::memory::yeti_entries_free(e) };
    }

    #[test]
    fn entries_from_entries_preserves_order_and_values() {
        let entries = vec![
            client::Entry {
                key: "a".to_owned(),
                value: b"A".to_vec(),
            },
            client::Entry {
                key: "b".to_owned(),
                value: b"BB".to_vec(),
            },
        ];
        let e = YetiEntries::from_entries(entries);
        assert_eq!(e.len, 2);
        let items = unsafe { std::slice::from_raw_parts(e.items, e.len) };
        let key0 = unsafe { std::ffi::CStr::from_ptr(items[0].key) };
        let key1 = unsafe { std::ffi::CStr::from_ptr(items[1].key) };
        assert_eq!(key0.to_str().unwrap(), "a");
        assert_eq!(key1.to_str().unwrap(), "b");
        let val0 = unsafe { std::slice::from_raw_parts(items[0].value, items[0].value_len) };
        let val1 = unsafe { std::slice::from_raw_parts(items[1].value, items[1].value_len) };
        assert_eq!(val0, b"A");
        assert_eq!(val1, b"BB");
        unsafe { crate::memory::yeti_entries_free(e) };
    }

    #[test]
    fn entries_from_records_uses_empty_keys() {
        let e = YetiEntries::from_records(vec![b"r0".to_vec(), b"r1".to_vec()]);
        assert_eq!(e.len, 2);
        let items = unsafe { std::slice::from_raw_parts(e.items, e.len) };
        for it in items {
            let key = unsafe { std::ffi::CStr::from_ptr(it.key) };
            assert_eq!(key.to_str().unwrap(), "", "records carry no key");
        }
        unsafe { crate::memory::yeti_entries_free(e) };
    }

    #[test]
    fn status_maps_each_client_error_kind() {
        use client::ClientError;
        let conn = ClientError::Connection("x".into());
        let codec = ClientError::Codec("x".into());
        assert!(matches!(YetiStatus::of(&conn), YetiStatus::Connection));
        assert!(matches!(YetiStatus::of(&codec), YetiStatus::Codec));
    }
}
