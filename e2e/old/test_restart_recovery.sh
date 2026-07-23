#!/usr/bin/env bash
# E2E: aidb BGSAVE → kill -9 (simulate crash) → restart → data intact
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
BIN="${ROOT}/target/release/aikv"
HOST="${AIKV_HOST:-127.0.0.1}"
PORT="${AIKV_PORT:-$((20000 + RANDOM % 40000))}"
ADDR="${HOST}:${PORT}"
DATA_DIR="$(mktemp -d)"

cargo build --release --manifest-path "${ROOT}/Cargo.toml" --quiet

require_redis_cli() {
  if ! command -v redis-cli >/dev/null 2>&1; then
    echo "redis-cli is required for e2e tests" >&2
    exit 1
  fi
}
require_redis_cli

cli() { redis-cli -h "${HOST}" -p "${PORT}" "$@"; }

start_server() {
  "${BIN}" --engine aidb --data-dir "${DATA_DIR}" --bind "${ADDR}" >/dev/null 2>&1 &
  SERVER_PID=$!
  for _ in $(seq 1 30); do
    if cli PING 2>/dev/null | grep -q PONG; then
      return 0
    fi
    sleep 0.1
  done
  echo "ERROR: server did not start" >&2
  exit 1
}

cleanup() {
  kill "${SERVER_PID:-}" 2>/dev/null || true
  wait "${SERVER_PID:-}" 2>/dev/null || true
  rm -rf "$DATA_DIR"
}
trap cleanup EXIT

echo "=== [1/4] Start server ==="
start_server

echo "=== [2/4] Write test data ==="
cli SET string_key "hello_world"
cli SET counter 42
cli HSET myhash field1 hello field2 world
cli LPUSH mylist c b a

echo "=== [3/4] BGSAVE (checkpoint) ==="
cli BGSAVE
for _ in $(seq 1 50); do
  IN_PROGRESS=$(cli INFO persistence | grep rdb_bgsave_in_progress | tr -d '[:space:]' | cut -d: -f2)
  if [[ "$IN_PROGRESS" == "0" ]]; then
    break
  fi
  sleep 0.1
done
[[ "$IN_PROGRESS" == "0" ]] || { echo "ERROR: BGSAVE did not complete" >&2; exit 1; }

echo "=== Crash simulation (kill -9) ==="
kill -9 "$SERVER_PID"
sleep 0.5

echo "=== [4/4] Restart and verify data ==="
start_server

GOT_STRING=$(cli GET string_key)
[[ "$GOT_STRING" == "hello_world" ]] || { echo "FAIL: string_key=$GOT_STRING" >&2; exit 1; }

GOT_COUNTER=$(cli GET counter)
[[ "$GOT_COUNTER" == "42" ]] || { echo "FAIL: counter=$GOT_COUNTER" >&2; exit 1; }

GOT_HFIELD=$(cli HGET myhash field1)
[[ "$GOT_HFIELD" == "hello" ]] || { echo "FAIL: myhash.field1=$GOT_HFIELD" >&2; exit 1; }

GOT_LIST=$(cli LRANGE mylist 0 -1)
echo "$GOT_LIST" | grep -q "a" || { echo "FAIL: mylist missing 'a': $GOT_LIST" >&2; exit 1; }

echo "=== All checks passed ==="
echo "PASS: test_restart_recovery"
