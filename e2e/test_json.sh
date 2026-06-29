#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/utils.sh"

require_redis_cli
build_release
start_server

rc DEL jk >/dev/null || true
rc JSON.SET jk '$' '{"name":"John","age":30}' | grep -q '^OK$'
got="$(rc JSON.GET jk '$')"
echo "$got" | grep -q 'John'
echo "$got" | grep -q '30'

rc JSON.SET jk '$.age' '31' | grep -q '^OK$'
test "$(rc JSON.GET jk '$.age')" = "31"

rc JSON.TYPE jk '$.name' | grep -q '^string$'
rc JSON.DEL jk '$.name' | grep -q '^1$'

echo "test_json.sh: OK"
