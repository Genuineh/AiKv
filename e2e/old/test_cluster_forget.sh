#!/usr/bin/env bash
# Cluster E2E: CLUSTER FORGET — remove node, verify remaining cluster
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/utils.sh"

require_redis_cli
build_release_cluster

echo "=== Cluster Forget Test ==="

# Launch 3 nodes
DATA1="$(mktemp -d)"; DATA2="$(mktemp -d)"; DATA3="$(mktemp -d)"
declare -a _CLUSTER_PIDS _CLUSTER_HOSTS _CLUSTER_PORTS _CLUSTER_RPC

start_cluster_node 0 1 "${DATA1}"
N1_HOST="${_CLUSTER_HOSTS[0]}"; N1_PORT="${_CLUSTER_PORTS[0]}"; N1_RPC="${_CLUSTER_RPC[0]}"

start_cluster_node 1 2 "${DATA2}" "${N1_RPC}"
N2_HOST="${_CLUSTER_HOSTS[1]}"; N2_PORT="${_CLUSTER_PORTS[1]}"; N2_RPC="${_CLUSTER_RPC[1]}"

start_cluster_node 2 3 "${DATA3}" "${N1_RPC}"
N3_HOST="${_CLUSTER_HOSTS[2]}"; N3_PORT="${_CLUSTER_PORTS[2]}"; N3_RPC="${_CLUSTER_RPC[2]}"

register_cluster_cleanup "${DATA1}" "${DATA2}" "${DATA3}"

# MEET
rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER MEET "${N2_HOST}" "${N2_PORT}" "${N2_RPC##*:}" >/dev/null
rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER MEET "${N3_HOST}" "${N3_PORT}" "${N3_RPC##*:}" >/dev/null
sleep 2

# ADDSLOTS
rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER ADDSLOTS $(seq 0 16383) >/dev/null
sleep 2

# Write data
echo "--- Writing pre-forget data ---"
for i in $(seq 0 19); do
  key="fg:$(printf '%04x' "${i}")"
  rc_node "${N1_HOST}" "${N1_PORT}" SET "${key}" "fv-${i}" >/dev/null
done
DBS1=$(rc_node "${N1_HOST}" "${N1_PORT}" DBSIZE | tr -d '\r\n')
echo "Pre-forget DBSIZE: ${DBS1}"

# Get node 3's ID
NODES_BEFORE=$(rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER NODES)
echo "Pre-forget NODES:"
echo "${NODES_BEFORE}"
N3_ID=$(echo "${NODES_BEFORE}" | tail -1 | awk '{print $1}')
echo "Node 3 ID: ${N3_ID}"

# CLUSTER FORGET
echo "--- CLUSTER FORGET ${N3_ID} ---"
FORGET=$(rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER FORGET "${N3_ID}" 2>&1)
echo "FORGET: ${FORGET}"
sleep 1

# Verify node 1 still functional
PING=$(rc_node "${N1_HOST}" "${N1_PORT}" PING | tr -d '\r\n')
if [[ "${PING}" != "PONG" ]]; then echo "FAIL: node 1 dead after forget" >&2; exit 1; fi

# Data still accessible
DBS2=$(rc_node "${N1_HOST}" "${N1_PORT}" DBSIZE | tr -d '\r\n')
echo "Post-forget DBSIZE: ${DBS2}"

# Verify data integrity
echo "--- Data integrity ---"
errors=0
for i in $(seq 0 19); do
  key="fg:$(printf '%04x' "${i}")"
  val=$(rc_node "${N1_HOST}" "${N1_PORT}" GET "${key}" | tr -d '\r\n')
  if [[ "${val}" != "fv-${i}" ]]; then errors=$((errors+1)); fi
done
if [[ "${errors}" -gt 0 ]]; then
  echo "FAIL: ${errors} keys lost after forget" >&2; exit 1
fi
echo "OK: all 20 keys intact after forget"

echo ""
echo "=== test_cluster_forget.sh: PASS ==="
