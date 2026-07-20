#!/usr/bin/env bash
# E2E: Extra string commands (APPEND/STRLEN/INCR/DECR/INCRBY/MGET/MSET)
set -euo pipefail
source "$(dirname "$0")/utils.sh"

require_redis_cli
build_release
start_server

echo "==> String extra E2E tests on ${ADDR}"

cli() { redis-cli -h "${HOST}" -p "${PORT}" "$@" 2>&1; }

# APPEND / STRLEN (integer replies)
cli DEL mystr >/dev/null || true

out=$(cli APPEND mystr hello 2>&1)
stripped=$(echo "${out}" | tr -dc '0-9')
[ "$stripped" = "5" ] || { echo "FAIL: APPEND1 expected 5 raw='${out}' stripped='${stripped}'" >&2; exit 1; }

out=$(cli APPEND mystr ' world' 2>&1)
stripped=$(echo "${out}" | tr -dc '0-9')
[ "$stripped" = "11" ] || { echo "FAIL: APPEND2 expected 11 raw='${out}' stripped='${stripped}'" >&2; exit 1; }

cli GET mystr | tr -d '\r' | grep -q '^hello world$' || { echo "FAIL: GET mystr" >&2; exit 1; }

out=$(cli STRLEN mystr 2>&1)
stripped=$(echo "${out}" | tr -dc '0-9')
[ "$stripped" = "11" ] || { echo "FAIL: STRLEN expected 11 raw='${out}'" >&2; exit 1; }

out=$(cli STRLEN nonexistent 2>&1)
stripped=$(echo "${out}" | tr -dc '0-9')
[ "$stripped" = "0" ] || { echo "FAIL: STRLEN nonexistent expected 0 raw='${out}'" >&2; exit 1; }

# INCR / DECR / INCRBY / DECRBY
cli DEL mycounter >/dev/null || true
out=$(cli INCR mycounter 2>&1); stripped=$(echo "${out}" | tr -dc '0-9')
[ "$stripped" = "1" ] || { echo "FAIL: INCR1 raw='${out}'" >&2; exit 1; }
out=$(cli INCR mycounter 2>&1); stripped=$(echo "${out}" | tr -dc '0-9')
[ "$stripped" = "2" ] || { echo "FAIL: INCR2 raw='${out}'" >&2; exit 1; }
out=$(cli DECR mycounter 2>&1); stripped=$(echo "${out}" | tr -dc '0-9')
[ "$stripped" = "1" ] || { echo "FAIL: DECR raw='${out}'" >&2; exit 1; }
out=$(cli INCRBY mycounter 10 2>&1); stripped=$(echo "${out}" | tr -dc '0-9')
[ "$stripped" = "11" ] || { echo "FAIL: INCRBY raw='${out}'" >&2; exit 1; }
out=$(cli DECRBY mycounter 3 2>&1); stripped=$(echo "${out}" | tr -dc '0-9')
[ "$stripped" = "8" ] || { echo "FAIL: DECRBY raw='${out}'" >&2; exit 1; }
out=$(cli INCRBYFLOAT mycounter 2.5 2>&1); cleaned=$(echo "${out}" | tr -d '\r\n')
[ "$cleaned" = "10.5" ] || { echo "FAIL: INCRBYFLOAT raw='${out}' cleaned='${cleaned}'" >&2; exit 1; }

# MGET / MSET
cli MSET k1 v1 k2 v2 k3 v3 >/dev/null
test "$(cli MGET k1 k2 k3 | xargs)" = "v1 v2 v3" || { echo "FAIL: MGET all" >&2; exit 1; }
test "$(cli MGET k1 nonexistent k3 | xargs)" = "v1 v3" || { echo "FAIL: MGET null" >&2; exit 1; }

# GET after SET (GETSET not implemented in AiKv)
cli SET mygs oldval >/dev/null
cli GET mygs | tr -d '\r' | grep -q '^oldval$'

echo "test_string_extra.sh: OK"
