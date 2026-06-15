#!/usr/bin/env bash
# 2-shard, 6-node (1 master + 2 replicas per shard) non-container cluster test.
# Cluster keeps running after this script exits. Stop manually.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/utils.sh"

require_redis_cli
build_release_cluster

BASE_PORT="${WIKV_CLUSTER_BASE_PORT:-7000}"
NODE_COUNT=6

echo "=== 6-Node Cluster Setup ==="
echo "Base port: ${BASE_PORT}  (nodes spaced 200 apart)"
echo ""

DATA_DIRS=()
for i in $(seq 0 $((NODE_COUNT - 1))); do
  DATA_DIRS[$i]="$(mktemp -d /tmp/wiqun-node-$((i + 1))-XXXXXX)"
done

PIDS=()
CLIENT_PORTS=()
RPC_PORTS=()

for i in $(seq 0 $((NODE_COUNT - 1))); do
  CLIENT_PORTS[$i]=$((BASE_PORT + i * 200))
  RPC_PORTS[$i]=$((BASE_PORT + i * 200 + 1))
done

# Topology:
#   Shard 1: Node 1 (master, port 7000), Node 2 (replica, 7200), Node 3 (replica, 7400)
#   Shard 2: Node 4 (master, port 7600), Node 5 (replica, 7800), Node 6 (replica, 8000)

# ── Start node 1 (bootstrap) ──
echo "--- Starting Node 1 (shard 1 master, port ${CLIENT_PORTS[0]}) ---"
rm -rf "${DATA_DIRS[0]}"
mkdir -p "${DATA_DIRS[0]}"
nohup "${BIN}" \
  --bind "127.0.0.1:${CLIENT_PORTS[0]}" \
  --engine memory \
  --data-dir "${DATA_DIRS[0]}" \
  --cluster-node-id 1 \
  --cluster-rpc-addr "127.0.0.1:${RPC_PORTS[0]}" \
  >"${DATA_DIRS[0]}/server.log" 2>&1 &
PIDS[0]=$!
wait_node "127.0.0.1" "${CLIENT_PORTS[0]}"
echo "  PID ${PIDS[0]}"

# ── Start nodes 2-6 (join via node 1) ──
for i in $(seq 1 $((NODE_COUNT - 1))); do
  nid=$((i + 1))
  echo "--- Starting Node ${nid} (port ${CLIENT_PORTS[$i]}) ---"
  rm -rf "${DATA_DIRS[$i]}"
  mkdir -p "${DATA_DIRS[$i]}"
  nohup "${BIN}" \
    --bind "127.0.0.1:${CLIENT_PORTS[$i]}" \
    --engine memory \
    --data-dir "${DATA_DIRS[$i]}" \
    --cluster-node-id "${nid}" \
    --cluster-rpc-addr "127.0.0.1:${RPC_PORTS[$i]}" \
    --cluster-peers "127.0.0.1:${RPC_PORTS[0]}" \
    >"${DATA_DIRS[$i]}/server.log" 2>&1 &
  PIDS[$i]=$!
  wait_node "127.0.0.1" "${CLIENT_PORTS[$i]}"
  echo "  PID ${PIDS[$i]}"
done
echo "=== All ${NODE_COUNT} nodes started ==="

# ── CLUSTER MEET ──
echo ""
echo "--- CLUSTER MEET ---"
for i in $(seq 1 $((NODE_COUNT - 1))); do
  redis-cli -h 127.0.0.1 -p "${CLIENT_PORTS[0]}" \
    CLUSTER MEET 127.0.0.1 "${CLIENT_PORTS[$i]}" "${RPC_PORTS[$i]}" >/dev/null
done

# Wait for convergence
for _ in $(seq 1 15); do
  nodes=$(redis-cli -h 127.0.0.1 -p "${CLIENT_PORTS[0]}" CLUSTER NODES 2>/dev/null || true)
  count=$(echo "${nodes}" | grep -c . || true)
  [[ "${count}" -ge ${NODE_COUNT} ]] && break
  sleep 1
done
echo "  ${count:-0}/${NODE_COUNT} nodes visible"

# ── Assign slots ──
echo ""
echo "--- Assigning slots ---"
echo "  Slots 0-8191 → Node 1"
redis-cli -h 127.0.0.1 -p "${CLIENT_PORTS[0]}" \
  CLUSTER ADDSLOTS $(seq 0 8191) >/dev/null
