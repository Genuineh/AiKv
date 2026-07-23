#!/usr/bin/env bash
# Shared helpers for AiKv e2e shell tests.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${ROOT}/target/release/aikv"
HOST="${AIKV_HOST:-127.0.0.1}"
PORT="${AIKV_PORT:-$((20000 + RANDOM % 40000))}"
ADDR="${HOST}:${PORT}"
DATA_DIR="${ROOT}/target/e2e-data-$$"

require_redis_cli() {
  if ! command -v redis-cli >/dev/null 2>&1; then
    echo "redis-cli is required for e2e tests" >&2
    exit 1
  fi
}

build_release() {
  # Always build with cluster features — non-cluster mode works fine with the same binary.
  cargo build --release --features cluster --manifest-path "${ROOT}/Cargo.toml"
}

start_server() {
  rm -rf "${DATA_DIR}"
  mkdir -p "${DATA_DIR}"
  "${BIN}" --bind "${ADDR}" --engine memory >/dev/null 2>&1 &
  SERVER_PID=$!
  trap 'kill "${SERVER_PID}" 2>/dev/null || true; rm -rf "${DATA_DIR}"' EXIT
  for _ in $(seq 1 50); do
    if redis-cli -h "${HOST}" -p "${PORT}" PING >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  echo "server failed to start on ${ADDR}" >&2
  exit 1
}

rc() {
  redis-cli -h "${HOST}" -p "${PORT}" "$@"
}

# ── Cluster mode helpers ───────────────────────────────────────────────

# Base port for cluster nodes; each node uses 3 ports (client, rpc, data-plane=rpc+10000).
# Use --cluster-data-port-offset to change the data-plane offset (default 10000). Must be consistent across all nodes.
# Nodes are spaced 200 apart to avoid data-plane port collisions.
CLUSTER_BASE_PORT="${AIKV_CLUSTER_BASE_PORT:-$((20000 + RANDOM % 40000))}"

# Port offsets per node index (0-based)
_cluster_client_port() { echo $((CLUSTER_BASE_PORT + $1 * 200)); }
_cluster_rpc_port()   { echo $((CLUSTER_BASE_PORT + $1 * 200 + 1)); }

build_release_cluster() {
  cargo build --release --features cluster --manifest-path "${ROOT}/Cargo.toml"
}

# wait_node HOST PORT [ATTEMPTS]
# 默认 30s 超时 (150 × 0.2s). 集群初始化需要 MetaRaft bootstrap +
# gRPC 启动 + LifecycleManager + 后台 task, 5s 在 CI 上不够.
wait_node() {
  local h="$1" p="$2" attempts="${3:-150}"
  for _ in $(seq 1 "${attempts}"); do
    if redis-cli -h "$h" -p "$p" PING >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.2
  done
  echo "node ${h}:${p} failed to start after ${attempts} attempts" >&2
  return 1
}

# wait_port_free PORT [ATTEMPTS]
wait_port_free() {
  local p="$1" attempts="${2:-60}"
  for _ in $(seq 1 "${attempts}"); do
    if ! nc -z 127.0.0.1 "$p" 2>/dev/null; then
      return 0
    fi
    sleep 0.2
  done
  echo "port ${p} still in use after ${attempts} attempts" >&2
  return 1
}

# wait_ports_free PORT... — wait for multiple ports to be free
wait_ports_free() {
  local attempts=60
  for _ in $(seq 1 "${attempts}"); do
    local all_free=true
    for p in "$@"; do
      if nc -z 127.0.0.1 "$p" 2>/dev/null; then
        all_free=false
        break
      fi
    done
    if $all_free; then
      return 0
    fi
    sleep 0.2
  done
  echo "ports $* still in use after ${attempts} attempts" >&2
  return 1
}

# rc_node HOST PORT [ARGS...]
rc_node() {
  local h="$1" p="$2"; shift 2
  redis-cli -h "$h" -p "$p" "$@"
}

# start_cluster_node NODE_INDEX NODE_ID DATA_DIR [PEERS...]
# NODE_INDEX: 0-based index for port calculation (0, 1, 2)
# NODE_ID: u64 node id (1, 2, 3)
# DATA_DIR: persistent data directory for this node
# PEERS: optional RPC addresses of existing nodes
# Sets: _CLUSTER_PIDS[NODE_INDEX], _CLUSTER_HOSTS[NODE_INDEX], _CLUSTER_PORTS[NODE_INDEX]
start_cluster_node() {
  local idx="$1" node_id="$2" data_dir="$3"; shift 3
  local peers=("$@")

  local client_port
  local rpc_port
  client_port=$(_cluster_client_port "${idx}")
  rpc_port=$(_cluster_rpc_port "${idx}")

  rm -rf "${data_dir}"
  mkdir -p "${data_dir}"

  if [[ ${#peers[@]} -gt 0 ]]; then
    "${BIN}" \
      --bind "127.0.0.1:${client_port}" \
      --engine memory \
      --data-dir "${data_dir}" \
      --cluster-node-id "${node_id}" \
      --cluster-rpc-addr "127.0.0.1:${rpc_port}" \
      --cluster-peers "$(IFS=','; echo "${peers[*]}")" \
      >"${data_dir}/server.log" 2>&1 &
  else
    "${BIN}" \
      --bind "127.0.0.1:${client_port}" \
      --engine memory \
      --data-dir "${data_dir}" \
      --cluster-node-id "${node_id}" \
      --cluster-rpc-addr "127.0.0.1:${rpc_port}" \
      >"${data_dir}/server.log" 2>&1 &
  fi

  _CLUSTER_PIDS["${idx}"]=$!
  _CLUSTER_HOSTS["${idx}"]="127.0.0.1"
  _CLUSTER_PORTS["${idx}"]="${client_port}"
  _CLUSTER_RPC["${idx}"]="127.0.0.1:${rpc_port}"

  wait_node "127.0.0.1" "${client_port}" || return 1
}

# stop_cluster_node NODE_INDEX
stop_cluster_node() {
  local idx="$1"
  local pid="${_CLUSTER_PIDS[${idx}]:-}"
  if [[ -n "${pid}" ]] && kill -0 "${pid}" 2>/dev/null; then
    kill "${pid}" 2>/dev/null || true
    wait "${pid}" 2>/dev/null || true
  fi
}

# register_cluster_cleanup — call after all start_cluster_node calls to set up EXIT trap.
# 使用 pkill 按端口模式清理, 避免 trap 捕获的 PID 在节点重启后变为过期值.
register_cluster_cleanup() {
  local data_dirs=("$@")
  # Build a pkill pattern that matches the exact cluster ports used by this test.
  # This survives process restarts (unlike hard-coded PID lists).
  local port_pattern=""
  for idx in 0 1 2; do
    local cp; cp=$(_cluster_client_port "${idx}")
    port_pattern="${port_pattern}${port_pattern:+,}${cp}"
  done
  trap 'pkill -f "aikv.*--bind.*($(echo '"${port_pattern}"' | tr "," "|"))" 2>/dev/null || true; rm -rf '"${data_dirs[*]}"'; wait 2>/dev/null || true' EXIT
}
