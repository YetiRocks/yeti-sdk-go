//! Messaging exports: fire-and-forget publish and the table subscription
//! stream.

use std::ffi::{c_char, CString};

use crate::ffi::{YetiBytes, YetiStatus};
use crate::registry::{publisher_of, runtime, subscriber_of, subscriptions};
use crate::util::{bad_handle, bytes_arg, fail, str_arg};

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
/// owned C string the caller frees with [`crate::memory::yeti_string_free`];
/// `value` is a [`YetiBytes`] the caller frees with
/// [`crate::memory::yeti_bytes_free`]; `timestamp_ms` is the event time in
/// epoch milliseconds.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_on_dead_client_is_bad_handle() {
        let table = CString::new("t").unwrap();
        let key = CString::new("k").unwrap();
        let mut err: *mut c_char = std::ptr::null_mut();
        let status = unsafe {
            yeti_publish(
                0,
                table.as_ptr(),
                1,
                key.as_ptr(),
                std::ptr::null(),
                0,
                &mut err,
            )
        };
        assert!(matches!(status, YetiStatus::BadHandle));
        unsafe { crate::memory::yeti_string_free(err) };
    }

    #[test]
    fn subscription_next_on_dead_handle_is_bad_handle() {
        let mut op = 0u8;
        let mut key: *mut c_char = std::ptr::null_mut();
        let mut value = YetiBytes::absent();
        let mut ts = 0u64;
        let mut done = false;
        let mut err: *mut c_char = std::ptr::null_mut();
        let status = unsafe {
            yeti_subscription_next(
                555, &mut op, &mut key, &mut value, &mut ts, &mut done, &mut err,
            )
        };
        assert!(matches!(status, YetiStatus::BadHandle));
        unsafe { crate::memory::yeti_string_free(err) };
    }

    #[test]
    fn subscription_close_on_unknown_handle_is_a_noop() {
        unsafe { yeti_subscription_close(0) };
    }
}
