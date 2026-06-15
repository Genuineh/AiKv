#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/utils.sh"

require_redis_cli
build_release
start_server

rc SET foo bar | grep -q '^OK$'
test "$(rc GET foo)" = "bar"
rc SELECT 1 | grep -q '^OK$'
test "$(redis-cli -h "${HOST}" -p "${PORT}" -n 1 DBSIZE)" = "0"
test "$(redis-cli -h "${HOST}" -p "${PORT}" -n 0 DBSIZE)" = "1"

echo "test_basic.sh: OK"
