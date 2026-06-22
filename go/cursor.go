package yetisdk

/*
#include "yeti_sdk_go.h"
*/
import "C"

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
