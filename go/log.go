package yetisdk

/*
#include <stdlib.h>
#include "yeti_sdk_go.h"
*/
import "C"

import "unsafe"

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
