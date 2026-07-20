#!/usr/bin/env bash
# Cluster E2E: slot allocation — ADDSLOTS, SLOTS, KEYSLOT (single-node)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/utils.sh"

require_redis_cli
build_release_cluster

echo "=== Cluster Slots Test ==="

DATA1="$(mktemp -d)"
declare -a _CLUSTER_PIDS _CLUSTER_HOSTS _CLUSTER_PORTS _CLUSTER_RPC

start_cluster_node 0 1 "${DATA1}"
N1_HOST="${_CLUSTER_HOSTS[0]}"; N1_PORT="${_CLUSTER_PORTS[0]}"
register_cluster_cleanup "${DATA1}"
sleep 1

# ── ADDSLOTS ──
echo "--- ADDSLOTS (all 16384 slots) ---"
rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER ADDSLOTS $(seq 0 16383) | grep -q OK
sleep 2
echo "OK: all slots assigned"

# ── CLUSTER SLOTS ──
echo "--- CLUSTER SLOTS ---"
SLOTS_OUT="$(rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER SLOTS)"
echo "${SLOTS_OUT}" | head -3
# Verify non-empty
if [[ -z "$(echo "${SLOTS_OUT}" | tr -d '\r\n ')" ]]; then
  echo "FAIL: CLUSTER SLOTS returned empty" >&2; exit 1
fi
echo "OK: CLUSTER SLOTS populated"

# ── KEYSLOT determinism ──
echo "--- CLUSTER KEYSLOT ---"
S1=$(rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER KEYSLOT "testkey" | tr -d '\r\n')
S2=$(rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER KEYSLOT "testkey" | tr -d '\r\n')
if [[ "${S1}" != "${S2}" ]]; then
  echo "FAIL: KEYSLOT not deterministic" >&2; exit 1
fi
# Range check
if [[ "${S1}" -lt 0 || "${S1}" -gt 16383 ]]; then
  echo "FAIL: KEYSLOT ${S1} out of range" >&2; exit 1
fi
echo "testkey → slot ${S1} (deterministic, in range)"

# ── Hash tag ──
HASH=$(rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER KEYSLOT "{user1000}.following" | tr -d '\r\n')
TAG=$(rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER KEYSLOT "user1000" | tr -d '\r\n')
if [[ "${HASH}" != "${TAG}" ]]; then
  echo "FAIL: hash tag mismatch: ${HASH} vs ${TAG}" >&2; exit 1
fi
echo "{user1000}.following → slot ${HASH} = user1000 → ${TAG} (hash tag OK)"

# ── Different keys → different slots (spot check) ──
A=$(rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER KEYSLOT "alpha" | tr -d '\r\n')
B=$(rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER KEYSLOT "beta" | tr -d '\r\n')
echo "alpha → ${A}, beta → ${B}"

echo ""
echo "=== test_cluster_slots.sh: PASS ==="
