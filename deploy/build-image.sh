#!/usr/bin/env bash
set -Eeuo pipefail

usage() {
    printf 'Usage: %s [--local]\n' "$(basename "$0")" >&2
}

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
AIKV_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
AIKV_PARENT="$(dirname -- "$AIKV_ROOT")"

if [[ "$(basename -- "$AIKV_PARENT")" == ".worktrees" ]]; then
    WORKSPACE_ROOT="$(cd -- "$AIKV_ROOT/../.." && pwd)"
    IN_WORKTREE=1
else
    WORKSPACE_ROOT="$(cd -- "$AIKV_ROOT/.." && pwd)"
    IN_WORKTREE=0
fi

IMAGE="${AIKV_IMAGE:-aikv:dev}"

copy_context_tree() {
    local source="$1"
    local destination="$2"

    tar -C "$source" \
        --exclude='.git' \
        --exclude='target' \
        --exclude='.runtime' \
        --exclude='.env' \
        --exclude='.env.*' \
        --exclude='.venv*' \
        --exclude='*.log' \
        --exclude='*.pid' \
        -cf - . | tar -C "$destination" -xf -
}

cleanup_local_context() {
    if [[ -n "${LOCAL_CONTEXT:-}" && -d "$LOCAL_CONTEXT" ]]; then
        rm -rf -- "$LOCAL_CONTEXT"
    fi
}

case "$#" in
    0)
        printf 'Building %s with GitHub main aidb dependency\n' "$IMAGE"
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
        if (( IN_WORKTREE )); then
            LOCAL_CONTEXT="$(mktemp -d "${TMPDIR:-/tmp}/aikv-local-context.XXXXXX")"
            trap cleanup_local_context EXIT
            mkdir -p "$LOCAL_CONTEXT/aikv" "$LOCAL_CONTEXT/aidb"
            copy_context_tree "$AIKV_ROOT" "$LOCAL_CONTEXT/aikv"
            copy_context_tree "$WORKSPACE_ROOT/aidb" "$LOCAL_CONTEXT/aidb"
            printf 'Using temporary Docker build context: %s\n' "$LOCAL_CONTEXT"
            docker build -f "$LOCAL_CONTEXT/aikv/deploy/Dockerfile.local" \
                -t "$IMAGE" \
                "$LOCAL_CONTEXT"
        else
            docker build -f "$AIKV_ROOT/deploy/Dockerfile.local" \
                -t "$IMAGE" \
                "$WORKSPACE_ROOT"
        fi
        ;;
    *)
        printf 'error: expected no arguments or --local\n' >&2
        usage
        exit 2
        ;;
esac
