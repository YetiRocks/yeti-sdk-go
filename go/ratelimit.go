package yetisdk

/*
#include <stdlib.h>
#include "yeti_sdk_go.h"
*/
import "C"

import "unsafe"

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
