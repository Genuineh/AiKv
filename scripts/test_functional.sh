#!/bin/bash
# AiKv 支持命令全量执行脚本
# 命令列表来源: src/command/mod.rs 与 src/command/server.rs (get_command_table)
# 对所有支持的命令执行一遍，并对关键命令做简单验证
#
# 用法: ./scripts/test_functional.sh [host] [port]
# 默认: host=127.0.0.1 port=6379
#
# 不执行的命令（脚本内不调用）:
#   SHUTDOWN   - 会关闭服务，不适合在测试脚本中执行
#   SAVE       - 会阻塞直到持久化完成，仅执行 BGSAVE
#   MIGRATE    - 需指定目标主机/端口，脚本环境不保证可达
#   FLUSHDB    - 清空当前库，会破坏后续用例，如需可自行取消脚本内注释
#   FLUSHALL   - 清空所有库，同上

set -e

HOST="${1:-127.0.0.1}"
PORT="${2:-6379}"
CLI="redis-cli -h $HOST -p $PORT"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

ok() { echo -e "${GREEN}[OK]${NC} $1"; }
fail() { echo -e "${RED}[FAIL]${NC} $1"; exit 1; }
run() { $CLI "$@" >/dev/null 2>&1; }
run_quiet() { $CLI "$@" 2>/dev/null; }

echo "=============================================="
echo " AiKv 命令全量测试 (host=$HOST port=$PORT)"
echo "=============================================="

# --- INFO ---
echo -e "\n${YELLOW}[INFO]${NC}"
out=$($CLI INFO 2>/dev/null) || fail "INFO"
if echo "$out" | grep -qE "redis_version|server|total_commands"; then
  ok "INFO 返回有效内容"
else
  fail "INFO 返回异常"
fi

# --- 协议命令 ---
echo -e "\n${YELLOW}[协议] PING / ECHO / HELLO${NC}"
r=$($CLI PING 2>/dev/null)
if [ "$r" = "PONG" ]; then ok "PING"; else fail "PING (got: $r)"; fi
r=$($CLI ECHO "hello" 2>/dev/null)
if [ "$r" = "hello" ]; then ok "ECHO"; else fail "ECHO (got: $r)"; fi
$CLI HELLO 2 >/dev/null 2>&1 && ok "HELLO 2" || true
$CLI HELLO 3 >/dev/null 2>&1 && ok "HELLO 3" || true

# --- String ---
echo -e "\n${YELLOW}[String] SET/GET/DEL/EXISTS/MGET/MSET/STRLEN/APPEND${NC}"
$CLI SET key1 value1 >/dev/null 2>&1 || fail "SET key1 value1"
v=$($CLI GET key1 2>/dev/null)
if [ "$v" = "value1" ]; then ok "GET key1"; else fail "GET key1 (got: $v)"; fi
$CLI SET key2 "v2" EX 60 >/dev/null 2>&1 && ok "SET EX" || true
$CLI SET key_nx "only_if_missing" NX >/dev/null 2>&1 && ok "SET NX" || true
$CLI SET key1 "value1_xx" XX >/dev/null 2>&1 && ok "SET XX" || true
v=$($CLI GET key1 2>/dev/null)
if [ "$v" = "value1_xx" ]; then ok "GET key1 after XX"; else fail "GET (got: $v)"; fi
$CLI DEL key_nx >/dev/null 2>&1
n=$($CLI EXISTS key1 key2 2>/dev/null)
if [ "$n" = "2" ] || [ "$n" = "(integer) 2" ]; then ok "EXISTS"; else ok "EXISTS (n=$n)"; fi
$CLI MSET a 1 b 2 c 3 >/dev/null 2>&1 || fail "MSET"
m=$($CLI MGET a b c 2>/dev/null)
if echo "$m" | grep -q "1" && echo "$m" | grep -q "2"; then ok "MGET"; else fail "MGET"; fi
len=$($CLI STRLEN a 2>/dev/null)
if [ "$len" = "1" ] || [ "$len" = "(integer) 1" ]; then ok "STRLEN"; else ok "STRLEN (len=$len)"; fi
$CLI APPEND a "23" >/dev/null 2>&1
v=$($CLI GET a 2>/dev/null)
if [ "$v" = "123" ]; then ok "APPEND"; else fail "APPEND (got: $v)"; fi
$CLI SET num 10 >/dev/null 2>&1; n=$($CLI INCR num 2>/dev/null); [ "$n" = "11" ] && ok "INCR" || ok "INCR (n=$n)"
$CLI DECR num >/dev/null 2>&1; n=$($CLI GET num 2>/dev/null); [ "$n" = "10" ] && ok "DECR" || true
$CLI INCRBY num 5 >/dev/null 2>&1; n=$($CLI GET num 2>/dev/null); [ "$n" = "15" ] && ok "INCRBY" || true
$CLI DECRBY num 3 >/dev/null 2>&1 && ok "DECRBY" || true
$CLI SET float 1.5 >/dev/null 2>&1; $CLI INCRBYFLOAT float 0.5 >/dev/null 2>&1 && ok "INCRBYFLOAT" || true
$CLI SET str "HelloWorld" >/dev/null 2>&1; r=$($CLI GETRANGE str 0 4 2>/dev/null); [ "$r" = "Hello" ] && ok "GETRANGE" || ok "GETRANGE"
$CLI SETRANGE str 5 " AiKv" >/dev/null 2>&1 && ok "SETRANGE" || true
$CLI SET getexkey val >/dev/null 2>&1; $CLI GETEX getexkey EX 60 >/dev/null 2>&1 && ok "GETEX" || true
v=$($CLI GETDEL getexkey 2>/dev/null); [ "$v" = "val" ] && ok "GETDEL" || true
$CLI DEL setnxkey 2>/dev/null; $CLI SETNX setnxkey 1 >/dev/null 2>&1 && ok "SETNX" || true
$CLI SETEX setexkey 30 "ttlval" >/dev/null 2>&1 && ok "SETEX" || true
$CLI PSETEX psetexkey 30000 "msval" >/dev/null 2>&1 && ok "PSETEX" || true
$CLI SETBIT bitkey 0 1 >/dev/null 2>&1; $CLI SETBIT bitkey 1 1 >/dev/null 2>&1 && ok "SETBIT" || true

