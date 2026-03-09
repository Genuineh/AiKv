#!/bin/bash
# AiKv 冒烟测试 (Smoke Test)
# 快速验证：能编译、能启动、能连通并响应 PING/ECHO/SET/GET/DEL/EXISTS，用于发现明显故障。
#
# 用法: ./scripts/test_smoke.sh [host] [port]
# 默认: host=127.0.0.1 port=6379

set -e

HOST="${1:-127.0.0.1}"
PORT="${2:-6379}"
CLI="redis-cli -h $HOST -p $PORT"
AIKV_BINARY="./target/debug/aikv"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

ok() { echo -e "${GREEN}[OK]${NC} $1"; }
fail() { echo -e "${RED}[FAIL]${NC} $1"; exit 1; }

# 确保在项目根目录执行
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$(cd "$SCRIPT_DIR/.." && pwd)"

echo "=============================================="
echo " AiKv 冒烟测试 (host=$HOST port=$PORT)"
echo "=============================================="

# --- 编译 ---
echo -e "\n${YELLOW}[Build]${NC}"
cargo build && ok "Build" || fail "Build"

# --- 启动服务 ---
echo -e "\n${YELLOW}[Server]${NC}"
$AIKV_BINARY -c config/aikv.toml >/dev/null 2>&1 &
AIKV_PID=$!
cleanup() {
    [ -n "$AIKV_PID" ] && kill $AIKV_PID 2>/dev/null || true
    wait $AIKV_PID 2>/dev/null || true
    echo -e "\n${YELLOW}[Cleanup]${NC}"; ok "Server stopped"
}
trap cleanup EXIT INT TERM

wait_count=0
while [ $wait_count -lt 15 ]; do
    $CLI PING 2>/dev/null | grep -q PONG && break
    kill -0 $AIKV_PID 2>/dev/null || fail "Server process exited before ready"
    sleep 1; wait_count=$((wait_count + 1))
done
$CLI PING 2>/dev/null | grep -q PONG || fail "Server failed to accept connections within 15s"
ok "Server started"

# --- 协议与基础命令 ---
echo -e "\n${YELLOW}[协议] PING / ECHO / SET/GET / DEL / EXISTS${NC}"
if ! command -v redis-cli &>/dev/null; then
    echo -e "${YELLOW}redis-cli not found, skip protocol tests${NC}"; ok "Server startup"
else
    r=$($CLI PING 2>/dev/null); [ "$r" = "PONG" ] && ok "PING" || fail "PING (got: $r)"
    r=$($CLI ECHO "Hello AiKv" 2>/dev/null); [ "$r" = "Hello AiKv" ] && ok "ECHO" || fail "ECHO (got: $r)"
    $CLI SET testkey "testvalue" >/dev/null 2>&1 || fail "SET"
    r=$($CLI GET testkey 2>/dev/null); [ "$r" = "testvalue" ] && ok "GET" || fail "GET (got: $r)"
    r=$($CLI DEL testkey 2>/dev/null); [ "$r" = "1" ] || [ "$r" = "(integer) 1" ] && ok "DEL" || fail "DEL (got: $r)"
    $CLI SET existskey "value" >/dev/null 2>&1
    r=$($CLI EXISTS existskey 2>/dev/null); [ "$r" = "1" ] || [ "$r" = "(integer) 1" ] && ok "EXISTS" || fail "EXISTS (got: $r)"
    $CLI DEL existskey >/dev/null 2>&1 || true
fi

trap - EXIT INT TERM
cleanup

echo -e "${GREEN}\n[SUCCESS] 冒烟测试完成\n${NC}"
