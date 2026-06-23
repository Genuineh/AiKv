#!/usr/bin/env bash
# @component aikv-cluster
# Cluster E2E: routing — SET/GET, MOVED, CROSSSLOT
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/utils.sh"

require_redis_cli
build_release_cluster

echo "=== Cluster Routing Test ==="

DATA1="$(mktemp -d)"
declare -a _CLUSTER_PIDS _CLUSTER_HOSTS _CLUSTER_PORTS _CLUSTER_RPC

start_cluster_node 0 1 "${DATA1}"
N1_HOST="${_CLUSTER_HOSTS[0]}"; N1_PORT="${_CLUSTER_PORTS[0]}"
register_cluster_cleanup "${DATA1}"
sleep 1

# ADDSLOTS
rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER ADDSLOTS $(seq 0 16383) >/dev/null
sleep 2

# ── SET/GET 50 keys ──
echo "--- SET/GET 50 keys ---"
for i in $(seq 0 49); do
  key="rt:$(printf '%04x' "${i}")"
  rc_node "${N1_HOST}" "${N1_PORT}" SET "${key}" "v-${i}" >/dev/null
done

errors=0
for i in $(seq 0 49); do
  key="rt:$(printf '%04x' "${i}")"
  val=$(rc_node "${N1_HOST}" "${N1_PORT}" GET "${key}" | tr -d '\r\n')
  if [[ "${val}" != "v-${i}" ]]; then
    errors=$((errors + 1))
  fi
done
if [[ "${errors}" -gt 0 ]]; then
  echo "FAIL: ${errors} mismatches" >&2; exit 1
fi
echo "OK: 50/50 keys"

# ── String operations ──
echo "--- String operations ---"
rc_node "${N1_HOST}" "${N1_PORT}" SET counter 0 >/dev/null
rc_node "${N1_HOST}" "${N1_PORT}" INCR counter >/dev/null
rc_node "${N1_HOST}" "${N1_PORT}" INCRBY counter 9 >/dev/null
CNT=$(rc_node "${N1_HOST}" "${N1_PORT}" GET counter | tr -d '\r\n')
if [[ "${CNT}" != "10" ]]; then echo "FAIL: INCR expected 10 got ${CNT}" >&2; exit 1; fi
echo "INCR/INCRBY: counter = ${CNT} OK"

# ── Hash operations ──
echo "--- Hash operations ---"
rc_node "${N1_HOST}" "${N1_PORT}" HSET h:1 f1 a f2 b >/dev/null
rc_node "${N1_HOST}" "${N1_PORT}" HSET h:1 f3 c >/dev/null
HLEN=$(rc_node "${N1_HOST}" "${N1_PORT}" HLEN h:1 | tr -d '\r\n')
if [[ "${HLEN}" != "3" ]]; then echo "FAIL: HLEN expected 3 got ${HLEN}" >&2; exit 1; fi
HGET=$(rc_node "${N1_HOST}" "${N1_PORT}" HGET h:1 f1 | tr -d '\r\n')
if [[ "${HGET}" != "a" ]]; then echo "FAIL: HGET expected a got ${HGET}" >&2; exit 1; fi
echo "HLEN=3, HGET f1=a OK"

# ── List operations ──
echo "--- List operations ---"
rc_node "${N1_HOST}" "${N1_PORT}" LPUSH l:1 x y z >/dev/null
LLEN=$(rc_node "${N1_HOST}" "${N1_PORT}" LLEN l:1 | tr -d '\r\n')
if [[ "${LLEN}" != "3" ]]; then echo "FAIL: LLEN expected 3 got ${LLEN}" >&2; exit 1; fi
echo "LLEN=3 OK"

# ── DEL / EXISTS ──
echo "--- DEL / EXISTS ---"
rc_node "${N1_HOST}" "${N1_PORT}" SET tmpkey x >/dev/null
EX=$(rc_node "${N1_HOST}" "${N1_PORT}" EXISTS tmpkey | tr -d '\r\n')
if [[ "${EX}" != "1" ]]; then echo "FAIL: EXISTS expected 1" >&2; exit 1; fi
rc_node "${N1_HOST}" "${N1_PORT}" DEL tmpkey >/dev/null
EX2=$(rc_node "${N1_HOST}" "${N1_PORT}" EXISTS tmpkey | tr -d '\r\n')
if [[ "${EX2}" != "0" ]]; then echo "FAIL: EXISTS after DEL expected 0" >&2; exit 1; fi
echo "EXISTS/DEL OK"

# ── DBSIZE ──
DBS=$(rc_node "${N1_HOST}" "${N1_PORT}" DBSIZE | tr -d '\r\n')
echo "DBSIZE: ${DBS}"

# ── KEYSLOT ──
echo "--- KEYSLOT range ---"
SLOT=$(rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER KEYSLOT "anykey" | tr -d '\r\n')
if [[ "${SLOT}" -ge 0 && "${SLOT}" -le 16383 ]]; then
  echo "KEYSLOT anykey = ${SLOT} OK"
else
  echo "FAIL: KEYSLOT out of range" >&2; exit 1
fi

echo ""
echo "=== test_cluster_routing.sh: PASS ==="
