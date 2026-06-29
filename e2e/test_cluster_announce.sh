#!/usr/bin/env bash
# E2E: cluster announce mode — CLUSTER SLOTS empty host + cross-port SET/GET
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/utils.sh"

require_redis_cli
build_release_cluster

export AIKV_CLUSTER_ANNOUNCE_MODE="${AIKV_CLUSTER_ANNOUNCE_MODE:-unknown}"

echo "=== Cluster Announce Test (mode=${AIKV_CLUSTER_ANNOUNCE_MODE}) ==="

DATA1="$(mktemp -d)"
DATA2="$(mktemp -d)"
declare -a _CLUSTER_PIDS _CLUSTER_HOSTS _CLUSTER_PORTS _CLUSTER_RPC

start_cluster_node 0 1 "${DATA1}"
start_cluster_node 1 2 "${DATA2}"
N1_HOST="${_CLUSTER_HOSTS[0]}"; N1_PORT="${_CLUSTER_PORTS[0]}"
N2_HOST="${_CLUSTER_HOSTS[1]}"; N2_PORT="${_CLUSTER_PORTS[1]}"
N2_RPC="${_CLUSTER_RPC[1]}"
register_cluster_cleanup "${DATA1}" "${DATA2}"
sleep 1

echo "--- CLUSTER MEET ---"
rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER MEET "${N2_HOST}" "${N2_PORT}" "${N2_RPC##*:}" | grep -q OK

echo "--- ADDSLOTS split ---"
rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER ADDSLOTS $(seq 0 8191) >/dev/null
rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER ADDSLOTS NODE 2 $(seq 8192 16383) >/dev/null
sleep 2

echo "--- CLUSTER SLOTS host field ---"
SLOTS="$(rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER SLOTS)"
echo "${SLOTS}" | head -5
if [[ "${AIKV_CLUSTER_ANNOUNCE_MODE}" == "unknown" ]]; then
  if echo "${SLOTS}" | grep -qE '"[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+"'; then
    echo "FAIL: unknown mode should not expose IP in CLUSTER SLOTS" >&2
    exit 1
  fi
  echo "OK: no fixed IP in SLOTS (unknown mode)"
else
  echo "OK: fixed mode (skipped IP check)"
fi

echo "--- cross-shard SET/GET via redis-cli -c ---"
KEY="aikv:announce:e2e"
rc_cluster "${N1_HOST}" "${N1_PORT}" SET "${KEY}" "ok" | grep -qi ok
VAL="$(rc_cluster "${N1_HOST}" "${N1_PORT}" GET "${KEY}" | tr -d '\r\n')"
if [[ "${VAL}" != "ok" ]]; then
  echo "FAIL: GET want ok, got ${VAL:-none}" >&2
  exit 1
fi
echo "OK: cross-shard SET/GET"

echo ""
echo "=== test_cluster_announce.sh: PASS ==="
