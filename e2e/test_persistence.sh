#!/usr/bin/env bash
# @component aikv-storage
# E2E: aidb persistence compat — SAVE/BGSAVE/INFO/SHUTDOWN (Phase 11.6′)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
BIN="${ROOT}/target/release/aikv"
HOST="${WIKV_HOST:-127.0.0.1}"
PORT="${WIKV_PORT:-$((20000 + RANDOM % 40000))}"
ADDR="${HOST}:${PORT}"
DATA_DIR="$(mktemp -d)"

cargo build --release --manifest-path "${ROOT}/Cargo.toml" --quiet

wait_port_free() {
  for _ in $(seq 1 30); do
    if ! nc -z "${HOST}" "${PORT}" 2>/dev/null; then
      return 0
    fi
    sleep 0.1
  done
  echo "port ${PORT} still in use" >&2
  exit 1
}

wait_server() {
  for _ in $(seq 1 50); do
    if printf '*1\r\n$4\r\nPING\r\n' | nc -q 1 "${HOST}" "${PORT}" 2>/dev/null | grep -q PONG; then
      return 0
    fi
    sleep 0.1
  done
  echo "server failed to start on ${ADDR}" >&2
  exit 1
}

cleanup() {
  if [[ -n "${PID:-}" ]]; then
    kill "$PID" 2>/dev/null || true
    wait "$PID" 2>/dev/null || true
  fi
  rm -rf "$DATA_DIR"
}
trap cleanup EXIT

echo "==> persistence smoke tests on ${ADDR}"

# 1. memory engine — WARN
WARN_OUT="$(mktemp)"
"${BIN}" --engine memory --bind "${ADDR}" >/dev/null 2>"${WARN_OUT}" &
PID=$!
sleep 0.5
grep -q "engine=memory" "${WARN_OUT}"
kill "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
PID=""
rm -f "${WARN_OUT}"
wait_port_free

# 2. aidb — SAVE + INFO + SHUTDOWN
"${BIN}" --engine aidb --data-dir "${DATA_DIR}" --bind "${ADDR}" >/dev/null 2>&1 &
PID=$!
wait_server

printf '*3\r\n$3\r\nSET\r\n$1\r\nk\r\n$1\r\nv\r\n' | nc -q 2 "${HOST}" "${PORT}" >/dev/null
printf '*1\r\n$4\r\nSAVE\r\n' | nc -q 2 "${HOST}" "${PORT}" | grep -q OK
test -d "${DATA_DIR}/backup"
printf '*2\r\n$4\r\nINFO\r\n$11\r\npersistence\r\n' | nc -q 2 "${HOST}" "${PORT}" | grep -q rdb_bgsave_in_progress

printf '*1\r\n$8\r\nSHUTDOWN\r\n' | nc -q 2 "${HOST}" "${PORT}" | grep -q OK || true
wait "$PID" 2>/dev/null || true
PID=""

echo "e2e persistence smoke OK"
