package yetisdk

/*
#include <stdlib.h>
#include "yeti_sdk_go.h"
*/
import "C"

import "unsafe"

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
