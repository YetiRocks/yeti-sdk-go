# yeti-sdk-go

The Go binding for the Yeti SDK — idiomatic Go over the shared
[`yeti-sdk`](../yeti-sdk) Rust core via **cgo** (ADR-025,
YTC-519). The same "one source, no drift" model as the napi (Node) and uniffi
(Python/Swift/Kotlin) bindings: this crate adds only a C ABI; all behavior
lives in `yeti-sdk`.

## Layout

- `src/lib.rs` — the `#[no_mangle] extern "C"` surface (a `staticlib`) that
  blocks the async client on a shared tokio runtime and hands results across
  the boundary via out-params + owned buffers.
- `src/bin/header.rs` — emits `yeti_sdk_go.h` from that surface via cbindgen.
- `go/` — the `yetisdk` Go package (`yetisdk.go`) that links the archive +
  header through cgo, plus the round-trip test.
- `build.sh` — builds the staticlib, generates the header, and stages both
  into `go/` so `go build`/`go test` link them.

## Surface

`yetisdk.Connect(addr) -> *Client`; `client.Store()` exposes the full canonical
`persistence/store` contract (`Put`/`Get`/`PutIfAbsent`/`Delete`/`Exists`/
`Scan`/`ScanPrefix`/`CountPrefix`/`GetMany`/`PutMany`/`DeleteMany`/`OpenScan`
+ the paged `Cursor`), and `client.Log()` exposes `telemetry/logs` `Emit`.

## Build & test

```sh
./build.sh                 # stage go/libyeti_sdk_go.a + go/yeti_sdk_go.h
./run-roundtrip.sh 7831    # build + server + Go round-trip, all in one
```

The staged `.a`/`.h` are build outputs (gitignored); regenerate with
`build.sh`. **No publish** (Go module tag/proxy held under YTC-516) — examples
consume the package with a local `replace`.

## Consuming from a Go module

```go.mod
require github.com/yetirocks/yeti-sdk-go v0.0.0
replace github.com/yetirocks/yeti-sdk-go => /path/to/sdks/yeti-sdk-go/go
```

Run `../yeti-sdk-go/build.sh` first so the archive/header are present, then
build with `CGO_ENABLED=1`. See
[`../example-sdk-projects/go/remote-native`](../example-sdk-projects/go/remote-native).
