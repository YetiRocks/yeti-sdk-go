//! Argument-coercion and error-reporting helpers shared by the exports. Each
//! reads a raw C argument or writes an owned error string into an `err`
//! out-param — the bookkeeping every fallible export repeats.

use std::ffi::{c_char, CStr, CString};

use yeti_sdk::remote as client;

use crate::ffi::YetiStatus;

/// Write an owned C error string into `err` (if non-null) and return the
/// status for `e`.
///
/// # Safety
/// `err` is a valid writable pointer to a `*mut c_char`, or null.
pub(crate) unsafe fn fail(e: &client::ClientError, err: *mut *mut c_char) -> YetiStatus {
    if !err.is_null() {
        *err = CString::new(e.to_string()).unwrap_or_default().into_raw();
    }
    YetiStatus::of(e)
}

/// Coerce a NUL-terminated C string argument to a `&str`, treating null or
/// invalid UTF-8 as the empty string (never panic across the ABI).
///
/// # Safety
/// `p` is a valid NUL-terminated C string, or null.
pub(crate) unsafe fn str_arg<'a>(p: *const c_char) -> &'a str {
    if p.is_null() {
        return "";
    }
    CStr::from_ptr(p).to_str().unwrap_or("")
}

/// Coerce a `(ptr, len)` pair to a byte slice, treating null/empty as `&[]`.
///
/// # Safety
/// `p` points to `len` valid bytes, or is null.
pub(crate) unsafe fn bytes_arg<'a>(p: *const u8, len: usize) -> &'a [u8] {
    if p.is_null() || len == 0 {
        return &[];
    }
    std::slice::from_raw_parts(p, len)
}

/// Write the canonical "invalid client handle" error string into `err` and
/// return [`YetiStatus::BadHandle`].
///
/// # Safety
/// `err` is a valid writable pointer, or null.
pub(crate) unsafe fn bad_handle(err: *mut *mut c_char) -> YetiStatus {
    if !err.is_null() {
        *err = CString::new("invalid client handle")
            .unwrap_or_default()
            .into_raw();
    }
    YetiStatus::BadHandle
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn str_arg_reads_null_as_empty() {
        let s = unsafe { str_arg(std::ptr::null()) };
        assert_eq!(s, "");
    }

    #[test]
    fn str_arg_reads_a_c_string() {
        let c = CString::new("table").unwrap();
        let s = unsafe { str_arg(c.as_ptr()) };
        assert_eq!(s, "table");
    }

    #[test]
    fn bytes_arg_reads_null_and_empty_as_empty_slice() {
        assert_eq!(unsafe { bytes_arg(std::ptr::null(), 0) }, &[] as &[u8]);
        let buf = [1u8, 2, 3];
        assert_eq!(unsafe { bytes_arg(buf.as_ptr(), 0) }, &[] as &[u8]);
        assert_eq!(unsafe { bytes_arg(buf.as_ptr(), 3) }, &[1, 2, 3]);
    }

    #[test]
    fn bad_handle_writes_message_and_status() {
        let mut err: *mut c_char = std::ptr::null_mut();
        let status = unsafe { bad_handle(&mut err) };
        assert!(matches!(status, YetiStatus::BadHandle));
        assert!(!err.is_null());
        let msg = unsafe { CString::from_raw(err) };
        assert_eq!(msg.to_str().unwrap(), "invalid client handle");
    }

    #[test]
    fn bad_handle_tolerates_null_err() {
        let status = unsafe { bad_handle(std::ptr::null_mut()) };
        assert!(matches!(status, YetiStatus::BadHandle));
    }

    #[test]
    fn fail_writes_error_string_and_status() {
        let e = client::ClientError::Remote("nope".to_owned());
        let mut err: *mut c_char = std::ptr::null_mut();
        let status = unsafe { fail(&e, &mut err) };
        assert!(matches!(status, YetiStatus::Remote));
        let msg = unsafe { CString::from_raw(err) };
        assert_eq!(msg.to_str().unwrap(), "remote error: nope");
    }
}
