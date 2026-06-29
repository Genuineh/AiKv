#!/usr/bin/env bash
# E2E: Lua / JSON / TTL 过期验证 (self-starting)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/utils.sh"

require_redis_cli
build_release
start_server

echo "==> Lua / JSON / TTL E2E tests on ${ADDR}"

# Cleanup test keys
rc DEL mykey doc ttlkey >/dev/null 2>&1 || true

# ── 1. Lua 脚本测试 ──
echo "--- [1/3] Lua EVAL test"
rc EVAL "return redis.call('SET', KEYS[1], ARGV[1])" 1 mykey myval >/dev/null
result=$(rc GET mykey)
if [ "$result" = "myval" ]; then
  echo "PASS: Lua SET via EVAL"
else
  echo "FAIL: expected 'myval', got '${result}'" >&2
  exit 1
fi

# ── 2. JSON 测试 ──
echo "--- [2/3] JSON.SET / JSON.GET test"
rc JSON.SET doc '$' '{"name":"alice","age":30}' >/dev/null
result=$(rc JSON.GET doc '$.name' 2>/dev/null)
if [ "$result" = '"alice"' ] || [ "$result" = '["alice"]' ]; then
  echo "PASS: JSON.GET doc $.name"
else
  echo "FAIL: expected '[\"alice\"]', got '${result}'" >&2
  exit 1
fi

# ── 3. TTL 过期测试 ──
echo "--- [3/3] TTL expiry test (EX 1, sleep 2)"
rc SET ttlkey val EX 1 >/dev/null
sleep 2
result=$(rc GET ttlkey)
if [ -z "$result" ] || [ "$result" = "(nil)" ]; then
  echo "PASS: TTL expired"
else
  echo "FAIL: TTL not expired, got '${result}'" >&2
  exit 1
fi

echo ""
echo "All Lua/JSON/TTL E2E tests passed."
