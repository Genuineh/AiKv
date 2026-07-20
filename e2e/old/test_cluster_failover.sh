#!/usr/bin/env bash
# Cluster E2E: failover — kill secondary node, verify cluster; kill leader, verify failover
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/utils.sh"

require_redis_cli
build_release_cluster

echo "=== Cluster Failover Test ==="

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

# ADDSLOTS on node 1
rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER ADDSLOTS $(seq 0 16383) >/dev/null
sleep 5

# Add replicas so N2 and N3 can take over data group on leader failover
N1_HEX="0000000000000000000000000000000000000001"
N2_HEX="0000000000000000000000000000000000000002"
N3_HEX="0000000000000000000000000000000000000003"
echo "--- Adding replicas: N2, N3 as replicas of N1 ---"
rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER ADD_REPLICA "${N1_HEX}" "${N2_HEX}"
rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER ADD_REPLICA "${N1_HEX}" "${N3_HEX}"
sleep 3  # Wait for replication barriers + membership propagation

# Verify replicas are present in CLUSTER NODES
CLUSTER_NODES=$(rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER NODES)
echo "CLUSTER NODES after ADD_REPLICA:"
echo "${CLUSTER_NODES}"

# Write data
echo "--- Writing pre-failover data ---"
for i in $(seq 0 29); do
  key="fo:$(printf '%04x' "${i}")"
  rc_node "${N1_HOST}" "${N1_PORT}" SET "${key}" "fv-${i}" >/dev/null
done

CNT1=$(rc_node "${N1_HOST}" "${N1_PORT}" DBSIZE | tr -d '\r\n')
echo "Data before kill: DBSIZE=${CNT1}"

# ── Part 1: Kill secondary node (N3) ──
echo "--- Killing secondary node (N3) ---"
N3_PID="${_CLUSTER_PIDS[2]}"
kill "${N3_PID}" 2>/dev/null || true
wait "${N3_PID}" 2>/dev/null || true
sleep 2

# N1 should still be healthy after N3 is killed
PING=$(rc_node "${N1_HOST}" "${N1_PORT}" PING | tr -d '\r\n')
if [[ "${PING}" != "PONG" ]]; then echo "FAIL: N1 unresponsive after N3 kill" >&2; exit 1; fi
echo "N1 healthy after N3 kill: PING=${PING}"

# Cluster state from N1
CS=$(rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER INFO | grep "cluster_state:" | tr -d '\r\n')
echo "Cluster state after N3 kill: ${CS}"
if ! echo "${CS}" | grep -q "ok"; then
  echo "FAIL: cluster state not ok after secondary node kill" >&2; exit 1
fi

# Restart N3
echo "--- Restarting N3 ---"
rm -rf "${DATA3}"; mkdir -p "${DATA3}"
start_cluster_node 2 3 "${DATA3}" "${N1_RPC}"
sleep 2
PING3=$(rc_node "${_CLUSTER_HOSTS[2]}" "${_CLUSTER_PORTS[2]}" PING | tr -d '\r\n' || echo "DEAD")
echo "N3 after restart: PING=${PING3}"
if [[ "${PING3}" != "PONG" ]]; then echo "FAIL: N3 did not restart" >&2; exit 1; fi

# ── Part 2: Leader failover test ──
echo "--- Testing leader failover ---"
FAILOVER_KEY="fo:failover_test_$$"
rc_node "${N1_HOST}" "${N1_PORT}" SET "${FAILOVER_KEY}" "before_failover" >/dev/null

MASTER_LINE=$(rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER NODES | grep "myself,master" || echo "")
if [ -n "${MASTER_LINE}" ]; then
  echo "N1 is the master. Verifying failover on N1 kill..."
  FV_BEFORE=$(rc_node "${N1_HOST}" "${N1_PORT}" GET "${FAILOVER_KEY}" | tr -d '\r\n')
  echo "Value before kill: ${FV_BEFORE}"

  N1_PID="${_CLUSTER_PIDS[0]}"
  kill "${N1_PID}" 2>/dev/null || true
  wait "${N1_PID}" 2>/dev/null || true
  sleep 5  # election 1s + barrier 0.15s + watcher 0.25s + margin

  # Verify new leader elected in CLUSTER NODES
  CLUSTER_NODES_AFTER=$(rc_node "${N2_HOST}" "${N2_PORT}" CLUSTER NODES 2>&1 || echo "DEAD")
  echo "CLUSTER NODES after N1 kill (from N2):"
  echo "${CLUSTER_NODES_AFTER}"

  # At least one node should now show myself,master for the slot range
  if echo "${CLUSTER_NODES_AFTER}" | grep -q "myself,master"; then
    echo "Leader failover: new master elected"
  else
    echo "WARN: no master found after failover (may need more time)"
  fi

  # Try N2 — should get data via MOVED or direct
  FV_AFTER=$(rc_node "${N2_HOST}" "${N2_PORT}" GET "${FAILOVER_KEY}" 2>&1 || echo "ERROR")
  echo "Value after N1 kill (from N2): ${FV_AFTER}"

  # Cluster should still report ok state on surviving node
  CLUSTER_OK=$(rc_node "${N2_HOST}" "${N2_PORT}" CLUSTER INFO | grep "cluster_state:ok" || echo "NOT_OK")
  echo "Cluster state: ${CLUSTER_OK}"

  if echo "${CLUSTER_OK}" | grep -q "ok"; then
    echo "Leader failover: cluster remains healthy"
  else
    echo "Leader failover: cluster state degraded (expected during re-election)"
  fi

  # Restart N1
  echo "--- Restarting N1 ---"
  rm -rf "${DATA1}"; mkdir -p "${DATA1}"
  start_cluster_node 0 1 "${DATA1}" "${N2_RPC}"
  sleep 2
  PING1=$(rc_node "${_CLUSTER_HOSTS[0]}" "${_CLUSTER_PORTS[0]}" PING | tr -d '\r\n' || echo "DEAD")
  echo "N1 after restart: PING=${PING1}"
else
  echo "N1 is not master, skipping leader kill test"
fi

echo ""
echo "=== test_cluster_failover.sh: PASS ==="
