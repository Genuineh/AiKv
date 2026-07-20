#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/utils.sh"

require_redis_cli
build_release
start_server

# LINSERT
rc DEL mylist >/dev/null || true
rc LPUSH mylist a c | grep -q '^2$'
rc LINSERT mylist BEFORE a b | grep -q '^3$'
test "$(rc LRANGE mylist 0 -1 | tr '\n' ' ' | xargs)" = "c b a"

# SMOVE
rc DEL srcset dstset >/dev/null || true
rc SADD srcset x y | grep -q '^2$'
rc SADD dstset z | grep -q '^1$'
rc SMOVE srcset dstset y | grep -q '^1$'
test "$(rc SMEMBERS srcset | tr '\n' ' ' | xargs)" = "x"
test "$(rc SMEMBERS dstset | sort | tr '\n' ' ' | xargs)" = "y z"

# DUMP / RESTORE (AiKv internal format; pipe binary payload)
rc SET dumpkey hello | grep -q '^OK$'
rc DEL restorekey >/dev/null 2>&1 || true
rc --raw DUMP dumpkey | rc -x RESTORE restorekey 0
test "$(rc GET restorekey)" = "hello"

# SLOWLOG LEN (after slowlog-eligible work)
rc CONFIG SET slowlog-log-slower-than 0 | grep -q '^OK$'
rc SET slowkey slowval | grep -q '^OK$'
test "$(rc SLOWLOG LEN)" -ge 1

# EXPIRETIME
rc SET exkey exval EX 3600 | grep -q '^OK$'
ex="$(rc EXPIRETIME exkey)"
test "${ex}" -gt 0
rc SET noexkey novalue | grep -q '^OK$'
test "$(rc EXPIRETIME noexkey)" = "-1"
test "$(rc EXPIRETIME missing)" = "-2"

echo "test_ext.sh: OK"
