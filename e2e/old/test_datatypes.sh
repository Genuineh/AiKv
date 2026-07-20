#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/utils.sh"

require_redis_cli
build_release
start_server

rc DEL mylist >/dev/null || true
rc LPUSH mylist a b c | grep -q '^3$'
test "$(rc LRANGE mylist 0 -1 | tr '\n' ' ' | xargs)" = "c b a"

rc DEL myset >/dev/null || true
rc SADD myset x y | grep -q '^2$'
rc SMEMBERS myset | sort | tr '\n' ' ' | grep -q 'x y'

rc DEL myz >/dev/null || true
rc ZADD myz 1 m 2 n | grep -q '^2$'
test "$(rc ZRANGE myz 0 -1 | tr '\n' ' ' | xargs)" = "m n"

echo "test_datatypes.sh: OK"
