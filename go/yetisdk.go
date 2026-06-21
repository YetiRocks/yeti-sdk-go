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
	"fmt"
	"runtime"
	"unsafe"
)

// Severity mirrors the wire's integer log-level encoding.
type Severity uint8

const (
	SeverityTrace Severity = 0
	SeverityDebug Severity = 1
	SeverityInfo  Severity = 2
	SeverityWarn  Severity = 3
	SeverityError Severity = 4
)

// Entry is a (key, value) pair returned by the batch ops, ScanPrefix, and
// cursor pages.
type Entry struct {
	Key   string
	Value []byte
}

// Error is a SDK error carrying the kind discriminant from the Rust core so
// callers can branch (connection vs. remote vs. codec).
type Error struct {
	Kind    Kind
	Message string
}

func (e *Error) Error() string { return e.Message }

// Kind classifies an Error, mirroring yeti-sdk's ClientError.
type Kind int

const (
	KindConnection Kind = -1
	KindRemote     Kind = -2
	KindCodec      Kind = -3
	KindBadHandle  Kind = -4
)

// statusErr turns a non-OK C status + owned error string into a Go *Error,
// freeing the C string.
func statusErr(status C.YetiStatus, cerr *C.char) error {
	if status == C.Ok {
		return nil
	}
	msg := ""
	if cerr != nil {
		msg = C.GoString(cerr)
		C.yeti_string_free(cerr)
	}
	if msg == "" {
		msg = fmt.Sprintf("yeti sdk error (status %d)", int(status))
	}
	return &Error{Kind: Kind(int(status)), Message: msg}
}

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
	c := &Client{handle: handle}
	// Best-effort release if the caller forgets to Close.
	runtime.SetFinalizer(c, func(c *Client) { C.yeti_client_close(c.handle) })
	return c, nil
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
	c := &Client{handle: handle}
	// Best-effort release if the caller forgets to Close.
	runtime.SetFinalizer(c, func(c *Client) { C.yeti_client_close(c.handle) })
	return c, nil
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

// Store is the typed persistence facade over the canonical table-aware
// persistence/store contract.
type Store struct{ client C.uint64_t }

// cString allocates a C string the caller must free.
func cString(s string) *C.char { return C.CString(s) }

// Put stores value under (table, key).
func (s *Store) Put(table, key string, value []byte) error {
	ctable, ckey := cString(table), cString(key)
	defer C.free(unsafe.Pointer(ctable))
	defer C.free(unsafe.Pointer(ckey))
	vptr, vlen := bytesPtr(value)
	var cerr *C.char
	status := C.yeti_store_put(s.client, ctable, ckey, vptr, vlen, &cerr)
	return statusErr(status, cerr)
}

