// Package yetisdk is the Go binding for the Yeti SDK — idiomatic Go over the
// shared yeti-sdk Rust core via cgo (ADR-025, YTC-519).
//
// It is a thin wrapper: every method delegates across a C ABI into the same
// yeti-sdk implementation the napi (Node) and uniffi (Python/Swift/
// Kotlin) bindings wrap. One source, no drift. The widening surface lives in
// yeti-sdk; this package follows on rebuild.
//
// # Build
//
// The package links a static archive (libyeti_sdk_go.a) and its generated
// header (yeti_sdk_go.h), both staged into this directory by ../build.sh. Run
// that script before `go build`/`go test`. The cgo LDFLAGS below pull in the
// macOS/Linux system libraries the Rust core (tokio, rustls) needs.
//
// # Layout
//
// The package is split by capability, one concern per file:
//
//   - yetisdk.go    — the Client and its connect/facade surface (this file).
//   - errors.go     — Error, Kind, Severity, and the C-status translation.
//   - store.go      — the Store persistence facade.
//   - cursor.go     — the paged scan Cursor.
//   - query.go      — the FIQL Query builder and its RecordStream.
//   - messaging.go  — Publisher, Subscriber, Subscription, Event.
//   - ratelimit.go  — the RateLimit facade and RateDecision.
//   - log.go        — the Log facade and Field.
//   - cgo_helpers.go — the C<->Go marshalling helpers and AbiVersion.
package yetisdk

/*
#cgo CFLAGS: -I${SRCDIR}
#cgo LDFLAGS: -L${SRCDIR} -lyeti_sdk_go
#cgo darwin LDFLAGS: -framework CoreFoundation -framework Security -framework SystemConfiguration -lc++
#cgo linux LDFLAGS: -lm -lpthread -ldl
#include <stdlib.h>
#include "yeti_sdk_go.h"
*/
import "C"

import (
	"runtime"
	"unsafe"
)

// Client is one connection to a Yeti host-rpc endpoint. Its capability facades
// (Store, Log, Publisher, Subscriber, RateLimit) share the multiplexed
// connection. A Client is safe for concurrent use; the C ABI serializes
// blocking calls onto a shared runtime.
type Client struct {
	handle C.uint64_t
}

// Connect dials a Yeti host-rpc endpoint over TLS (e.g. "127.0.0.1:7799").
//
// serverName is the hostname verified against the peer's leaf certificate. The
// dial is TLS-only (the SDK connect has no plaintext path): trust is the public
// WebPKI root store, so the peer must present a certificate chaining to a public
// CA — mirroring yeti-sdk-node's minimal connect(addr, serverName) surface.
func Connect(addr, serverName string) (*Client, error) {
	caddr := C.CString(addr)
	defer C.free(unsafe.Pointer(caddr))
	cname := C.CString(serverName)
	defer C.free(unsafe.Pointer(cname))
	var handle C.uint64_t
	var cerr *C.char
	status := C.yeti_connect(caddr, cname, &handle, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return nil, err
	}
	return newClient(handle), nil
}

// ConnectPinned dials a Yeti host-rpc endpoint over TLS, trusting exactly the
// CA cert(s) in caPem instead of the public WebPKI root store — the dev /
// self-host path where the node presents a certificate signed by a private CA
// (e.g. an mkcert root) that the public roots reject.
//
// serverName is the hostname verified against the peer's leaf certificate. The
// dial stays TLS-only (no plaintext, no skip-verify path): a bad CA PEM or an
// untrusted peer fails. Use Connect for a peer whose certificate chains to a
// public CA.
func ConnectPinned(addr, serverName, caPem string) (*Client, error) {
	caddr := C.CString(addr)
	defer C.free(unsafe.Pointer(caddr))
	cname := C.CString(serverName)
	defer C.free(unsafe.Pointer(cname))
	cca := C.CString(caPem)
	defer C.free(unsafe.Pointer(cca))
	var handle C.uint64_t
	var cerr *C.char
	status := C.yeti_connect_pinned(caddr, cname, cca, &handle, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return nil, err
	}
	return newClient(handle), nil
}

// newClient wraps a live native handle and arms a best-effort finalizer that
// releases it if the caller forgets to Close.
func newClient(handle C.uint64_t) *Client {
	c := &Client{handle: handle}
	runtime.SetFinalizer(c, func(c *Client) { C.yeti_client_close(c.handle) })
	return c
}

// Close releases the connection and its handle (idempotent).
func (c *Client) Close() {
	if c.handle != 0 {
		C.yeti_client_close(c.handle)
		c.handle = 0
		runtime.SetFinalizer(c, nil)
	}
}

// Store returns the persistence/store capability over this connection.
func (c *Client) Store() *Store { return &Store{client: c.handle} }

// Log returns the telemetry/logs capability over this connection.
func (c *Client) Log() *Log { return &Log{client: c.handle} }

// Publisher returns the messaging/publisher capability over this connection —
// fire a change out to a table's subscribers on the peer.
func (c *Client) Publisher() *Publisher { return &Publisher{client: c.handle} }

// Subscriber returns the messaging/subscriber capability over this connection —
// open a table's update stream on the peer and drain its events.
func (c *Client) Subscriber() *Subscriber { return &Subscriber{client: c.handle} }

// RateLimit returns the ingress/rate-limit capability over this connection —
// consume a token from the peer's (bucket, key) slot.
func (c *Client) RateLimit() *RateLimit { return &RateLimit{client: c.handle} }
