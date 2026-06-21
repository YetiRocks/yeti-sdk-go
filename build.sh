#!/usr/bin/env bash
# Build the cgo static archive + C header and stage them next to the Go
# package (go/). After this runs, `go build`/`go test` in go/ links the
# binding. Prints the staged archive path on the last line.
#
#   ./build.sh [--release]
set -euo pipefail
cd "$(dirname "$0")"

PROFILE_DIR=debug
CARGO_FLAGS=()
if [[ "${1:-}" == "--release" ]]; then
  PROFILE_DIR=release
  CARGO_FLAGS+=(--release)
fi

# 1. The staticlib (cgo links a .a, not a .dylib/.so).
cargo build --lib ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"} >&2

# 2. The C header from the #[no_mangle] surface.
cargo run --quiet --bin yeti-sdk-go-header -- "$(pwd)/go/yeti_sdk_go.h" >&2

# 3. Stage the archive next to the package so cgo's -L${SRCDIR} finds it.
ARCHIVE="target/${PROFILE_DIR}/libyeti_sdk_go.a"
cp "$ARCHIVE" "go/libyeti_sdk_go.a"

echo "$(pwd)/go/libyeti_sdk_go.a"
