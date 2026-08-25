#!/usr/bin/env bash
set -Eeuo pipefail

usage() {
    printf 'Usage: %s [--local]\n' "$(basename "$0")" >&2
}

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
AIKV_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
AIKV_PARENT="$(dirname -- "$AIKV_ROOT")"

if [[ "$(basename -- "$AIKV_PARENT")" == ".worktrees" ]]; then
    WORKSPACE_ROOT="$(cd -- "$AIKV_ROOT/../../.." && pwd)"
else
    WORKSPACE_ROOT="$(cd -- "$AIKV_ROOT/.." && pwd)"
fi

IMAGE="${AIKV_IMAGE:-aikv:dev}"

case "$#" in
    0)
        printf 'Building %s with the cloud AiDb Git source\n' "$IMAGE"
        docker build -f "$AIKV_ROOT/deploy/Dockerfile" \
            -t "$IMAGE" \
            "$AIKV_ROOT"
        ;;
    1)
        if [[ "$1" != "--local" ]]; then
            printf 'error: unknown argument: %s\n' "$1" >&2
            usage
            exit 2
        fi

        if [[ ! -d "$WORKSPACE_ROOT/aidb" ]]; then
            printf 'error: local AiDb checkout not found: %s\n' \
                "$WORKSPACE_ROOT/aidb" >&2
            exit 1
        fi

        printf 'Building %s with the local AiDb source at /src/aidb\n' "$IMAGE"
        docker build -f "$AIKV_ROOT/deploy/Dockerfile.local" \
            -t "$IMAGE" \
            "$WORKSPACE_ROOT"
        ;;
    *)
        printf 'error: expected no arguments or --local\n' >&2
        usage
        exit 2
        ;;
esac
