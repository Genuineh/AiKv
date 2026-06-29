#!/usr/bin/env bash
# Cluster E2E: 3-node formation — bootstrap, CLUSTER MEET, NODES, INFO, MYID
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/utils.sh"

require_redis_cli
build_release_cluster

echo "=== Cluster Formation Test ==="

DATA1="$(mktemp -d)"; DATA2="$(mktemp -d)"; DATA3="$(mktemp -d)"
declare -a _CLUSTER_PIDS _CLUSTER_HOSTS _CLUSTER_PORTS _CLUSTER_RPC

# Launch 3-node cluster
start_cluster_node 0 1 "${DATA1}"
N1_HOST="${_CLUSTER_HOSTS[0]}"; N1_PORT="${_CLUSTER_PORTS[0]}"; N1_RPC="${_CLUSTER_RPC[0]}"

start_cluster_node 1 2 "${DATA2}" "${N1_RPC}"
N2_HOST="${_CLUSTER_HOSTS[1]}"; N2_PORT="${_CLUSTER_PORTS[1]}"; N2_RPC="${_CLUSTER_RPC[1]}"

start_cluster_node 2 3 "${DATA3}" "${N1_RPC}"
N3_HOST="${_CLUSTER_HOSTS[2]}"; N3_PORT="${_CLUSTER_PORTS[2]}"; N3_RPC="${_CLUSTER_RPC[2]}"

register_cluster_cleanup "${DATA1}" "${DATA2}" "${DATA3}"

# ── CLUSTER MEET ──
echo "--- CLUSTER MEET node 2 ---"
rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER MEET "${N2_HOST}" "${N2_PORT}" "${N2_RPC##*:}" | grep -q OK
echo "OK"

echo "--- CLUSTER MEET node 3 ---"
rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER MEET "${N3_HOST}" "${N3_PORT}" "${N3_RPC##*:}" | grep -q OK
echo "OK"
sleep 2

# ── CLUSTER NODES: all 3 visible ──
echo "--- CLUSTER NODES ---"
NODES="$(rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER NODES)"
echo "${NODES}"
NODE_COUNT=$(echo "${NODES}" | grep -c . || true)
if [[ "${NODE_COUNT}" -lt 3 ]]; then
  echo "FAIL: expected >=3 nodes, got ${NODE_COUNT}" >&2; exit 1
fi
echo "OK: ${NODE_COUNT} nodes"

# All nodes agree
for host_port in "${N2_HOST}:${N2_PORT}" "${N3_HOST}:${N3_PORT}"; do
  h="${host_port%:*}"; p="${host_port##*:}"
  c=$(rc_node "$h" "$p" CLUSTER NODES | grep -c . || true)
  if [[ "${c}" -lt 3 ]]; then echo "FAIL: node ${host_port} sees only ${c} nodes" >&2; exit 1; fi
done
echo "OK: all nodes agree on membership"

# ── CLUSTER INFO ──
INFO="$(rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER INFO)"
echo "--- CLUSTER INFO ---"; echo "${INFO}"
echo "${INFO}" | grep -q "cluster_state:ok" || { echo "FAIL: cluster not ok" >&2; exit 1; }
echo "OK: cluster_state:ok"

# ── CLUSTER MYID ──
echo "--- CLUSTER MYID ---"
MYID1=$(rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER MYID | tr -d '\r\n')
MYID2=$(rc_node "${N2_HOST}" "${N2_PORT}" CLUSTER MYID | tr -d '\r\n')
MYID3=$(rc_node "${N3_HOST}" "${N3_PORT}" CLUSTER MYID | tr -d '\r\n')
for id in "${MYID1}" "${MYID2}" "${MYID3}"; do
  echo "${id}" | grep -qE '^[0-9a-fA-F]+$' || { echo "FAIL: invalid MYID" >&2; exit 1; }
done
if [[ "${MYID1}" == "${MYID2}" || "${MYID1}" == "${MYID3}" || "${MYID2}" == "${MYID3}" ]]; then
  echo "FAIL: duplicate MYID" >&2; exit 1
fi
echo "OK: all MYIDs unique hex"

echo ""
echo "=== test_cluster_formation.sh: PASS ==="