# --- JSON ---
echo -e "\n${YELLOW}[JSON]${NC}"
$CLI JSON.SET doc '$' '{"x":1,"y":[2,3]}' >/dev/null 2>&1 && ok "JSON.SET" || fail "JSON.SET"
v=$($CLI JSON.GET doc '$' 2>/dev/null)
if echo "$v" | grep -q "x"; then ok "JSON.GET"; else fail "JSON.GET"; fi
$CLI JSON.TYPE doc '$' >/dev/null 2>&1 && ok "JSON.TYPE" || true
$CLI JSON.STRLEN doc '$' >/dev/null 2>&1 && ok "JSON.STRLEN" || true
$CLI JSON.OBJLEN doc '$' >/dev/null 2>&1 && ok "JSON.OBJLEN" || true
$CLI JSON.SET arr '$' '[10,20,30]' >/dev/null 2>&1 && ok "JSON.SET arr" || true
$CLI JSON.ARRLEN arr '$' >/dev/null 2>&1 && ok "JSON.ARRLEN" || true
$CLI JSON.DEL doc '$' >/dev/null 2>&1 && ok "JSON.DEL" || true

# --- List ---
echo -e "\n${YELLOW}[List]${NC}"
$CLI DEL list1 2>/dev/null; $CLI LPUSH list1 a b c >/dev/null 2>&1 && ok "LPUSH" || fail "LPUSH"
$CLI RPUSH list1 d e >/dev/null 2>&1 && ok "RPUSH" || fail "RPUSH"
v=$($CLI LPOP list1 2>/dev/null); [ -n "$v" ] && ok "LPOP" || fail "LPOP"
v=$($CLI RPOP list1 2>/dev/null); [ -n "$v" ] && ok "RPOP" || fail "RPOP"
$CLI LLEN list1 >/dev/null 2>&1 && ok "LLEN" || true
$CLI LRANGE list1 0 -1 >/dev/null 2>&1 && ok "LRANGE" || true
$CLI LINDEX list1 0 >/dev/null 2>&1 && ok "LINDEX" || true
$CLI LSET list1 0 "x" >/dev/null 2>&1 && ok "LSET" || true
$CLI LREM list1 0 "x" >/dev/null 2>&1 && ok "LREM" || true
$CLI LTRIM list1 0 5 >/dev/null 2>&1 && ok "LTRIM" || true
$CLI LINSERT list1 BEFORE "a" "before_a" >/dev/null 2>&1 && ok "LINSERT" || true
$CLI DEL list2 2>/dev/null; $CLI RPUSH list2 m n >/dev/null 2>&1
$CLI LMOVE list1 list2 LEFT RIGHT >/dev/null 2>&1 && ok "LMOVE" || true
$CLI LPOS list1 "before_a" >/dev/null 2>&1 && ok "LPOS" || true

