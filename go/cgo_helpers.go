package yetisdk

/*
#include <stdlib.h>
#include "yeti_sdk_go.h"
*/
import "C"

import "unsafe"

// AbiVersion reports the C-ABI version of the linked native binding.
func AbiVersion() uint32 { return uint32(C.yeti_abi_version()) }

// cString allocates a C string the caller must free.
func cString(s string) *C.char { return C.CString(s) }

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
