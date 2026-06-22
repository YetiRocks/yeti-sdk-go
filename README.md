<p align="center">
  <img src="https://cdn.prod.website-files.com/68e09cef90d613c94c3671c0/697e805a9246c7e090054706_logo_horizontal_grey.png" alt="Yeti" width="200" />
</p>

---

# yeti-sdk-go

[![Yeti](https://img.shields.io/badge/Yeti-SDK-blue)](https://yetirocks.com)
[![Go](https://img.shields.io/badge/go-1.22+-00ADD8.svg)](https://go.dev/)
[![Rust](https://img.shields.io/badge/rust-1.85+-orange.svg)](https://www.rust-lang.org/)
[![cgo](https://img.shields.io/badge/cgo-required-lightgrey.svg)](https://pkg.go.dev/cmd/cgo)
[![License](https://img.shields.io/badge/license-Apache--2.0-green.svg)](LICENSE)

> The Go binding for the [Yeti](https://yetirocks.com) SDK: dial a Yeti
> host-rpc node over TLS and drive its store, query, pub/sub, rate-limit, and
> log capabilities from idiomatic Go. It is a thin wrapper over the same
> `yeti-sdk` Rust client the Node (napi) and Python/Swift/Kotlin (uniffi)
> bindings wrap — one source, no drift — exposed to Go across a C ABI via cgo.

This repository is two halves that build into one Go package:

- a **Rust `staticlib`** (`src/`) that puts a blocking C ABI over the async
  `yeti_sdk::remote` client, and
- a **Go package** (`go/`) that links that archive through cgo and presents a
  `Client` with typed capability facades.

`build.sh` compiles the archive, generates the C header, and stages both into
`go/` so a `go build` links them. The Go API is the surface you program against;
the Rust layer adds no behavior of its own.

---

## Install

The package links a native archive, so a consuming build needs **`CGO_ENABLED=1`**
plus a C toolchain, and the archive must be staged first (see
[Build](#build)). There is no published module tag yet (held under YTC-516), so
consume it with a local `replace` against the `go/` subdirectory:

```go
// go.mod
require github.com/yetirocks/yeti-sdk-go v0.0.0
replace github.com/yetirocks/yeti-sdk-go => /path/to/yeti-sdk-go/go
```

Building the archive from source requires Rust 1.85+ (for the `yeti-sdk`
dependency) and Go 1.22+.

---

## Build

```sh
./build.sh            # debug archive + header, staged into go/
./build.sh --release  # release archive (thin LTO)
```

This runs three steps: `cargo build --lib` produces `libyeti_sdk_go.a`,
the `yeti-sdk-go-header` binary emits `yeti_sdk_go.h` from the `#[no_mangle]`
surface via cbindgen, and both are copied next to the Go package so cgo's
`-L${SRCDIR}` / `-I${SRCDIR}` find them. The staged `.a`/`.h` are build outputs
(gitignored); regenerate them whenever the Rust surface changes.

---

## Quickstart

```go
package main

import (
	"fmt"
	"log"

	yetisdk "github.com/yetirocks/yeti-sdk-go"
)

func main() {
	// Dial a Yeti host-rpc node over TLS. serverName is verified against the
	// peer's leaf certificate. Connect trusts the public WebPKI roots; use
	// ConnectPinned(addr, serverName, caPEM) for a private CA (e.g. mkcert).
	client, err := yetisdk.Connect("127.0.0.1:7799", "localhost")
	if err != nil {
		log.Fatal(err)
	}
	defer client.Close()

	store := client.Store()

	// Put / get a value under (table, key). Values are opaque bytes.
	if err := store.Put("notes", "greeting", []byte("hello")); err != nil {
		log.Fatal(err)
	}
	value, found, err := store.Get("notes", "greeting")
	if err != nil {
		log.Fatal(err)
	}
	fmt.Printf("found=%v value=%q\n", found, value) // found=true value="hello"

	// Page a prefix scan over a server-held cursor.
	cursor, err := store.OpenScan("notes", "")
	if err != nil {
		log.Fatal(err)
	}
	defer cursor.Close()
	for {
		page, err := cursor.Next(100)
		if err != nil {
			log.Fatal(err)
		}
		if page == nil {
			break // exhausted
		}
		for _, e := range page {
			fmt.Printf("%s = %q\n", e.Key, e.Value)
		}
	}

	// Ship a structured log record to the node's tracing.
	_ = client.Log().Emit(yetisdk.SeverityInfo, "quickstart", "done", []yetisdk.Field{
		{Key: "lang", Value: "go"},
	})
}
```

---

## API overview

A `*Client` is one TLS connection. Its facade accessors share that connection;
each returns a thin typed handle.

| Accessor | Facade | What it does |
|---|---|---|
| `client.Store()` | `*Store` | Table-aware persistence: single-key, batch, scan/count, cursor, query. |
| `client.Publisher()` | `*Publisher` | Fire a change out to a table's subscribers (fire-and-forget). |
| `client.Subscriber()` | `*Subscriber` | Open a table's update stream and drain `*Event`s. |
| `client.RateLimit()` | `*RateLimit` | Consume a token from a `(bucket, key)` slot; returns a `RateDecision`. |
| `client.Log()` | `*Log` | Emit a structured log record (fire-and-acknowledge). |

**Store.** `Put` · `Get` · `PutIfAbsent` · `Delete` · `Exists` · `Scan` ·
`ScanPrefix` · `CountPrefix` · `GetMany` · `PutMany` · `DeleteMany` ·
`OpenScan` (→ `*Cursor`) · `Query` (→ `*Query`).

**Cursor.** `Next(max)` pulls a page of `[]Entry`; it returns `nil` once
exhausted. `Close` releases the server-side resource early.

**Query** (FIQL filter). `SortBy` · `Limit` · `Offset` refine the builder;
`Execute` (→ `[][]byte`) and `Count` materialize it; `Each` (→ `*RecordStream`)
streams it. `Close` releases the peer-side resource.

**Errors.** Every fallible call returns an `error` that is an `*Error` carrying
a `Kind` (`KindConnection`, `KindRemote`, `KindCodec`, `KindBadHandle`) so
callers can branch with `errors.As`.

`yetisdk.AbiVersion()` reports the C-ABI version of the linked archive — a
mismatch with the package means the staged `.a` is stale and `build.sh` must
re-run.

---

## Testing

```sh
./build.sh                      # stage the archive + header first
cd go && CGO_ENABLED=1 go test ./...
```

The suite has two tiers:

- **Unit tests** (`unit_test.go`) run with no peer — they pin the wire-constant
  encodings, the `Error` type, and the linked-ABI probe. The Rust crate carries
  its own `#[cfg(test)]` tests for the FFI buffer construction, ownership, and
  handle-registry behavior; run them with `cargo test`.
- **The live round-trip** (`roundtrip_test.go`) drives the full capability
  surface against a real `yeti-host-rpc-server`. It reads the address from
  `YETI_RPC_ADDR` (and `YETI_RPC_SERVER_NAME` for the TLS name) and **skips**
  when no server is reachable, so a bare `go test ./...` stays green offline.

`run-roundtrip.sh [port]` is the one-shot integration harness: it stages the
archive, builds and launches the canonical server on a loopback port, points the
test at it, runs the round-trip, and tears the server down.

Gate changes with the standard checks:

```sh
# Rust
cargo build && cargo test && cargo fmt --check && cargo clippy -- -D warnings
# Go (archive staged)
cd go && go build ./... && go test ./... && gofmt -l . && go vet ./...
```

---

## License

Apache-2.0.
