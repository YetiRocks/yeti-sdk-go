// Package yetisdk is the Go binding for the Yeti SDK — idiomatic Go over the
// shared yeti-sdk Rust core via cgo (ADR-025, YTC-519).
//
// It is a thin wrapper: every method delegates across a C ABI into the same
// yeti-sdk implementation the napi (Node) and uniffi (Python/Swift/
// Kotlin) bindings wrap. One source, no drift. The widening surface lives in
// yeti-sdk; this package follows on rebuild.
//
// The surface is the front door (ADR-030): ConfigureFrontDoor* in frontdoor.go
// dials a running node's authenticated /yeti-sdk/ws channel. This file holds
// the shared cgo scaffolding — error translation and the C-string / byte-slice
// marshalling those entry points share.
//
// # Build
//
// The package links a static archive (libyeti_sdk_go.a) and its generated
// header (yeti_sdk_go.h), both staged into this directory by sdk-cells go. Run
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
	"unsafe"
)

// Error is a SDK error carrying the kind discriminant from the Rust core so
// callers can branch (connection vs. remote vs. codec). For a KindRemote error
// the node's answer is data, not text (YTC-1890): Status is the HTTP status it
// answered with (0 when the host refused the op before HTTP dispatch), and
// ProblemType / Title / Detail are the RFC 9457 problem members ("" when the
// body was not a problem). Branch on these; never parse Message.
type Error struct {
	Kind        Kind
	Message     string
	Status      int
	ProblemType string
	Title       string
	Detail      string
	Body        string
}

func (e *Error) Error() string { return e.Message }

// IsRouteNotFound reports a 404 for an address the node does not serve (no
// such app, resource or route), as opposed to a missing record, which Get
// reports as absent rather than as an error.
func (e *Error) IsRouteNotFound() bool {
	return e.Kind == KindRemote && e.Status == 404 && e.ProblemType == RouteNotFoundType
}

// RouteNotFoundType is the problem type a node answers for an address it does
// not serve, read from the Rust core so the two cannot drift.
var RouteNotFoundType = C.GoString(C.yeti_route_not_found_type())

// Kind classifies an Error, mirroring yeti-sdk's ClientError.
type Kind int

const (
	KindConnection Kind = -1
	KindRemote     Kind = -2
	KindCodec      Kind = -3
	KindBadHandle  Kind = -4
)

// goStringOrEmpty copies a nullable C string.
func goStringOrEmpty(s *C.char) string {
	if s == nil {
		return ""
	}
	return C.GoString(s)
}

// statusErr turns a non-OK C status + owned YetiError into a Go *Error,
// freeing the C error.
func statusErr(status C.YetiStatus, cerr *C.YetiError) error {
	if status == C.Ok {
		return nil
	}
	e := &Error{Kind: Kind(int(status))}
	if cerr != nil {
		e.Message = goStringOrEmpty(cerr.message)
		e.Status = int(cerr.http_status)
		e.ProblemType = goStringOrEmpty(cerr.problem_type)
		e.Title = goStringOrEmpty(cerr.title)
		e.Detail = goStringOrEmpty(cerr.detail)
		e.Body = goStringOrEmpty(cerr.body)
		C.yeti_error_free(cerr)
	}
	if e.Message == "" {
		e.Message = fmt.Sprintf("yeti sdk error (status %d)", int(status))
	}
	return e
}

// cString allocates a C string the caller must free.
func cString(s string) *C.char { return C.CString(s) }

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
