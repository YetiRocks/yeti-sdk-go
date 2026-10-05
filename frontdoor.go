// Front door (ADR-030, YTC-570) — the authenticated /yeti-sdk/ws channel every
// running node serves.
//
// Where Connect/ConnectPinned in yetisdk.go dial the back-door host-RPC socket
// (which a deployed node does not expose), ConfigureFrontDoor dials the node's
// public edge: a wss upgrade to /yeti-sdk/ws, falling down the transport ladder
// to H2 / REST+SSE when the network forbids a WebSocket. The returned FrontDoor
// derives Table(app, name) with Get/GetAll/Put/Delete, plus Call and Subscribe.
//
// Values on this surface are the JSON record bytes the REST surface returns —
// not the back door's named-MessagePack — so a Go process marshals with
// encoding/json.

package yetisdk

/*
#cgo CFLAGS: -I${SRCDIR}
#include <stdlib.h>
#include "yeti_sdk_go.h"
*/
import "C"

import (
	"runtime"
	"unsafe"
)

// FrontDoor is one front-door connection — the Go-owned handle holding the
// persistent WebSocket (or the H2 / REST+SSE carrier the ladder settled on).
// The carrier is shared, so many tables (and Call / Subscribe) ride one channel.
// A FrontDoor is safe for concurrent use; the C ABI serializes blocking calls
// onto a shared runtime.
type FrontDoor struct {
	handle C.uint64_t
}

// ConfigureFrontDoor dials a running node's front door trusting the public
// WebPKI root store (a node whose cert chains to a public CA — the
// deployed-cloud path).
//
// url is "wss://host:port/yeti-sdk/ws"; serverName is verified against the
// node's leaf. Supply at most one credential: apiKey (one-shot Bearer; the host
// mints a session that SessionToken returns) or session (resume a held token);
// pass "" for both to bind anonymous (public ops only). The dial walks the
// Transport::Auto ladder (WebSocket → H2 → REST+SSE).
func ConfigureFrontDoor(url, serverName, apiKey, session string) (*FrontDoor, error) {
	curl, cname := cString(url), cString(serverName)
	defer C.free(unsafe.Pointer(curl))
	defer C.free(unsafe.Pointer(cname))
	ckey, freeKey := optCString(apiKey)
	defer freeKey()
	csess, freeSess := optCString(session)
	defer freeSess()
	var handle C.uint64_t
	var cerr *C.YetiError
	status := C.yeti_front_door_configure(curl, cname, ckey, csess, &handle, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return nil, err
	}
	return newFrontDoor(handle), nil
}

// ConfigureFrontDoorPinned dials a running node's front door trusting exactly
// the CA cert(s) in caPem instead of the public WebPKI root store — the dev /
// self-host path where the node presents a certificate signed by a private CA
// (e.g. an mkcert root) that the public roots reject.
//
// See ConfigureFrontDoor for the url, serverName, and credential rules. The dial
// stays TLS-only (no plaintext, no skip-verify): a bad or empty CA PEM fails
// before the dial.
func ConfigureFrontDoorPinned(url, serverName, caPem, apiKey, session string) (*FrontDoor, error) {
	curl, cname, cca := cString(url), cString(serverName), cString(caPem)
	defer C.free(unsafe.Pointer(curl))
	defer C.free(unsafe.Pointer(cname))
	defer C.free(unsafe.Pointer(cca))
	ckey, freeKey := optCString(apiKey)
	defer freeKey()
	csess, freeSess := optCString(session)
	defer freeSess()
	var handle C.uint64_t
	var cerr *C.YetiError
	status := C.yeti_front_door_configure_pinned(curl, cname, cca, ckey, csess, &handle, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return nil, err
	}
	return newFrontDoor(handle), nil
}

// ConfigureFrontDoorMtls dials a running node's front door with mutual TLS —
// presenting clientCertPem/clientKeyPem as the client identity (the leaf a
// require_auth peer pins tenant authorization on) and trusting caPem as the root
// that signs the node's server cert. The mTLS twin of ConfigureFrontDoorPinned.
// apiKey/session still apply on top (the app-level identity).
func ConfigureFrontDoorMtls(url, serverName, clientCertPem, clientKeyPem, caPem, apiKey, session string) (*FrontDoor, error) {
	curl, cname := cString(url), cString(serverName)
	defer C.free(unsafe.Pointer(curl))
	defer C.free(unsafe.Pointer(cname))
	ccert, ckeyPem, cca := cString(clientCertPem), cString(clientKeyPem), cString(caPem)
	defer C.free(unsafe.Pointer(ccert))
	defer C.free(unsafe.Pointer(ckeyPem))
	defer C.free(unsafe.Pointer(cca))
	capi, freeKey := optCString(apiKey)
	defer freeKey()
	csess, freeSess := optCString(session)
	defer freeSess()
	var handle C.uint64_t
	var cerr *C.YetiError
	status := C.yeti_front_door_configure_mtls(curl, cname, ccert, ckeyPem, cca, capi, csess, &handle, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return nil, err
	}
	return newFrontDoor(handle), nil
}

