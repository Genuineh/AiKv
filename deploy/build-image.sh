#!/usr/bin/env bash
set -Eeuo pipefail

usage() {
    cat <<EOF
Usage: $(basename "$0") [-h|--help] [--local]

构建 aikv 容器镜像.
环境变量:
  AIKV_IMAGE    镜像名称与标签 (默认: aikv:dev)

选项:
  --local       使用工作区同层级本地 ../aidb 源码编译
  -h, --help    显示帮助信息
EOF
}

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
AIKV_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
AIKV_PARENT="$(dirname -- "$AIKV_ROOT")"

if [[ "$(basename -- "$AIKV_PARENT")" == ".worktrees" ]]; then
    WORKSPACE_ROOT="$(cd -- "$AIKV_ROOT/../.." && pwd)"
else
    WORKSPACE_ROOT="$(cd -- "$AIKV_ROOT/.." && pwd)"
fi

IMAGE="${AIKV_IMAGE:-aikv:dev}"
export DOCKER_BUILDKIT=1

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
        printf 'Building %s with GitHub main aidb dependency...\n' "$IMAGE"
        docker build -f "$AIKV_ROOT/deploy/Dockerfile" \
            -t "$IMAGE" \
            "$AIKV_ROOT"
        printf 'Successfully built %s\n' "$IMAGE"
        ;;
    1)
        case "$1" in
            -h|--help)
                usage
                exit 0
                ;;
            --local)
                if [[ ! -d "$WORKSPACE_ROOT/aidb" ]]; then
                    printf 'error: local AiDb checkout not found: %s\n' \
                        "$WORKSPACE_ROOT/aidb" >&2
                    exit 1
                fi

                printf 'Building %s with local AiDb source at %s...\n' "$IMAGE" "$WORKSPACE_ROOT/aidb"
                LOCAL_CONTEXT="$(mktemp -d "${TMPDIR:-/tmp}/aikv-local-context.XXXXXX")"
                trap cleanup_local_context EXIT
                mkdir -p "$LOCAL_CONTEXT/aikv" "$LOCAL_CONTEXT/aidb"
                copy_context_tree "$AIKV_ROOT" "$LOCAL_CONTEXT/aikv"
                copy_context_tree "$WORKSPACE_ROOT/aidb" "$LOCAL_CONTEXT/aidb"
                docker build -f "$LOCAL_CONTEXT/aikv/deploy/Dockerfile.local" \
                    -t "$IMAGE" \
                    "$LOCAL_CONTEXT"
                printf 'Successfully built %s\n' "$IMAGE"
                ;;
            *)
                printf 'error: unknown argument: %s\n' "$1" >&2
                usage >&2
                exit 2
                ;;
        esac
        ;;
    *)
        printf 'error: too many arguments\n' >&2
        usage >&2
        exit 2
        ;;
esac
