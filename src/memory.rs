//! Memory-release exports. Every owned buffer this crate hands to Go is
//! reclaimed through exactly one of these, matching the constructor that leaked
//! it in [`crate::ffi`].

use std::ffi::{c_char, c_void, CString};

use crate::ffi::{YetiBytes, YetiEntries};

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
            drop(Vec::from_raw_parts(
                item.value,
                item.value_len,
                item.value_len,
            ));
        }
    }
}

// Silence "unused" for the c_void import kept for header completeness.
#[allow(dead_code)]
fn _abi_void_anchor(_: *mut c_void) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_free_tolerates_null() {
        unsafe { yeti_string_free(std::ptr::null_mut()) };
    }

    #[test]
    fn bytes_free_tolerates_absent() {
        // An absent YetiBytes has a null ptr; freeing it is a no-op.
        unsafe { yeti_bytes_free(YetiBytes::absent()) };
    }

    #[test]
    fn entries_free_tolerates_empty() {
        unsafe { yeti_entries_free(YetiEntries::empty()) };
    }
}