func newFrontDoor(handle C.uint64_t) *FrontDoor {
	fd := &FrontDoor{handle: handle}
	// Best-effort release if the caller forgets to Close.
	runtime.SetFinalizer(fd, func(fd *FrontDoor) { C.yeti_front_door_close(fd.handle) })
	return fd
}

// Close releases the front-door connection (idempotent).
func (f *FrontDoor) Close() {
	C.yeti_front_door_close(f.handle)
	runtime.SetFinalizer(f, nil)
}

// SessionToken returns the session token bound to this connection (minted on
// one-shot auth or resumed), or "" for an anonymous connection. Store it to
// reconnect the same identity without re-presenting credentials
// (ConfigureFrontDoor with session).
func (f *FrontDoor) SessionToken() (string, error) {
	var ctok *C.char
	var cerr *C.YetiError
	status := C.yeti_front_door_session_token(f.handle, &ctok, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return "", err
	}
	if ctok == nil {
		return "", nil
	}
	tok := C.GoString(ctok)
	C.yeti_string_free(ctok)
	return tok, nil
}

// ActiveTransport reports which rung the ladder settled on: "WebSocket", "H2",
// or "RestSse".
func (f *FrontDoor) ActiveTransport() (string, error) {
	var cname *C.char
	var cerr *C.YetiError
	status := C.yeti_front_door_active_transport(f.handle, &cname, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return "", err
	}
	name := C.GoString(cname)
	C.yeti_string_free(cname)
	return name, nil
}

// Table returns a handle on (app, name) over this connection — routed as
// /{app}/{name} on whichever rung the ladder negotiated. name may carry path
// segments (e.g. "api/Todo") to address a dynamic app's REST mount.
func (f *FrontDoor) Table(app, name string) *FrontDoorTable {
	return &FrontDoorTable{fd: f.handle, app: app, name: name}
}

// Call invokes a custom resource over the front door — POST /{app}/{resource}
// (or /{app}/{resource}/{key} when key is non-empty), routed exactly as a REST
// call. It returns the host's response body bytes.
func (f *FrontDoor) Call(app, resource, key string, payload []byte) ([]byte, error) {
	capp, cres := cString(app), cString(resource)
	defer C.free(unsafe.Pointer(capp))
	defer C.free(unsafe.Pointer(cres))
	ckey, freeKey := optCString(key)
	defer freeKey()
	ptr, n := bytesPtr(payload)
	var out C.YetiBytes
	var cerr *C.YetiError
	status := C.yeti_front_door_call(f.handle, capp, cres, ckey, ptr, n, &out, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return nil, err
	}
	v := goBytes(out)
	C.yeti_bytes_free(out)
	return v, nil
}

// Subscribe opens a table's change stream over the front door
// (GET /{app}/{table}?stream=sse); rows arrive as stream_item push frames.
// query is an optional raw URL query string ("limit=5"); pass "" to subscribe to
// every change.
func (f *FrontDoor) Subscribe(app, table, query string) (*FrontDoorSubscription, error) {
	capp, ctable := cString(app), cString(table)
	defer C.free(unsafe.Pointer(capp))
	defer C.free(unsafe.Pointer(ctable))
	cquery, freeQuery := optCString(query)
	defer freeQuery()
	var handle C.uint64_t
	var cerr *C.YetiError
	status := C.yeti_front_door_subscribe(f.handle, capp, ctable, cquery, &handle, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return nil, err
	}
	return &FrontDoorSubscription{handle: handle}, nil
}

// FrontDoorTable is a single (app, table) over the front-door channel — the
// front-door twin of Store. Values are the JSON record bytes the REST surface
// returns.
type FrontDoorTable struct {
	fd   C.uint64_t
	app  string
	name string
}

// App returns the app id this handle's table is routed under.
func (t *FrontDoorTable) App() string { return t.app }