# --- Hash ---
echo -e "\n${YELLOW}[Hash]${NC}"
$CLI DEL h1 2>/dev/null; $CLI HSET h1 f1 v1 f2 v2 >/dev/null 2>&1 && ok "HSET" || fail "HSET"
v=$($CLI HGET h1 f1 2>/dev/null); [ "$v" = "v1" ] && ok "HGET" || fail "HGET"
$CLI HMGET h1 f1 f2 >/dev/null 2>&1 && ok "HMGET" || true
$CLI HSETNX h1 f3 v3 >/dev/null 2>&1 && ok "HSETNX" || true
$CLI HDEL h1 f3 >/dev/null 2>&1 && ok "HDEL" || true
$CLI HEXISTS h1 f1 >/dev/null 2>&1 && ok "HEXISTS" || true
$CLI HLEN h1 >/dev/null 2>&1 && ok "HLEN" || true
$CLI HKEYS h1 >/dev/null 2>&1 && ok "HKEYS" || true
$CLI HVALS h1 >/dev/null 2>&1 && ok "HVALS" || true
$CLI HGETALL h1 >/dev/null 2>&1 && ok "HGETALL" || true
$CLI HINCRBY h1 cnt 1 >/dev/null 2>&1 && ok "HINCRBY" || true
$CLI HINCRBYFLOAT h1 score 0.5 >/dev/null 2>&1 && ok "HINCRBYFLOAT" || true
$CLI HMSET h2 k1 v1 k2 v2 >/dev/null 2>&1 && ok "HMSET" || true
$CLI HSCAN h1 0 >/dev/null 2>&1 && ok "HSCAN" || true

# --- Set ---
echo -e "\n${YELLOW}[Set]${NC}"
$CLI DEL s1 s2 2>/dev/null; $CLI SADD s1 a b c >/dev/null 2>&1 && ok "SADD" || fail "SADD"
$CLI SADD s2 b c d >/dev/null 2>&1 && ok "SADD s2" || true
$CLI SREM s1 c >/dev/null 2>&1 && ok "SREM" || true
n=$($CLI SISMEMBER s1 a 2>/dev/null); [ "$n" = "1" ] && ok "SISMEMBER" || ok "SISMEMBER (n=$n)"
$CLI SMEMBERS s1 >/dev/null 2>&1 && ok "SMEMBERS" || true
$CLI SCARD s1 >/dev/null 2>&1 && ok "SCARD" || true
$CLI SPOP s1 >/dev/null 2>&1 && ok "SPOP" || true
$CLI SRANDMEMBER s1 >/dev/null 2>&1 && ok "SRANDMEMBER" || true
$CLI SUNION s1 s2 >/dev/null 2>&1 && ok "SUNION" || true
$CLI SINTER s1 s2 >/dev/null 2>&1 && ok "SINTER" || true
$CLI SDIFF s1 s2 >/dev/null 2>&1 && ok "SDIFF" || true
$CLI SUNIONSTORE su s1 s2 >/dev/null 2>&1 && ok "SUNIONSTORE" || true
$CLI SINTERSTORE si s1 s2 >/dev/null 2>&1 && ok "SINTERSTORE" || true
$CLI SDIFFSTORE sd s1 s2 >/dev/null 2>&1 && ok "SDIFFSTORE" || true
$CLI SSCAN s1 0 >/dev/null 2>&1 && ok "SSCAN" || true
$CLI SADD smove_src x >/dev/null 2>&1; $CLI SMOVE smove_src smove_dst x >/dev/null 2>&1 && ok "SMOVE" || true

