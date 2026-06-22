//! Ingress exports: rate-limit `check` and structured log `emit`.

use std::ffi::c_char;

use yeti_sdk::remote as client;

use crate::ffi::{YetiRateDecision, YetiStatus};
use crate::registry::{log_of, rate_limit_of, runtime};
use crate::util::{bad_handle, fail, str_arg};

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
    let sev = severity_of(severity);
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

/// Map the wire severity integer onto the SDK [`Severity`](client::Severity),
/// defaulting any unrecognized value to `Info` (mirrors the napi binding).
fn severity_of(severity: u8) -> client::Severity {
    match severity {
        0 => client::Severity::Trace,
        1 => client::Severity::Debug,
        3 => client::Severity::Warn,
        4 => client::Severity::Error,
        _ => client::Severity::Info,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn severity_maps_known_values_and_defaults_to_info() {
        assert!(matches!(severity_of(0), client::Severity::Trace));
        assert!(matches!(severity_of(1), client::Severity::Debug));
        assert!(matches!(severity_of(2), client::Severity::Info));
        assert!(matches!(severity_of(3), client::Severity::Warn));
        assert!(matches!(severity_of(4), client::Severity::Error));
        // 5 is out of range — the wire never sends it, but the mapping must not
        // panic; it collapses to Info like the other bindings.
        assert!(matches!(severity_of(99), client::Severity::Info));
    }

    #[test]
    fn ratelimit_on_dead_client_is_bad_handle() {
        let bucket = CString::new("b").unwrap();
        let key = CString::new("k").unwrap();
        let mut out = YetiRateDecision {
            allowed: false,
            remaining: 0,
            retry_after_ms: 0,
        };
        let mut err: *mut c_char = std::ptr::null_mut();
        let status =
            unsafe { yeti_ratelimit_check(0, bucket.as_ptr(), key.as_ptr(), &mut out, &mut err) };
        assert!(matches!(status, YetiStatus::BadHandle));
        unsafe { crate::memory::yeti_string_free(err) };
    }

    #[test]
    fn log_emit_on_dead_client_is_bad_handle() {
        let target = CString::new("t").unwrap();
        let message = CString::new("m").unwrap();
        let mut err: *mut c_char = std::ptr::null_mut();
        let status = unsafe {
            yeti_log_emit(
                0,
                2,
                target.as_ptr(),
                message.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                &mut err,
            )
        };
        assert!(matches!(status, YetiStatus::BadHandle));
        unsafe { crate::memory::yeti_string_free(err) };
    }
}