// Name returns the table name this handle is bound to.
func (t *FrontDoorTable) Name() string { return t.name }

// Get fetches the row JSON under (table, key). The bool reports presence; a
// missing record returns (nil, false, nil). A 404 for an app or route the node
// does not serve is an *Error (Status 404, IsRouteNotFound true), not absence.
func (t *FrontDoorTable) Get(key string) ([]byte, bool, error) {
	capp, cname, ckey := cString(t.app), cString(t.name), cString(key)
	defer C.free(unsafe.Pointer(capp))
	defer C.free(unsafe.Pointer(cname))
	defer C.free(unsafe.Pointer(ckey))
	var out C.YetiBytes
	var cerr *C.YetiError
	status := C.yeti_front_door_table_get(t.fd, capp, cname, ckey, &out, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return nil, false, err
	}
	if !bool(out.present) {
		return nil, false, nil
	}
	v := goBytes(out)
	C.yeti_bytes_free(out)
	return v, true, nil
}

// GetAll fetches the collection JSON, optionally filtered by a raw URL query
// string ("limit=5&offset=10"); pass "" for no filter.
func (t *FrontDoorTable) GetAll(query string) ([]byte, error) {
	capp, cname := cString(t.app), cString(t.name)
	defer C.free(unsafe.Pointer(capp))
	defer C.free(unsafe.Pointer(cname))
	cquery, freeQuery := optCString(query)
	defer freeQuery()
	var out C.YetiBytes
	var cerr *C.YetiError
	status := C.yeti_front_door_table_get_all(t.fd, capp, cname, cquery, &out, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return nil, err
	}
	v := goBytes(out)
	C.yeti_bytes_free(out)
	return v, nil
}

// Put writes the JSON record value. A non-empty key routes
// PUT /{app}/{table}/{key}; an empty one routes POST /{app}/{table}
// (server-assigned key). It returns the persisted row JSON.
func (t *FrontDoorTable) Put(key string, value []byte) ([]byte, error) {
	capp, cname := cString(t.app), cString(t.name)
	defer C.free(unsafe.Pointer(capp))
	defer C.free(unsafe.Pointer(cname))
	ckey, freeKey := optCString(key)
	defer freeKey()
	ptr, n := bytesPtr(value)
	var out C.YetiBytes
	var cerr *C.YetiError
	status := C.yeti_front_door_table_put(t.fd, capp, cname, ckey, ptr, n, &out, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return nil, err
	}
	v := goBytes(out)
	C.yeti_bytes_free(out)
	return v, nil
}

// Delete removes (table, key).
func (t *FrontDoorTable) Delete(key string) error {
	capp, cname, ckey := cString(t.app), cString(t.name), cString(key)
	defer C.free(unsafe.Pointer(capp))
	defer C.free(unsafe.Pointer(cname))
	defer C.free(unsafe.Pointer(ckey))
	var cerr *C.YetiError
	status := C.yeti_front_door_table_delete(t.fd, capp, cname, ckey, &cerr)
	return statusErr(status, cerr)
}

// FrontDoorSubscription drains a host-held subscribe stream, one row (JSON
// bytes) per stream_item push frame. Next returns (nil, nil) once the host
// closes the stream. Close frees the local receiver early.
type FrontDoorSubscription struct {
	handle C.uint64_t
	done   bool
}

// Next pulls the next row (JSON bytes). It returns (nil, nil) once the stream
// ends.
func (s *FrontDoorSubscription) Next() ([]byte, error) {
	if s.done {
		return nil, nil
	}
	var out C.YetiBytes
	var done C.bool
	var cerr *C.YetiError
	status := C.yeti_front_door_subscription_next(s.handle, &out, &done, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return nil, err
	}
	if bool(done) {
		s.done = true
		return nil, nil
	}
	v := goBytes(out)
	C.yeti_bytes_free(out)
	return v, nil
}

// Close frees the subscription's local receiver (idempotent).
func (s *FrontDoorSubscription) Close() {
	if s.done {
		return
	}
	s.done = true
	C.yeti_front_door_subscription_close(s.handle)
}

// optCString maps an empty Go string to a null C pointer (the C ABI reads null
// as None — a server-assigned key, an unfiltered query, an absent credential)
// and a non-empty one to an owned C string plus its free func.
func optCString(s string) (*C.char, func()) {
	if s == "" {
		return nil, func() {}
	}
	p := C.CString(s)
	return p, func() { C.free(unsafe.Pointer(p)) }
}