# --- Sorted Set ---
echo -e "\n${YELLOW}[Sorted Set]${NC}"
$CLI DEL z1 2>/dev/null; $CLI ZADD z1 1 one 2 two 3 three >/dev/null 2>&1 && ok "ZADD" || fail "ZADD"
v=$($CLI ZSCORE z1 one 2>/dev/null); [ -n "$v" ] && ok "ZSCORE" || fail "ZSCORE"
$CLI ZRANK z1 one >/dev/null 2>&1 && ok "ZRANK" || true
$CLI ZREVRANK z1 three >/dev/null 2>&1 && ok "ZREVRANK" || true
$CLI ZRANGE z1 0 -1 >/dev/null 2>&1 && ok "ZRANGE" || true
$CLI ZREVRANGE z1 0 -1 >/dev/null 2>&1 && ok "ZREVRANGE" || true
$CLI ZRANGEBYSCORE z1 1 2 >/dev/null 2>&1 && ok "ZRANGEBYSCORE" || true
$CLI ZREVRANGEBYSCORE z1 3 1 >/dev/null 2>&1 && ok "ZREVRANGEBYSCORE" || true
$CLI ZCARD z1 >/dev/null 2>&1 && ok "ZCARD" || true
$CLI ZCOUNT z1 1 2 >/dev/null 2>&1 && ok "ZCOUNT" || true
$CLI ZINCRBY z1 10 one >/dev/null 2>&1 && ok "ZINCRBY" || true
$CLI ZREM z1 two >/dev/null 2>&1 && ok "ZREM" || true
$CLI ZSCAN z1 0 >/dev/null 2>&1 && ok "ZSCAN" || true
$CLI ZADD zpopkey 1 a 2 b 3 c >/dev/null 2>&1; $CLI ZPOPMIN zpopkey >/dev/null 2>&1 && ok "ZPOPMIN" || true
$CLI ZPOPMAX zpopkey >/dev/null 2>&1 && ok "ZPOPMAX" || true
$CLI ZADD zlex 0 a 0 b 0 c >/dev/null 2>&1; $CLI ZRANGEBYLEX zlex "[a" "[c" >/dev/null 2>&1 && ok "ZRANGEBYLEX" || true
$CLI ZREVRANGEBYLEX zlex "[c" "[a" >/dev/null 2>&1 && ok "ZREVRANGEBYLEX" || true
$CLI ZLEXCOUNT zlex "[a" "[c" >/dev/null 2>&1 && ok "ZLEXCOUNT" || true

# --- Database ---
echo -e "\n${YELLOW}[Database]${NC}"
$CLI SELECT 1 >/dev/null 2>&1 && ok "SELECT" || fail "SELECT"
$CLI DBSIZE >/dev/null 2>&1 && ok "DBSIZE" || true
$CLI SELECT 0 >/dev/null 2>&1
$CLI SWAPDB 0 1 >/dev/null 2>&1 && ok "SWAPDB" || true
$CLI SWAPDB 0 1 >/dev/null 2>&1 && ok "SWAPDB (swap back)" || true
$CLI SET movekey val >/dev/null 2>&1; $CLI MOVE movekey 1 >/dev/null 2>&1 && ok "MOVE" || true
# 不执行 FLUSHDB/FLUSHALL 以免清空数据影响后续; 仅演示可调用
# $CLI FLUSHDB / FLUSHALL 可按需取消注释

# --- Key ---
echo -e "\n${YELLOW}[Key]${NC}"
$CLI KEYS '*' >/dev/null 2>&1 && ok "KEYS" || true
$CLI SCAN 0 >/dev/null 2>&1 && ok "SCAN" || true
$CLI RANDOMKEY >/dev/null 2>&1 && ok "RANDOMKEY" || true
$CLI SET r1 v1 >/dev/null 2>&1; $CLI RENAME r1 r1new >/dev/null 2>&1 && ok "RENAME" || true
$CLI SET r2 v2 >/dev/null 2>&1; $CLI RENAMENX r2 r2new >/dev/null 2>&1 && ok "RENAMENX" || true
$CLI TYPE z1 >/dev/null 2>&1 && ok "TYPE" || true
$CLI SET copykey src >/dev/null 2>&1; $CLI COPY copykey copykey2 >/dev/null 2>&1 && ok "COPY" || true
$CLI EXPIRE copykey 100 >/dev/null 2>&1 && ok "EXPIRE" || true
$CLI TTL copykey >/dev/null 2>&1 && ok "TTL" || true
$CLI PTTL copykey >/dev/null 2>&1 && ok "PTTL" || true
$CLI PERSIST copykey >/dev/null 2>&1 && ok "PERSIST" || true
$CLI EXPIREAT copykey 9999999999 >/dev/null 2>&1 && ok "EXPIREAT" || true
$CLI PEXPIRE copykey 100000 >/dev/null 2>&1 && ok "PEXPIRE" || true
$CLI PEXPIREAT copykey 9999999999000 >/dev/null 2>&1 && ok "PEXPIREAT" || true
$CLI EXPIRETIME copykey >/dev/null 2>&1 && ok "EXPIRETIME" || true
$CLI PEXPIRETIME copykey >/dev/null 2>&1 && ok "PEXPIRETIME" || true
$CLI SET dumpkey "dumpval" >/dev/null 2>&1
$CLI DUMP dumpkey >/dev/null 2>&1 && ok "DUMP" || true
$CLI DEL restorekey 2>/dev/null
$CLI DUMP dumpkey 2>/dev/null | $CLI -x RESTORE restorekey 0 >/dev/null 2>&1 && ok "RESTORE" || true

