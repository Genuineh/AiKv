#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
SINGLE_COMPOSE="$SCRIPT_DIR/docker-compose.single.yaml"
CLUSTER_COMPOSE="$SCRIPT_DIR/docker-compose.cluster.yaml"

require_command() {
    command -v "$1" >/dev/null 2>&1 || {
        printf 'error: required command not found: %s\n' "$1" >&2
        exit 1
    }
}

require_command docker
require_command redis-cli
require_command curl
docker compose version >/dev/null

single_compose() {
    docker compose -p aikv-single -f "$SINGLE_COMPOSE" "$@"
}

cluster_compose() {
    docker compose -p aikv-cluster -f "$CLUSTER_COMPOSE" "$@"
}

single_ids="$(single_compose ps -aq)"
cluster_ids="$(cluster_compose ps -aq)"

validate_mode() {
    case "$1" in
        single)
            single_compose config --quiet
            ;;
        cluster)
            cluster_compose config --quiet
            ;;
        *)
            printf 'error: mode must be single or cluster\n' >&2
            exit 2
            ;;
    esac
}

status_single() {
    local running ping
    printf '%s\n' '[single]'
    single_compose ps -a
    running="$(single_compose ps --status running --services)"
    if [[ "$running" != *"aikv-single"* ]]; then
        printf '%s\n' 'state: stopped'
        return 1
    fi

    ping="$(redis-cli -h 127.0.0.1 -p 6379 -t 1 ping 2>/dev/null || true)"
    if [[ "$ping" != "PONG" ]]; then
        printf 'redis: failed (%s)\n' "${ping:-no response}"
        return 1
    fi
    if ! curl -fsS --max-time 5 http://127.0.0.1:9191/health >/dev/null; then
        printf '%s\n' 'metrics: failed'
        return 1
    fi
    printf '%s\n' 'redis: PONG' 'metrics: OK'
}

status_cluster() {
    local running info state known size assigned nodes
    local master_count replica_count
    local node port index
    local client_ports=(6379 6380 6381 7379 7380 7381)

    printf '%s\n' '[cluster]'
    cluster_compose ps -a
    running="$(cluster_compose ps --status running --services)"
    for node in aikv-1 aikv-2 aikv-3 aikv-4 aikv-5 aikv-6; do
        if [[ "$running" != *"$node"* ]]; then
            printf 'state: required service %s is not running\n' "$node"
            return 1
        fi
    done

    for index in "${!client_ports[@]}"; do
        node=$((index + 1))
        port="${client_ports[$index]}"
        if [[ "$(redis-cli -h 127.0.0.1 -p "$port" -t 1 ping 2>/dev/null || true)" != "PONG" ]]; then
            printf 'redis: node%s failed\n' "$node"
            return 1
        fi
        if ! curl -fsS --max-time 5 "http://127.0.0.1:$((9190 + node))/health" >/dev/null; then
            printf 'metrics: node%s failed\n' "$node"
            return 1
        fi
    done

    info="$(redis-cli -h 127.0.0.1 -p 6379 -t 1 --raw CLUSTER INFO)"
    nodes="$(redis-cli -h 127.0.0.1 -p 6379 -t 1 --raw CLUSTER NODES)"
    printf '%s\n' "$info"
    state="$(awk -F: '$1 == "cluster_state" { print $2 }' <<< "$info")"
    known="$(awk -F: '$1 == "cluster_known_nodes" { print $2 }' <<< "$info")"
    size="$(awk -F: '$1 == "cluster_size" { print $2 }' <<< "$info")"
    assigned="$(awk -F: '$1 == "cluster_slots_assigned" { print $2 }' <<< "$info")"
    master_count="$(awk '$3 ~ /(^|,)master(,|$)/ { count++ } END { print count + 0 }' <<< "$nodes")"
    replica_count="$(awk '$3 ~ /(^|,)slave(,|$)/ { count++ } END { print count + 0 }' <<< "$nodes")"
    printf 'topology: known=%s masters=%s replicas=%s size=%s slots=%s state=%s\n' \
        "$known" "$master_count" "$replica_count" "$size" "$assigned" "$state"

    [[ "$state" == "ok" && "$known" == "6" && "$size" == "2" &&
        "$assigned" == "16384" && "$master_count" == "2" &&
        "$replica_count" == "4" ]]
}

if (( $# > 1 )); then
    printf 'Usage: %s [single|cluster]\n' "$(basename "$0")" >&2
    exit 2
fi

if (( $# == 1 )); then
    mode="$1"
    validate_mode "$mode"
    case "$mode" in
        single)
            [[ -n "$single_ids" ]] || {
                printf '%s\n' 'no deployment found'
                exit 1
            }
            status_single
            ;;
        cluster)
            [[ -n "$cluster_ids" ]] || {
                printf '%s\n' 'no deployment found'
                exit 1
            }
            status_cluster
            ;;
    esac
    exit $?
fi

if [[ -z "$single_ids" && -z "$cluster_ids" ]]; then
    printf '%s\n' 'no deployment found'
    exit 1
fi

result=0
if [[ -n "$single_ids" ]]; then
    validate_mode single
    status_single || result=1
fi
if [[ -n "$cluster_ids" ]]; then
    validate_mode cluster
    status_cluster || result=1
fi
exit "$result"
