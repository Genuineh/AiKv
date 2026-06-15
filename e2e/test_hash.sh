#!/usr/bin/env bash
# E2E: Hash commands
set -euo pipefail
source "$(dirname "$0")/utils.sh"

require_redis_cli
build_release
start_server

echo "==> Hash E2E tests on ${ADDR}"

cli() { redis-cli -h "${HOST}" -p "${PORT}" "$@" 2>&1; }

# HSET / HGET — use test instead of grep to avoid \r / format issues
cli DEL myhash >/dev/null || true

h1=$(cli HSET myhash field1 hello | tr -dc '0-9')
test "$h1" = "1" || { echo "FAIL: HSET field1 got '$h1'"; exit 1; }

h2=$(cli HSET myhash field2 world | tr -dc '0-9')
test "$h2" = "1" || { echo "FAIL: HSET field2 got '$h2'"; exit 1; }

h3=$(cli HSET myhash field1 hello | tr -dc '0-9')
test "$h3" = "0" || { echo "FAIL: HSET field1 again got '$h3'"; exit 1; }

g1=$(cli HGET myhash field1 | tr -d '\r\n')
test "$g1" = "hello" || { echo "FAIL: HGET field1 got '$g1'"; exit 1; }

g2=$(cli HGET myhash field2 | tr -d '\r\n')
test "$g2" = "world" || { echo "FAIL: HGET field2 got '$g2'"; exit 1; }

# null bulk string → empty when piped (not "(nil)")
g3=$(cli HGET myhash nonexistent 2>/dev/null)
test -z "$g3" || { echo "FAIL: HGET nonexistent got '$g3'"; exit 1; }

# HDEL
d1=$(cli HDEL myhash field1 | tr -dc '0-9')
test "$d1" = "1" || { echo "FAIL: HDEL field1 got '$d1'"; exit 1; }
d2=$(cli HDEL myhash field1 | tr -dc '0-9')
test "$d2" = "0" || { echo "FAIL: HDEL field1 again got '$d2'"; exit 1; }
test -z "$(cli HGET myhash field1 2>/dev/null)"

# HEXISTS
test "$(cli HEXISTS myhash field2 | tr -dc '0-9')" = "1"
test "$(cli HEXISTS myhash field1 | tr -dc '0-9')" = "0"
test "$(cli HEXISTS nonexistent field1 | tr -dc '0-9')" = "0"

# HLEN
cli HSET myhash f1 a f2 b f3 c >/dev/null
test "$(cli HLEN myhash | tr -dc '0-9')" = "4"
test "$(cli HLEN nonexistent | tr -dc '0-9')" = "0"

# HKEYS / HVALS / HGETALL
test "$(cli HKEYS myhash | sort | tr '\n' ' ' | xargs)" = "f1 f2 f3 field2"
test "$(cli HVALS myhash | sort | tr '\n' ' ' | xargs)" = "a b c world"
cli HGETALL myhash | tr '\n' ' ' | grep -q 'field2 world'

# HSETNX
test "$(cli HSETNX myhash newfield val | tr -dc '0-9')" = "1"
test "$(cli HSETNX myhash newfield val | tr -dc '0-9')" = "0"

# HINCRBY
cli HSET mycounter val 10 >/dev/null
test "$(cli HINCRBY mycounter val 5 | tr -dc '0-9')" = "15"
test "$(cli HINCRBY mycounter val -3 | tr -dc '0-9')" = "12"

# WRONGTYPE error
cli SET strkey strval >/dev/null
cli HGET strkey field | grep -qi "wrongtype" || {
  echo "FAIL: expected WRONGTYPE" >&2
  exit 1
}

echo "test_hash.sh: OK"
