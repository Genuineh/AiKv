#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
RUNTIME_DIR="$SCRIPT_DIR/.runtime/single"
CONFIG_PATH="$RUNTIME_DIR/aikv.toml"
COMPOSE_FILE="$SCRIPT_DIR/docker-compose.single.yaml"
PROJECT_NAME="aikv-single"
STARTUP_TIMEOUT_SECONDS="${AIKV_STARTUP_TIMEOUT_SECONDS:-60}"

if ! [[ "$STARTUP_TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]]; then
    printf 'error: AIKV_STARTUP_TIMEOUT_SECONDS must be a positive integer\n' >&2
    exit 2
fi

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        printf 'error: required command not found: %s\n' "$1" >&2
        exit 1
    fi
}

require_command docker
require_command redis-cli
require_command curl
docker compose version >/dev/null

mkdir -p "$RUNTIME_DIR"
cp "$SCRIPT_DIR/aikv.example.toml" "$CONFIG_PATH"
sed -i \
    -e 's|^bind = .*|bind = "0.0.0.0:6379" # 仅用于容器内监听; 宿主端口映射由 Docker 提供|' \
    -e 's|^metrics_addr = .*|metrics_addr = "0.0.0.0" # 仅用于容器内监听; 宿主端口映射由 Docker 提供|' \
    "$CONFIG_PATH"

docker compose \
    -p "$PROJECT_NAME" \
    -f "$COMPOSE_FILE" \
    up -d

deadline=$((SECONDS + STARTUP_TIMEOUT_SECONDS))
while (( SECONDS < deadline )); do
    if [[ "$(redis-cli -h 127.0.0.1 -p 6379 -t 1 ping 2>/dev/null || true)" == "PONG" ]]; then
        break
    fi
    sleep 1
done

if (( SECONDS >= deadline )); then
    printf 'error: aikv single did not answer PONG within %ss\n' \
        "$STARTUP_TIMEOUT_SECONDS" >&2
    docker compose -p "$PROJECT_NAME" -f "$COMPOSE_FILE" ps || true
    exit 1
fi

if ! curl -fsS --max-time 5 http://127.0.0.1:9191/health >/dev/null; then
    printf 'error: aikv single health endpoint failed\n' >&2
    docker compose -p "$PROJECT_NAME" -f "$COMPOSE_FILE" ps || true
    exit 1
fi

printf 'aikv single is ready on 127.0.0.1:6379\n'
