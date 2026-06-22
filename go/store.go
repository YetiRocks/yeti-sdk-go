package yetisdk

/*
#include <stdlib.h>
#include "yeti_sdk_go.h"
*/
import "C"

import "unsafe"

// Entry is a (key, value) pair returned by the batch ops, ScanPrefix, and
// cursor pages.
type Entry struct {
	Key   string
	Value []byte
}

// Store is the typed persistence facade over the canonical table-aware
// persistence/store contract.
type Store struct{ client C.uint64_t }

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
