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
docker compose version >/dev/null

single_compose() {
    docker compose -p aikv-single -f "$SINGLE_COMPOSE" "$@"
}

cluster_compose() {
    docker compose -p aikv-cluster -f "$CLUSTER_COMPOSE" "$@"
}

mode=""
purge=0
for arg in "$@"; do
    case "$arg" in
        single|cluster)
            [[ -z "$mode" ]] || {
                printf 'error: deployment mode specified more than once\n' >&2
                exit 2
            }
            mode="$arg"
            ;;
        --purge)
            [[ "$purge" == 0 ]] || {
                printf 'error: --purge specified more than once\n' >&2
                exit 2
            }
            purge=1
            ;;
        *)
            printf 'Usage: %s [single|cluster] [--purge]\n' \
                "$(basename "$0")" >&2
            exit 2
            ;;
    esac
done

single_ids="$(single_compose ps -aq)"
cluster_ids="$(cluster_compose ps -aq)"

if [[ -z "$mode" ]]; then
    if [[ -n "$single_ids" && -n "$cluster_ids" ]]; then
        printf '%s\n' \
            'error: both single and cluster deployments exist; specify single or cluster' >&2
        exit 2
    elif [[ -n "$single_ids" ]]; then
        mode="single"
    elif [[ -n "$cluster_ids" ]]; then
        mode="cluster"
    else
        printf '%s\n' 'no deployment found'
        exit 1
    fi
fi

case "$mode" in
    single)
        single_compose config --quiet
        [[ -n "$single_ids" ]] || {
            printf '%s\n' 'no deployment found'
            exit 1
        }
        compose=(single_compose)
        ;;
    cluster)
        cluster_compose config --quiet
        [[ -n "$cluster_ids" ]] || {
            printf '%s\n' 'no deployment found'
            exit 1
        }
        compose=(cluster_compose)
        ;;
esac

if (( purge )); then
    "${compose[@]}" down --volumes
else
    "${compose[@]}" down
fi