echo "  Slots 8192-16383 → Node 4"
redis-cli -h 127.0.0.1 -p "${CLIENT_PORTS[0]}" \
  CLUSTER ADDSLOTS NODE 4 $(seq 8192 16383) >/dev/null

# Wait for slot propagation
for _ in $(seq 1 15); do
  info=$(redis-cli -h 127.0.0.1 -p "${CLIENT_PORTS[0]}" CLUSTER INFO 2>/dev/null || true)
  assigned=$(echo "${info}" | grep "cluster_slots_assigned" | grep -oE '[0-9]+' || echo "0")
  [[ "${assigned}" -eq 16384 ]] && break
  sleep 1
done
echo "  ${assigned:-0}/16384 slots assigned"

# ── Health check ──
echo ""
echo "--- Health check ---"
for i in $(seq 0 $((NODE_COUNT - 1))); do
  if kill -0 "${PIDS[$i]}" 2>/dev/null; then
    resp=$(redis-cli -h 127.0.0.1 -p "${CLIENT_PORTS[$i]}" PING 2>/dev/null | tr -d '\r\n')
    echo "  Node $((i + 1)): PID ${PIDS[$i]} alive, PING=${resp}"
  else
    echo "  Node $((i + 1)): DEAD (check ${DATA_DIRS[$i]}/server.log)"
  fi
done

# ── Display cluster state ──
echo ""
echo "--- CLUSTER NODES ---"
redis-cli -h 127.0.0.1 -p "${CLIENT_PORTS[0]}" CLUSTER NODES

echo ""
echo "--- CLUSTER INFO ---"
redis-cli -h 127.0.0.1 -p "${CLIENT_PORTS[0]}" CLUSTER INFO

echo ""
echo "=========================================="
echo " 6-node cluster ready for manual testing"
echo "=========================================="
echo ""
printf "  %-6s %-22s %-20s %-10s %-15s\n" "Node" "Client" "RPC" "Data" "Role"
printf "  %-6s %-22s %-20s %-10s %-15s\n" "----" "------" "---" "----" "----"
printf "  %-6s %-22s %-20s %-10s %-15s\n" "1" "127.0.0.1:${CLIENT_PORTS[0]}" "127.0.0.1:${RPC_PORTS[0]}" "$((${RPC_PORTS[0]} + 10000))" "shard1-master"
printf "  %-6s %-22s %-20s %-10s %-15s\n" "2" "127.0.0.1:${CLIENT_PORTS[1]}" "127.0.0.1:${RPC_PORTS[1]}" "$((${RPC_PORTS[1]} + 10000))" "shard1-replica"
printf "  %-6s %-22s %-20s %-10s %-15s\n" "3" "127.0.0.1:${CLIENT_PORTS[2]}" "127.0.0.1:${RPC_PORTS[2]}" "$((${RPC_PORTS[2]} + 10000))" "shard1-replica"
printf "  %-6s %-22s %-20s %-10s %-15s\n" "4" "127.0.0.1:${CLIENT_PORTS[3]}" "127.0.0.1:${RPC_PORTS[3]}" "$((${RPC_PORTS[3]} + 10000))" "shard2-master"
printf "  %-6s %-22s %-20s %-10s %-15s\n" "5" "127.0.0.1:${CLIENT_PORTS[4]}" "127.0.0.1:${RPC_PORTS[4]}" "$((${RPC_PORTS[4]} + 10000))" "shard2-replica"
printf "  %-6s %-22s %-20s %-10s %-15s\n" "6" "127.0.0.1:${CLIENT_PORTS[5]}" "127.0.0.1:${RPC_PORTS[5]}" "$((${RPC_PORTS[5]} + 10000))" "shard2-replica"
echo ""
echo "Quick commands:"
echo "  redis-cli -p ${CLIENT_PORTS[0]} PING"
echo "  redis-cli -p ${CLIENT_PORTS[0]} CLUSTER NODES"
echo "  redis-cli -p ${CLIENT_PORTS[0]} SET mykey myvalue"
echo ""
echo "Log files:"
for i in $(seq 0 $((NODE_COUNT - 1))); do
  echo "  Node $((i + 1)): ${DATA_DIRS[$i]}/server.log"
done
echo ""
echo "To stop: kill ${PIDS[*]}"
echo "=========================================="