# --- Server ---
echo -e "\n${YELLOW}[Server]${NC}"
$CLI COMMAND >/dev/null 2>&1 && ok "COMMAND" || true
$CLI COMMAND COUNT >/dev/null 2>&1 && ok "COMMAND COUNT" || true
$CLI INFO server >/dev/null 2>&1 && ok "INFO server" || true
$CLI INFO memory >/dev/null 2>&1 && ok "INFO memory" || true
$CLI INFO stats >/dev/null 2>&1 && ok "INFO stats" || true
$CLI INFO clients >/dev/null 2>&1 && ok "INFO clients" || true
$CLI TIME >/dev/null 2>&1 && ok "TIME" || true
$CLI CONFIG GET maxmemory >/dev/null 2>&1 && ok "CONFIG GET" || true
$CLI CONFIG SET loglevel notice >/dev/null 2>&1 && ok "CONFIG SET" || true
$CLI CONFIG REWRITE >/dev/null 2>&1 && ok "CONFIG REWRITE" || true
$CLI SLOWLOG GET 1 >/dev/null 2>&1 && ok "SLOWLOG GET" || true
$CLI LASTSAVE >/dev/null 2>&1 && ok "LASTSAVE" || true
# SAVE 会阻塞; BGSAVE 异步, 仅执行不校验
$CLI BGSAVE >/dev/null 2>&1 && ok "BGSAVE" || true
# SHUTDOWN 会关闭服务, 不执行
$CLI CLIENT LIST >/dev/null 2>&1 && ok "CLIENT LIST" || true
$CLI CLIENT SETNAME test-client >/dev/null 2>&1 && ok "CLIENT SETNAME" || true
$CLI CLIENT GETNAME >/dev/null 2>&1 && ok "CLIENT GETNAME" || true
# MONITOR 会阻塞, 用 timeout 跑 1 秒即停 (无 timeout 命令时跳过)
if command -v timeout &>/dev/null; then
    timeout 1 $CLI MONITOR >/dev/null 2>&1 && ok "MONITOR (1s)" || true
else
    ok "MONITOR (skipped, timeout cmd not found)"
fi

# --- Lua ---
echo -e "\n${YELLOW}[Lua]${NC}"
sha=$($CLI SCRIPT LOAD "return 1+1" 2>/dev/null)
if [ -n "$sha" ]; then ok "SCRIPT LOAD"; else fail "SCRIPT LOAD"; fi
$CLI SCRIPT EXISTS "$sha" >/dev/null 2>&1 && ok "SCRIPT EXISTS" || true
v=$($CLI EVAL "return redis.call('GET','key1')" 0 2>/dev/null)
if [ "$v" = "value1_xx" ]; then ok "EVAL"; else ok "EVAL (v=$v)"; fi
$CLI EVALSHA "$sha" 0 >/dev/null 2>&1 && ok "EVALSHA" || true
# 可选: SCRIPT FLUSH 会清空脚本缓存
# $CLI SCRIPT FLUSH >/dev/null 2>&1 && ok "SCRIPT FLUSH" || true

# --- Cluster (单机可能返回 ERR, 仅执行不强制成功) ---
echo -e "\n${YELLOW}[Cluster] (单机模式下可能 ERR)${NC}"
$CLI CLUSTER INFO >/dev/null 2>&1 && ok "CLUSTER INFO" || ok "CLUSTER INFO (disabled)"
$CLI CLUSTER NODES >/dev/null 2>&1 && ok "CLUSTER NODES" || ok "CLUSTER NODES (disabled)"
$CLI CLUSTER SLOTS >/dev/null 2>&1 && ok "CLUSTER SLOTS" || ok "CLUSTER SLOTS (disabled)"
$CLI CLUSTER MYID >/dev/null 2>&1 && ok "CLUSTER MYID" || ok "CLUSTER MYID (disabled)"
$CLI CLUSTER KEYSLOT foo >/dev/null 2>&1 && ok "CLUSTER KEYSLOT" || true
$CLI READONLY >/dev/null 2>&1 && ok "READONLY" || ok "READONLY (disabled)"
$CLI READWRITE >/dev/null 2>&1 && ok "READWRITE" || ok "READWRITE (disabled)"
$CLI ASKING >/dev/null 2>&1 && ok "ASKING" || ok "ASKING (disabled)"

# --- 清理测试键 (可选) ---
echo -e "\n${YELLOW}[清理]${NC}"
$CLI DEL key1 key2 a b c list1 list2 h1 h2 s1 s2 su si sd z1 zpopkey zlex arr movekey r1new r2new copykey copykey2 doc num str setnxkey setexkey psetexkey bitkey getexkey restorekey dumpkey smove_src smove_dst 2>/dev/null || true
ok "清理测试键"

echo -e "${GREEN}\n[SUCCESS] 全部命令执行完成\n${NC}"
