//! Connection lifecycle exports: dial a node over TLS, release the handle, and
//! report the ABI version.

use std::ffi::{c_char, CString};

use yeti_sdk::remote as client;

use crate::ffi::YetiStatus;
use crate::registry::{clients, runtime};
use crate::util::{fail, str_arg};

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

/// Returns the ABI version of this binding (bump on breaking C-ABI changes).
#[no_mangle]
pub extern "C" fn yeti_abi_version() -> u32 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_version_is_one() {
        assert_eq!(yeti_abi_version(), 1);
    }

    #[test]
    fn closing_an_unknown_handle_is_a_noop() {
        // Idempotent: closing a handle that was never registered must not panic.
        unsafe { yeti_client_close(0) };
        unsafe { yeti_client_close(123_456) };
    }
}
