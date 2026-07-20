#!/usr/bin/env bash
# Cluster E2E: aidb engine persistence — data survives restart with cluster mode
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/utils.sh"

require_redis_cli
build_release_cluster

echo "=== AiDb Cluster Persistence Test ==="
echo ""

DATA_DIR="$(mktemp -d)"
BIN="${ROOT}/target/release/aikv"
CLIENT_PORT=$(_cluster_client_port 0)
RPC_PORT=$(_cluster_rpc_port 0)
HOST="127.0.0.1"

echo "Starting node with --engine aidb..."
"${BIN}" \
  --bind "${HOST}:${CLIENT_PORT}" \
  --engine aidb \
  --data-dir "${DATA_DIR}" \
  --cluster-node-id 1 \
  --cluster-rpc-addr "${HOST}:${RPC_PORT}" \
  >"${DATA_DIR}/server.log" 2>&1 &
SERVER_PID=$!

# Register cleanup
trap 'kill "${SERVER_PID}" 2>/dev/null || true; rm -rf "${DATA_DIR}"; wait 2>/dev/null || true' EXIT

wait_node "${HOST}" "${CLIENT_PORT}" || { echo "FAIL: node didn't start" >&2; exit 1; }
echo "Node started: client=${CLIENT_PORT}"

rc_node "${HOST}" "${CLIENT_PORT}" CLUSTER ADDSLOTS $(seq 0 16383) >/dev/null
sleep 2

# ── Bulk write ──
echo ""
echo "--- Writing data ---"
for i in $(seq 0 49); do
  rc_node "${HOST}" "${CLIENT_PORT}" SET "pkey:${i}" "pval-${i}" >/dev/null
done

DBS1=$(rc_node "${HOST}" "${CLIENT_PORT}" DBSIZE | tr -d '\r\n')
echo "Written 50 keys. DBSIZE=${DBS1}"

# ── HSET data ──
rc_node "${HOST}" "${CLIENT_PORT}" HSET "phash:1" field1 val1 field2 val2 >/dev/null
HLEN=$(rc_node "${HOST}" "${CLIENT_PORT}" HLEN "phash:1" | tr -d '\r\n')
echo "HSET data: HLEN=${HLEN}"

# ── Read-back ──
echo ""
echo "--- Read-back before restart ---"
errors=0
for i in $(seq 0 49); do
  val=$(rc_node "${HOST}" "${CLIENT_PORT}" GET "pkey:${i}" | tr -d '\r\n')
  if [[ "${val}" != "pval-${i}" ]]; then
    echo "MISMATCH: pkey:${i} expected pval-${i} got ${val}"
    errors=$((errors + 1))
  fi
done
if [[ "${errors}" -gt 0 ]]; then
  echo "FAIL: ${errors} mismatches before restart" >&2; exit 1
fi
echo "PASS: 50/50 keys verified"

# ── Restart ──
echo ""
echo "--- Restarting node ---"
kill "${SERVER_PID}" 2>/dev/null || true
wait "${SERVER_PID}" 2>/dev/null || true
sleep 1

# Restart with SAME data directory (no --engine memory, uses persisted aidb)
"${BIN}" \
  --bind "${HOST}:${CLIENT_PORT}" \
  --engine aidb \
  --data-dir "${DATA_DIR}" \
  --cluster-node-id 1 \
  --cluster-rpc-addr "${HOST}:${RPC_PORT}" \
  >"${DATA_DIR}/server.log" 2>&1 &
SERVER_PID=$!

wait_node "${HOST}" "${CLIENT_PORT}" || { echo "FAIL: node didn't restart" >&2; exit 1; }
echo "Node restarted"

# Wait for LifecycleManager tick to refresh the router's slot table
echo "--- Waiting for router refresh ---"
for _ in $(seq 1 10); do
  RESULT=$(rc_node "${HOST}" "${CLIENT_PORT}" GET "pkey:0" 2>&1 | tr -d '\r\n' || true)
  if echo "${RESULT}" | grep -Eq "^(pval-0|MOVED)"; then
    echo "Router ready"
    break
  fi
  echo "  Router not ready: ${RESULT}"
  sleep 1
done

# ── Verify data survives ──
echo ""
echo "--- Read-back after restart ---"
DBS2=$(rc_node "${HOST}" "${CLIENT_PORT}" DBSIZE | tr -d '\r\n')
echo "DBSIZE after restart: ${DBS2}"

if [[ "${DBS2}" -eq 0 ]]; then
  echo "FAIL: No data survived restart (DBSIZE=0)" >&2
  exit 1
fi

errors=0
for i in $(seq 0 49); do
  val=$(rc_node "${HOST}" "${CLIENT_PORT}" GET "pkey:${i}" | tr -d '\r\n')
  if [[ "${val}" != "pval-${i}" ]]; then
    echo "MISMATCH: pkey:${i} expected pval-${i} got ${val}"
    errors=$((errors + 1))
  fi
done

if [[ "${errors}" -eq 0 ]]; then
  echo "PASS: all 50 keys survived restart"
else
  echo "FAIL: ${errors} keys lost after restart" >&2
  exit 1
fi

# Verify hash survived
HLEN2=$(rc_node "${HOST}" "${CLIENT_PORT}" HLEN "phash:1" | tr -d '\r\n')
if [[ "${HLEN2}" == "2" ]]; then
  echo "PASS: Hash data survived restart (HLEN=${HLEN2})"
else
  echo "FAIL: Hash data lost after restart (HLEN=${HLEN2})" >&2
  exit 1
fi

# ── Cluster metadata persists ──
echo ""
echo "--- Cluster metadata after restart ---"
INFO=$(rc_node "${HOST}" "${CLIENT_PORT}" CLUSTER INFO 2>&1 | tr -d '\r\n')
if echo "${INFO}" | grep -q "cluster_state:ok"; then
  echo "PASS: cluster_state:ok (slots restored)"
else
  echo "WARN: cluster_state not ok after restart"
fi

echo ""
echo "=== test_cluster_aidb_persistence.sh: PASS ==="
