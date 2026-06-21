#!/usr/bin/env bash
# Build the cgo binding, bring up a canonical yeti-host-rpc-server on a
# loopback port, run the Go round-trip test against it, and tear down. This is
# the Go counterpart to yeti-sdk-node/test/roundtrip.mjs.
#
#   ./run-roundtrip.sh [port]
set -euo pipefail
cd "$(dirname "$0")"

PORT="${1:-7831}"
GO="${GO:-/opt/homebrew/bin/go}"
REPO_ROOT="$(cd ../.. && pwd)"

# 1. Stage the archive + header for cgo.
./build.sh >/dev/null

# 2. Build + launch the canonical server (NOT legacy) from this worktree.
cargo build --manifest-path "$REPO_ROOT/Cargo.toml" -p yeti-host-rpc --bin yeti-host-rpc-server >&2
SERVER="$REPO_ROOT/target/debug/yeti-host-rpc-server"
"$SERVER" "127.0.0.1:$PORT" >/tmp/yeti-go-roundtrip-server.log 2>&1 &
SERVER_PID=$!
trap 'kill "$SERVER_PID" 2>/dev/null || true' EXIT
sleep 1

# 3. Run the round-trip.
cd go
CGO_ENABLED=1 YETI_RPC_ADDR="127.0.0.1:$PORT" "$GO" test -v ./...
