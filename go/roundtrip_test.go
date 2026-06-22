// End-to-end round-trip for the cgo Go binding. Drives the full capability
// surface against a live yeti-host-rpc-server (canonical contract), proving
// the Go binding shares the yeti-sdk logic with the napi/uniffi
// bindings.
//
// The server address comes from YETI_RPC_ADDR (default 127.0.0.1:7799). Run
// ../build.sh first so the archive + header are staged. The harness in
// run-roundtrip.sh spins up a server and sets the env.
package yetisdk

import (
	"bytes"
	"os"
	"sort"
	"testing"
)

func addr() string {
	if a := os.Getenv("YETI_RPC_ADDR"); a != "" {
		return a
	}
	return "127.0.0.1:7799"
}

// serverName is the TLS server name verified against the peer's leaf cert. The
// dial is TLS-only; the harness sets YETI_RPC_SERVER_NAME to match the cert the
// host-rpc server presents.
func serverName() string {
	if n := os.Getenv("YETI_RPC_SERVER_NAME"); n != "" {
		return n
	}
	return "localhost"
}

func TestRoundTrip(t *testing.T) {
	// This is the end-to-end integration test: it needs a live
	// yeti-host-rpc-server. run-roundtrip.sh stands one up and sets
	// YETI_RPC_ADDR. Without a reachable peer (a bare `go test ./...`), skip
	// rather than fail — the offline gate is the unit suite in unit_test.go.
	client, err := Connect(addr(), serverName())
	if err != nil {
		if os.Getenv("YETI_RPC_ADDR") == "" {
			t.Skipf("no YETI_RPC_ADDR set; skipping live round-trip (run ./run-roundtrip.sh): %v", err)
		}
		t.Fatalf("connect: %v", err)
	}
	defer client.Close()
	store := client.Store()

	// Self-clean: the server may be long-lived and shared.
	mustDeleteMany(t, store, "notes", []string{"greeting", "lang-js", "lang-py", "lang-rs"})
	mustDeleteMany(t, store, "kv", []string{"k1"})
	mustDeleteMany(t, store, "batch", []string{"a", "b"})
	mustDeleteMany(t, store, "pages", []string{"k0", "k1", "k2", "k3", "k4"})

	// put / get round-trip with binary value.
	if err := store.Put("notes", "greeting", []byte("hello")); err != nil {
		t.Fatalf("put: %v", err)
	}
	v, found, err := store.Get("notes", "greeting")
	if err != nil || !found || !bytes.Equal(v, []byte("hello")) {
		t.Fatalf("get greeting: v=%q found=%v err=%v", v, found, err)
	}

	// absent -> not found.
	if _, found, err := store.Get("notes", "absent"); err != nil || found {
		t.Fatalf("get absent: found=%v err=%v", found, err)
	}

	// exists.
	if ok, err := store.Exists("notes", "greeting"); err != nil || !ok {
		t.Fatalf("exists greeting: %v %v", ok, err)
	}
	if ok, err := store.Exists("notes", "absent"); err != nil || ok {
		t.Fatalf("exists absent: %v %v", ok, err)
	}

	// tables are first-class: same key, other table is absent.
	if _, found, err := store.Get("other", "greeting"); err != nil || found {
		t.Fatalf("get other/greeting: found=%v err=%v", found, err)
	}

	// scan keys by prefix.
	mustPut(t, store, "notes", "lang-js", []byte("1"))
	mustPut(t, store, "notes", "lang-py", []byte("3"))
	keys, err := store.Scan("notes", "lang-")
	if err != nil {
		t.Fatalf("scan: %v", err)
	}
	sort.Strings(keys)
	if len(keys) != 2 || keys[0] != "lang-js" || keys[1] != "lang-py" {
		t.Fatalf("scan keys = %v", keys)
	}

	// delete reports prior existence.
	if ok, err := store.Delete("notes", "greeting"); err != nil || !ok {
		t.Fatalf("delete greeting #1: %v %v", ok, err)
	}
	if ok, err := store.Delete("notes", "greeting"); err != nil || ok {
		t.Fatalf("delete greeting #2: %v %v", ok, err)
	}

	// put_if_absent inserts once, then refuses.
	if ins, err := store.PutIfAbsent("kv", "k1", []byte("v1")); err != nil || !ins {
		t.Fatalf("pia #1: %v %v", ins, err)
	}
	if ins, err := store.PutIfAbsent("kv", "k1", []byte("v2")); err != nil || ins {
		t.Fatalf("pia #2: %v %v", ins, err)
	}
	if v, found, err := store.Get("kv", "k1"); err != nil || !found || !bytes.Equal(v, []byte("v1")) {
		t.Fatalf("get kv/k1: v=%q found=%v err=%v", v, found, err)
	}

	// batch put / scanPrefix / countPrefix.
	if err := store.PutMany("batch", []Entry{
		{Key: "a", Value: []byte("A")},
		{Key: "b", Value: []byte("B")},
	}); err != nil {
		t.Fatalf("putMany: %v", err)
	}
	entries, err := store.ScanPrefix("batch", "")
	if err != nil {
		t.Fatalf("scanPrefix: %v", err)
	}
	sort.Slice(entries, func(i, j int) bool { return entries[i].Key < entries[j].Key })
	if len(entries) != 2 ||
		entries[0].Key != "a" || string(entries[0].Value) != "A" ||
		entries[1].Key != "b" || string(entries[1].Value) != "B" {
		t.Fatalf("scanPrefix entries = %+v", entries)
	}
	if n, err := store.CountPrefix("batch", ""); err != nil || n != 2 {
		t.Fatalf("countPrefix: %d %v", n, err)
	}

	// getMany: one slot per requested key, in order, nil where absent.
	vals, err := store.GetMany("batch", []string{"a", "absent", "b"})
	if err != nil {
		t.Fatalf("getMany: %v", err)
	}
	if len(vals) != 3 ||
		!bytes.Equal(vals[0], []byte("A")) || vals[1] != nil || !bytes.Equal(vals[2], []byte("B")) {
		t.Fatalf("getMany vals = %v", vals)
	}

	if n, err := store.DeleteMany("batch", []string{"a", "b", "absent"}); err != nil || n != 2 {
		t.Fatalf("deleteMany: %d %v", n, err)
	}

	// scan cursor: paged pull over a server-held resource.
	for i := 0; i < 5; i++ {
		mustPut(t, store, "pages", "k"+string(rune('0'+i)), []byte{byte(i)})
	}
	cursor, err := store.OpenScan("pages", "k")
	if err != nil {
		t.Fatalf("openScan: %v", err)
	}
	var pageSizes []int
	for {
		page, err := cursor.Next(2)
		if err != nil {
			t.Fatalf("cursor.Next: %v", err)
		}
		if page == nil {
			break
		}
		if len(page) > 2 {
			t.Fatalf("page too big: %d", len(page))
		}
		pageSizes = append(pageSizes, len(page))
	}
	if len(pageSizes) != 3 || pageSizes[0] != 2 || pageSizes[1] != 2 || pageSizes[2] != 1 {
		t.Fatalf("pageSizes = %v", pageSizes)
	}
	// exhausted cursor: Next stays nil, Close is a no-op.
	if page, err := cursor.Next(2); err != nil || page != nil {
		t.Fatalf("post-exhaustion Next: %v %v", page, err)
	}
	if err := cursor.Close(); err != nil {
		t.Fatalf("close exhausted: %v", err)
	}

	// early close releases the server-side resource.
	early, err := store.OpenScan("pages", "k")
	if err != nil {
		t.Fatalf("openScan early: %v", err)
	}
	first, err := early.Next(1)
	if err != nil || len(first) != 1 {
		t.Fatalf("early.Next: %v %v", first, err)
	}
	if err := early.Close(); err != nil {
		t.Fatalf("early.Close: %v", err)
	}

	// telemetry/logs: fire-and-acknowledge into the peer's tracing.
	log := client.Log()
	if err := log.Emit(SeverityInfo, "roundtrip", "hello from go", []Field{{Key: "lang", Value: "go"}}); err != nil {
		t.Fatalf("log.Emit info: %v", err)
	}
	if err := log.Emit(SeverityWarn, "roundtrip", "and a warning", nil); err != nil {
		t.Fatalf("log.Emit warn: %v", err)
	}

	// sanity: native binding ABI is the version the package expects.
	if AbiVersion() != 1 {
		t.Fatalf("abi version = %d", AbiVersion())
	}

	t.Log("OK: full capability surface round-trip via cgo Go binding")
}

func mustPut(t *testing.T, s *Store, table, key string, v []byte) {
	t.Helper()
	if err := s.Put(table, key, v); err != nil {
		t.Fatalf("put %s/%s: %v", table, key, err)
	}
}

func mustDeleteMany(t *testing.T, s *Store, table string, keys []string) {
	t.Helper()
	if _, err := s.DeleteMany(table, keys); err != nil {
		t.Fatalf("deleteMany %s: %v", table, err)
	}
}
