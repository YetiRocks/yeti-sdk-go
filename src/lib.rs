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
//! process-global registries (one for [`Client`](yeti_sdk::remote::Client), one
//! per streaming resource). The async core is driven to completion on a shared
//! multi-thread tokio runtime; every exported call is blocking from Go's view.
//!
//! Fallible calls return a [`YetiStatus`](ffi::YetiStatus) (0 = ok) and, on
//! error, write an owned C string into the `err` out-param that Go copies then
//! frees with `yeti_string_free`. Byte payloads come back as a
//! [`YetiBytes`](ffi::YetiBytes) the caller frees with `yeti_bytes_free`. The
//! widening surface lives in `yeti-sdk`; this binding follows on rebuild.
//!
//! ## Module map
//!
//! - [`ffi`] — the `#[repr(C)]` value types that cross the boundary.
//! - [`registry`] — the shared tokio runtime and the opaque-handle registries.
//! - [`util`] — argument coercion and error-reporting helpers.
//! - [`connection`] — `yeti_connect` / `yeti_connect_pinned` / close / version.
//! - [`store`] — single-key, scan/count, and batch ops.
//! - [`cursor`] — the paged scan cursor.
//! - [`query`] — the FIQL query builder and record stream.
//! - [`messaging`] — publish and the subscription stream.
//! - [`ingress`] — rate-limit check and structured log emit.
//! - [`memory`] — the buffer-release exports.

// The C ABI is inherently unsafe (raw pointers, ownership transfer across the
// boundary). The safety contract for every exported fn is documented at its
// site; callers (the Go package) honor it.
#![allow(clippy::missing_safety_doc)]

pub mod connection;
pub mod cursor;
pub mod ffi;
pub mod ingress;
pub mod memory;
pub mod messaging;
pub mod query;
pub mod registry;
pub mod store;
pub mod util;
