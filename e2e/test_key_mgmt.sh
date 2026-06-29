#!/usr/bin/env bash
# E2E: Key management commands (RENAME/EXPIRE/TTL/EXISTS/TYPE/COPY)
set -euo pipefail
source "$(dirname "$0")/utils.sh"

require_redis_cli
build_release
start_server

echo "==> Key management E2E tests on ${ADDR}"

# EXISTS / DEL
rc DEL k1 k2 >/dev/null || true
rc SET k1 v1 >/dev/null
rc EXISTS k1 | grep -q '^1$'
rc EXISTS k1 k2 | grep -q '^1$'
rc SET k2 v2 >/dev/null
rc EXISTS k1 k2 | grep -q '^2$'
rc DEL k1 | grep -q '^1$'
rc EXISTS k1 | grep -q '^0$'

# TYPE
rc SET strkey val >/dev/null
rc TYPE strkey | tr -d '\r' | grep -qi "string"
rc HSET hashkey f v >/dev/null
rc TYPE hashkey | tr -d '\r' | grep -qi "hash"
rc TYPE nonexistent | tr -d '\r' | grep -qi "none"

# RENAME / RENAMENX
rc SET oldkey oldval >/dev/null
rc RENAME oldkey newkey >/dev/null
test -z "$(rc GET oldkey 2>/dev/null | xargs)"  # null → empty when piped
rc GET newkey | tr -d '\r' | grep -q '^oldval$'

rc SET a aa >/dev/null
rc SET b bb >/dev/null
rc RENAMENX a b | grep -q '^0$'
rc RENAMENX a c | grep -q '^1$'

# EXPIRE / TTL / PERSIST
rc SET tmp val >/dev/null
rc EXPIRE tmp 9999 | grep -q '^1$'
rc TTL tmp | grep -q '999[0-9]'
rc PERSIST tmp | grep -q '^1$'
rc TTL tmp | grep -q '^-1$'

# PEXPIRE / PTTL
rc PEXPIRE tmp 9999999 | grep -q '^1$'
rc PTTL tmp | grep -q '^999'

# EXPIREAT / PEXPIREAT
rc SET tmp2 val2 >/dev/null
rc EXPIREAT tmp2 9999999999 | grep -q '^1$'
rc TTL tmp2 | grep -q '^[0-9]'

# COPY
rc SET src val >/dev/null
rc COPY src dst >/dev/null
rc GET src | tr -d '\r' | grep -q '^val$'
rc GET dst | tr -d '\r' | grep -q '^val$'

echo "test_key_mgmt.sh: OK"
