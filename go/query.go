package yetisdk

/*
#include <stdlib.h>
#include "yeti_sdk_go.h"
*/
import "C"

import "unsafe"

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