// Get fetches the bytes under (table, key). The bool reports presence; an
// absent key returns (nil, false, nil).
func (s *Store) Get(table, key string) ([]byte, bool, error) {
	ctable, ckey := cString(table), cString(key)
	defer C.free(unsafe.Pointer(ctable))
	defer C.free(unsafe.Pointer(ckey))
	var out C.YetiBytes
	var cerr *C.char
	status := C.yeti_store_get(s.client, ctable, ckey, &out, &cerr)
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

// PutIfAbsent stores value under (table, key) only if absent; reports whether
// it was inserted.
func (s *Store) PutIfAbsent(table, key string, value []byte) (bool, error) {
	ctable, ckey := cString(table), cString(key)
	defer C.free(unsafe.Pointer(ctable))
	defer C.free(unsafe.Pointer(ckey))
	vptr, vlen := bytesPtr(value)
	var out C.bool
	var cerr *C.char
	status := C.yeti_store_put_if_absent(s.client, ctable, ckey, vptr, vlen, &out, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return false, err
	}
	return bool(out), nil
}

// Delete removes (table, key); reports whether the key existed.
func (s *Store) Delete(table, key string) (bool, error) {
	ctable, ckey := cString(table), cString(key)
	defer C.free(unsafe.Pointer(ctable))
	defer C.free(unsafe.Pointer(ckey))
	var out C.bool
	var cerr *C.char
	status := C.yeti_store_delete(s.client, ctable, ckey, &out, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return false, err
	}
	return bool(out), nil
}

// Exists reports whether (table, key) is present.
func (s *Store) Exists(table, key string) (bool, error) {
	ctable, ckey := cString(table), cString(key)
	defer C.free(unsafe.Pointer(ctable))
	defer C.free(unsafe.Pointer(ckey))
	var out C.bool
	var cerr *C.char
	status := C.yeti_store_exists(s.client, ctable, ckey, &out, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return false, err
	}
	return bool(out), nil
}

// ScanPrefix lists (key, value) entries under table starting with prefix.
func (s *Store) ScanPrefix(table, prefix string) ([]Entry, error) {
	ctable, cprefix := cString(table), cString(prefix)
	defer C.free(unsafe.Pointer(ctable))
	defer C.free(unsafe.Pointer(cprefix))
	var out C.YetiEntries
	var cerr *C.char
	status := C.yeti_store_scan_prefix(s.client, ctable, cprefix, &out, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return nil, err
	}
	entries := goEntries(out)
	C.yeti_entries_free(out)
	return entries, nil
}

// Scan lists the keys under table starting with prefix.
func (s *Store) Scan(table, prefix string) ([]string, error) {
	entries, err := s.ScanPrefix(table, prefix)
	if err != nil {
		return nil, err
	}
	keys := make([]string, len(entries))
	for i, e := range entries {
		keys[i] = e.Key
	}
	return keys, nil
}

// CountPrefix counts the keys under table starting with prefix.
func (s *Store) CountPrefix(table, prefix string) (uint32, error) {
	ctable, cprefix := cString(table), cString(prefix)
	defer C.free(unsafe.Pointer(ctable))
	defer C.free(unsafe.Pointer(cprefix))
	var out C.uint32_t
	var cerr *C.char
	status := C.yeti_store_count_prefix(s.client, ctable, cprefix, &out, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return 0, err
	}
	return uint32(out), nil
}

// GetMany fetches many keys at once; one slot per requested key, in order,
// nil where absent.
func (s *Store) GetMany(table string, keys []string) ([][]byte, error) {
	ctable := cString(table)
	defer C.free(unsafe.Pointer(ctable))
	ckeys, freeKeys := cStringArray(keys)
	defer freeKeys()
	var out C.YetiEntries
	var cerr *C.char
	status := C.yeti_store_get_many(s.client, ctable, ckeys, C.uintptr_t(len(keys)), &out, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return nil, err
	}
	entries := goEntries(out)
	C.yeti_entries_free(out)
	vals := make([][]byte, len(entries))
	for i, e := range entries {
		vals[i] = e.Value // nil for an absent key
	}
	return vals, nil
}

// PutMany stores many (key, value) entries under table in one call.
func (s *Store) PutMany(table string, entries []Entry) error {
	ctable := cString(table)
	defer C.free(unsafe.Pointer(ctable))

	n := len(entries)
	keys := make([]string, n)
	for i, e := range entries {
		keys[i] = e.Key
	}
	ckeys, freeKeys := cStringArray(keys)
	defer freeKeys()

	// Parallel value pointer / length arrays.
	cvals := make([]*C.uint8_t, n)
	clens := make([]C.uintptr_t, n)
	pins := make([]unsafe.Pointer, 0, n)
	defer func() {
		for _, p := range pins {
			C.free(p)
		}
	}()
	for i, e := range entries {
		if len(e.Value) == 0 {
			cvals[i] = nil
			clens[i] = 0
			continue
		}
		buf := C.CBytes(e.Value)
		pins = append(pins, buf)
		cvals[i] = (*C.uint8_t)(buf)
		clens[i] = C.uintptr_t(len(e.Value))
	}
	var valsPtr **C.uint8_t
	var lensPtr *C.uintptr_t
	if n > 0 {
		valsPtr = &cvals[0]
		lensPtr = &clens[0]
	}
	var cerr *C.char
	status := C.yeti_store_put_many(s.client, ctable, ckeys, valsPtr, lensPtr, C.uintptr_t(n), &cerr)
	return statusErr(status, cerr)
}

// DeleteMany deletes many keys under table; returns how many existed.
func (s *Store) DeleteMany(table string, keys []string) (uint32, error) {
	ctable := cString(table)
	defer C.free(unsafe.Pointer(ctable))
	ckeys, freeKeys := cStringArray(keys)
	defer freeKeys()
	var out C.uint32_t
	var cerr *C.char
	status := C.yeti_store_delete_many(s.client, ctable, ckeys, C.uintptr_t(len(keys)), &out, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return 0, err
	}
	return uint32(out), nil
}

// OpenScan opens a paged scan cursor over (table, prefix). The server holds
// the scan as a connection-scoped resource; Cursor.Next pulls pages.
func (s *Store) OpenScan(table, prefix string) (*Cursor, error) {
	ctable, cprefix := cString(table), cString(prefix)
	defer C.free(unsafe.Pointer(ctable))
	defer C.free(unsafe.Pointer(cprefix))
	var handle C.uint64_t
	var cerr *C.char
	status := C.yeti_store_open_scan(s.client, ctable, cprefix, &handle, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return nil, err
	}
	return &Cursor{handle: handle}, nil
}

// Query opens a FIQL filter query over (table, fiql). The peer holds the query
// as a connection-scoped resource; refine it with SortBy/Limit/Offset, then
// materialize it with Execute/Count/Each. Close releases it early.
func (s *Store) Query(table, fiql string) (*Query, error) {
	ctable, cfiql := cString(table), cString(fiql)
	defer C.free(unsafe.Pointer(ctable))
	defer C.free(unsafe.Pointer(cfiql))
	var handle C.uint64_t
	var cerr *C.char
	status := C.yeti_store_open_query(s.client, ctable, cfiql, &handle, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return nil, err
	}
	return &Query{handle: handle}, nil
}

// Cursor is a paged scan cursor over a server-held open-scan resource. An
// exhausted cursor is released server-side automatically; Close an abandoned
// one early.
type Cursor struct {
	handle C.uint64_t
	done   bool
}

// Next pulls the next page (up to max entries). It returns (nil, nil) once the
// cursor is exhausted.
func (c *Cursor) Next(max uint32) ([]Entry, error) {
	if c.done {
		return nil, nil
	}
	var out C.YetiEntries
	var done C.bool
	var cerr *C.char
	status := C.yeti_cursor_next(c.handle, C.uint32_t(max), &out, &done, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return nil, err
	}
	if bool(done) {
		c.done = true
		C.yeti_entries_free(out)
		return nil, nil
	}
	entries := goEntries(out)
	C.yeti_entries_free(out)
	return entries, nil
}

// Close releases the server-side resource early (idempotent).
func (c *Cursor) Close() error {
	if c.done {
		return nil
	}
	c.done = true
	var cerr *C.char
	status := C.yeti_cursor_close(c.handle, &cerr)
	return statusErr(status, cerr)
}

// Query is a FIQL filter query held as a connection-scoped resource on the
// peer. SortBy/Limit/Offset refine the builder; Execute/Count materialize it
// and Each streams it. The handle persists across terminal calls (Execute then
// Count the same query); Close releases the peer-side resource early.
type Query struct {
	handle C.uint64_t
	closed bool
}

// SortBy adds a sort key (applied in call order); ascending picks the
// direction.
func (q *Query) SortBy(field string, ascending bool) error {
	cfield := cString(field)
	defer C.free(unsafe.Pointer(cfield))
	var cerr *C.char
	status := C.yeti_query_sort_by(q.handle, cfield, C.bool(ascending), &cerr)
	return statusErr(status, cerr)
}

// Limit caps the number of records the query returns.
func (q *Query) Limit(n uint32) error {
	var cerr *C.char
	status := C.yeti_query_limit(q.handle, C.uint32_t(n), &cerr)
	return statusErr(status, cerr)
}

// Offset skips the first n matched records.
func (q *Query) Offset(n uint32) error {
	var cerr *C.char
	status := C.yeti_query_offset(q.handle, C.uint32_t(n), &cerr)
	return statusErr(status, cerr)
}

// Execute materializes the matched records, each as the JSON bytes the SDK
// decodes.
func (q *Query) Execute() ([][]byte, error) {
	var out C.YetiEntries
	var cerr *C.char
	status := C.yeti_query_execute(q.handle, &out, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return nil, err
	}
	entries := goEntries(out)
	C.yeti_entries_free(out)
	records := make([][]byte, len(entries))
	for i, e := range entries {
		records[i] = e.Value
	}
	return records, nil
}

// Count returns the number of matched records.
func (q *Query) Count() (uint32, error) {
	var out C.uint32_t
	var cerr *C.char
	status := C.yeti_query_count(q.handle, &out, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return 0, err
	}
	return uint32(out), nil
}

// Each opens the streaming variant of Execute: a RecordStream draining one
// record (JSON bytes) per element.
func (q *Query) Each() (*RecordStream, error) {
	var handle C.uint64_t
	var cerr *C.char
	status := C.yeti_query_each(q.handle, &handle, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return nil, err
	}
	return &RecordStream{handle: handle}, nil
}

// Close releases the peer-side query resource early (idempotent).
func (q *Query) Close() error {
	if q.closed {
		return nil
	}
	q.closed = true
	var cerr *C.char
	status := C.yeti_query_close(q.handle, &cerr)
	return statusErr(status, cerr)
}

// RecordStream drains a peer-held Query.Each, one record (JSON bytes) per
// element. Next returns (nil, nil) once the peer closes the stream. Close frees
// the local receiver early.
type RecordStream struct {
	handle C.uint64_t
	done   bool
}

// Next pulls the next record. It returns (nil, nil) once the stream ends.
func (r *RecordStream) Next() ([]byte, error) {
	if r.done {
		return nil, nil
	}
	var out C.YetiBytes
	var done C.bool
	var cerr *C.char
	status := C.yeti_record_stream_next(r.handle, &out, &done, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return nil, err
	}
	if bool(done) {
		r.done = true
		C.yeti_bytes_free(out)
		return nil, nil
	}
	v := goBytes(out)
	C.yeti_bytes_free(out)
	return v, nil
}

// Close frees the stream's local receiver (idempotent).
func (r *RecordStream) Close() {
	if r.done {
		return
	}
	r.done = true
	C.yeti_record_stream_close(r.handle)
}

// Publisher is the fire-and-forget publish facade over messaging/publisher.
type Publisher struct{ client C.uint64_t }

// PubsubOp is the pubsub operation kind shared by Publish and Event.
type PubsubOp uint8

const (
	PubsubCreate PubsubOp = 0
	PubsubUpdate PubsubOp = 1
	PubsubDelete PubsubOp = 2
)

// Publish fires a change out to a table's subscribers on the peer
// (fire-and-forget).
func (p *Publisher) Publish(table string, op PubsubOp, key string, value []byte) error {
	ctable, ckey := cString(table), cString(key)
	defer C.free(unsafe.Pointer(ctable))
	defer C.free(unsafe.Pointer(ckey))
	vptr, vlen := bytesPtr(value)
	var cerr *C.char
	status := C.yeti_publish(p.client, ctable, C.uint8_t(op), ckey, vptr, vlen, &cerr)
	return statusErr(status, cerr)
}

// Subscriber is the subscribe facade over messaging/subscriber.
type Subscriber struct{ client C.uint64_t }

// Subscribe opens a table's update stream on the peer. filter is an optional
// server-side FIQL expression; pass "" to subscribe to every change.
func (s *Subscriber) Subscribe(table, filter string) (*Subscription, error) {
	ctable := cString(table)
	defer C.free(unsafe.Pointer(ctable))
	// An empty filter is "subscribe to all" — pass a null C pointer (the C-ABI
	// reads null as None), not an empty C string.
	var cfilter *C.char
	if filter != "" {
		cfilter = cString(filter)
		defer C.free(unsafe.Pointer(cfilter))
	}
	var handle C.uint64_t
	var cerr *C.char
	status := C.yeti_subscribe(s.client, ctable, cfilter, &handle, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return nil, err
	}
	return &Subscription{handle: handle}, nil
}

// Event is one change delivered over a Subscription.
type Event struct {
	Op          PubsubOp
	Key         string
	Value       []byte
	TimestampMs uint64
}

// Subscription drains a peer-held subscribe stream, one Event per change. Next
// returns (nil, nil) once the peer closes the stream. Close frees the local
// receiver early.
type Subscription struct {
	handle C.uint64_t
	done   bool
}

// Next pulls the next event. It returns (nil, nil) once the subscription ends.
func (s *Subscription) Next() (*Event, error) {
	if s.done {
		return nil, nil
	}
	var op C.uint8_t
	var ckey *C.char
	var value C.YetiBytes
	var ts C.uint64_t
	var done C.bool
	var cerr *C.char
	status := C.yeti_subscription_next(s.handle, &op, &ckey, &value, &ts, &done, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return nil, err
	}
	if bool(done) {
		s.done = true
		return nil, nil
	}
	key := ""
	if ckey != nil {
		key = C.GoString(ckey)
		C.yeti_string_free(ckey)
	}
	v := goBytes(value)
	C.yeti_bytes_free(value)
	return &Event{
		Op:          PubsubOp(op),
		Key:         key,
		Value:       v,
		TimestampMs: uint64(ts),
	}, nil
}

// Close frees the subscription's local receiver (idempotent).
func (s *Subscription) Close() {
	if s.done {
		return
	}
	s.done = true
	C.yeti_subscription_close(s.handle)
}

// RateLimit is the rate-limit facade over ingress/rate-limit.
type RateLimit struct{ client C.uint64_t }

// RateDecision is the outcome of a Check. Allowed reports whether a token was
// available and consumed; Remaining is the tokens left; RetryAfterMs is the
// suggested wait before retrying when denied (0 when allowed).
type RateDecision struct {
	Allowed      bool
	Remaining    uint32
	RetryAfterMs uint32
}

// Check consumes one token from the peer's (bucket, key) slot.
func (r *RateLimit) Check(bucket, key string) (RateDecision, error) {
	cbucket, ckey := cString(bucket), cString(key)
	defer C.free(unsafe.Pointer(cbucket))
	defer C.free(unsafe.Pointer(ckey))
	var out C.YetiRateDecision
	var cerr *C.char
	status := C.yeti_ratelimit_check(r.client, cbucket, ckey, &out, &cerr)
	if err := statusErr(status, cerr); err != nil {
		return RateDecision{}, err
	}
	return RateDecision{
		Allowed:      bool(out.allowed),
		Remaining:    uint32(out.remaining),
		RetryAfterMs: uint32(out.retry_after_ms),
	}, nil
}

// Log is the typed logging facade over telemetry/logs.
type Log struct{ client C.uint64_t }

// Field is one structured log field (a flat string key/value pair).
type Field struct {
	Key   string
	Value string
}

// Emit ships one log record (fire-and-acknowledge).
func (l *Log) Emit(severity Severity, target, message string, fields []Field) error {
	ctarget, cmessage := cString(target), cString(message)
	defer C.free(unsafe.Pointer(ctarget))
	defer C.free(unsafe.Pointer(cmessage))

	keys := make([]string, len(fields))
	vals := make([]string, len(fields))
	for i, f := range fields {
		keys[i] = f.Key
		vals[i] = f.Value
	}
	ckeys, freeKeys := cStringArray(keys)
	defer freeKeys()
	cvals, freeVals := cStringArray(vals)
	defer freeVals()

	var cerr *C.char
	status := C.yeti_log_emit(
		l.client, C.uint8_t(severity), ctarget, cmessage,
		ckeys, cvals, C.uintptr_t(len(fields)), &cerr,
	)
	return statusErr(status, cerr)
}

// AbiVersion reports the C-ABI version of the linked native binding.
func AbiVersion() uint32 { return uint32(C.yeti_abi_version()) }

// ── helpers ──────────────────────────────────────────────────────────────

// bytesPtr returns a C pointer + length for a Go byte slice without copying.
// The slice must outlive the call (it does: the call is synchronous and the
// slice is a live argument).
func bytesPtr(b []byte) (*C.uint8_t, C.uintptr_t) {
	if len(b) == 0 {
		return nil, 0
	}
	return (*C.uint8_t)(unsafe.Pointer(&b[0])), C.uintptr_t(len(b))
}

// goBytes copies a YetiBytes value into a Go slice.
func goBytes(b C.YetiBytes) []byte {
	if b.ptr == nil || b.len == 0 {
		return []byte{}
	}
	return C.GoBytes(unsafe.Pointer(b.ptr), C.int(b.len))
}

// goEntries copies a YetiEntries array into a Go slice. An entry with a null
// value (e.g. a GetMany miss) becomes Entry{Key, nil}.
func goEntries(e C.YetiEntries) []Entry {
	n := int(e.len)
	if e.items == nil || n == 0 {
		return nil
	}
	items := unsafe.Slice(e.items, n)
	out := make([]Entry, n)
	for i := 0; i < n; i++ {
		it := items[i]
		key := ""
		if it.key != nil {
			key = C.GoString(it.key)
		}
		var val []byte
		if it.value != nil && it.value_len > 0 {
			val = C.GoBytes(unsafe.Pointer(it.value), C.int(it.value_len))
		}
		out[i] = Entry{Key: key, Value: val}
	}
	return out
}

// cStringArray builds a C array of C strings from a Go slice, returning the
// array pointer and a free func that releases every string and the array.
func cStringArray(ss []string) (**C.char, func()) {
	if len(ss) == 0 {
		return nil, func() {}
	}
	arr := make([]*C.char, len(ss))
	for i, s := range ss {
		arr[i] = C.CString(s)
	}
	free := func() {
		for _, p := range arr {
			C.free(unsafe.Pointer(p))
		}
	}
	return &arr[0], free
}
