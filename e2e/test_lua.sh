#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/utils.sh"

require_redis_cli
build_release
start_server

rc EVAL "return 1+1" 0 | grep -q '^2$'

SHA=$(rc SCRIPT LOAD "return 'lua-ok'" | tr -d '\r\n')
rc EVALSHA "$SHA" 0 | grep -q 'lua-ok'

echo "test_lua.sh: OK"
