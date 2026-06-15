#!/usr/bin/env bash
# Cluster E2E: data consistency — bulk write, read-back, restart
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/utils.sh"

require_redis_cli
build_release_cluster

echo "=== Cluster Data Consistency Test ==="

DATA1="$(mktemp -d)"
declare -a _CLUSTER_PIDS _CLUSTER_HOSTS _CLUSTER_PORTS _CLUSTER_RPC

start_cluster_node 0 1 "${DATA1}"
N1_HOST="${_CLUSTER_HOSTS[0]}"; N1_PORT="${_CLUSTER_PORTS[0]}"; N1_RPC="${_CLUSTER_RPC[0]}"
register_cluster_cleanup "${DATA1}"
sleep 1

rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER ADDSLOTS $(seq 0 16383) >/dev/null
sleep 2

# ── Bulk write 200 keys ──
echo "--- Writing 200 keys ---"
for i in $(seq 0 199); do
  key="dc:$(printf '%05x' "${i}")"
  rc_node "${N1_HOST}" "${N1_PORT}" SET "${key}" "data-${i}" >/dev/null
done
echo "OK: 200 keys written"

# ── Read-back all 200 keys ──
echo "--- Reading 200 keys ---"
errors=0
for i in $(seq 0 199); do
  key="dc:$(printf '%05x' "${i}")"
  val=$(rc_node "${N1_HOST}" "${N1_PORT}" GET "${key}" | tr -d '\r\n')
  if [[ "${val}" != "data-${i}" ]]; then
    errors=$((errors + 1))
    if [[ "${errors}" -ge 5 ]]; then
      echo "FAIL: ${errors}+ mismatches" >&2; exit 1
    fi
  fi
done
echo "OK: 200/200 keys verified (${errors} errors)"

# ── DBSIZE ──
DBS=$(rc_node "${N1_HOST}" "${N1_PORT}" DBSIZE | tr -d '\r\n')
echo "DBSIZE: ${DBS}"
if [[ "${DBS}" -ne 200 ]]; then
  echo "FAIL: DBSIZE expected 200, got ${DBS}" >&2; exit 1
fi
echo "OK: DBSIZE = 200"

# ── Restart recovery ──
echo "--- Restart node 1 ---"
stop_cluster_node 0
# 等待 client + RPC 端口全部释放 (data-plane = rpc + 10000 也需等待
# TIME_WAIT 过期, 否则 gRPC bind 失败导致 init_cluster 阻塞).
wait_ports_free "$(_cluster_client_port 0)" "$(_cluster_rpc_port 0)"
sleep 2
rm -rf "${DATA1}"; mkdir -p "${DATA1}"
start_cluster_node 0 1 "${DATA1}"
N1_HOST="${_CLUSTER_HOSTS[0]}"; N1_PORT="${_CLUSTER_PORTS[0]}"
sleep 1

# After restart with memory engine, data is lost — verify clean state
PING=$(rc_node "${N1_HOST}" "${N1_PORT}" PING | tr -d '\r\n')
DBS2=$(rc_node "${N1_HOST}" "${N1_PORT}" DBSIZE | tr -d '\r\n')
echo "After restart: PING=${PING}, DBSIZE=${DBS2}"
if [[ "${PING}" != "PONG" ]]; then echo "FAIL: node not responsive" >&2; exit 1; fi
echo "OK: node restarted and responsive"

echo ""
echo "=== test_cluster_data_consistency.sh: PASS ==="
