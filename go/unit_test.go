// Unit tests for the parts of the binding observable without a live peer or a
// cgo type in the test signature: the linked-ABI probe, the wire-constant
// encodings, and the Error type. These run under a bare `go test ./...` (no
// server, no YETI_RPC_ADDR).
//
// A Go _test.go in the package under test cannot itself `import "C"`, so the
// helpers whose signatures name C types (statusErr, bytesPtr, goBytes,
// goEntries, cStringArray) are proved on two sides instead: the Rust FFI tests
// cover buffer construction/ownership, and the live round-trip in
// roundtrip_test.go exercises their consumption end to end.
package yetisdk

import (
	"errors"
	"testing"
)

func TestAbiVersion(t *testing.T) {
	// Calls into the linked native binding (AbiVersion's signature is C-free):
	// a non-1 value means the package and the staged archive disagree.
	if v := AbiVersion(); v != 1 {
		t.Fatalf("AbiVersion() = %d, want 1", v)
	}
}

func TestSeverityEncoding(t *testing.T) {
	// The wire encoding is load-bearing — the native side switches on these
	// exact integers, so pin them.
	if SeverityTrace != 0 || SeverityDebug != 1 || SeverityInfo != 2 ||
		SeverityWarn != 3 || SeverityError != 4 {
		t.Fatalf("severity encoding drifted: %d %d %d %d %d",
			SeverityTrace, SeverityDebug, SeverityInfo, SeverityWarn, SeverityError)
	}
}

func TestPubsubOpEncoding(t *testing.T) {
	if PubsubCreate != 0 || PubsubUpdate != 1 || PubsubDelete != 2 {
		t.Fatalf("pubsub-op encoding drifted: %d %d %d",
			PubsubCreate, PubsubUpdate, PubsubDelete)
	}
}

func TestKindEncoding(t *testing.T) {
	// Kind mirrors the negative C status codes the native side returns.
	if KindConnection != -1 || KindRemote != -2 || KindCodec != -3 || KindBadHandle != -4 {
		t.Fatalf("kind encoding drifted: %d %d %d %d",
			KindConnection, KindRemote, KindCodec, KindBadHandle)
	}
}

func TestErrorImplementsErrorAndIsMatchable(t *testing.T) {
	var err error = &Error{Kind: KindRemote, Message: "nope"}
	if err.Error() != "nope" {
		t.Fatalf("Error() = %q, want %q", err.Error(), "nope")
	}
	// errors.As must recover the typed value so callers can branch on Kind.
	var se *Error
	if !errors.As(err, &se) {
		t.Fatalf("errors.As did not match *Error: %T", err)
	}
	if se.Kind != KindRemote {
		t.Fatalf("recovered Kind = %d, want %d", se.Kind, KindRemote)
	}
}
