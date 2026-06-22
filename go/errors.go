package yetisdk

/*
#include "yeti_sdk_go.h"
*/
import "C"

import "fmt"

// Severity mirrors the wire's integer log-level encoding.
type Severity uint8

const (
	SeverityTrace Severity = 0
	SeverityDebug Severity = 1
	SeverityInfo  Severity = 2
	SeverityWarn  Severity = 3
	SeverityError Severity = 4
)

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
