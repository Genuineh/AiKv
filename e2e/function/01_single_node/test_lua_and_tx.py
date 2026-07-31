# @component aikv-server
# @title Lua 脚本与 MULTI/EXEC 事务功能测试
"""覆盖 Lua 脚本求值 (EVAL, SCRIPT LOAD, EVALSHA), 及 MULTI/EXEC/WATCH 事务乐观锁控制."""

from __future__ import annotations
import pytest
import redis

_PREFIX = "{tag0}:"


# @title Lua 脚本求值与 SCRIPT LOAD 预编译 (EVAL, SCRIPT LOAD, EVALSHA)
def test_lua_eval_and_sha(svc):
    """Lua 脚本动态求值与基于 SHA1 哈希的预编译脚本执行.

    1. 执行 EVAL 脚本返回传入的第一个参数 KEYS[1] | 返回 Key 名称
    2. 使用 SCRIPT LOAD 预编译 Lua 脚本并获取 SHA1 | 返回 40 位 SHA1 字符串
    3. 使用 EVALSHA 传入 SHA1 执行脚本 | 返回与 EVAL 一致的计算结果
    """
    c = svc.client()
    script = "return redis.call('GET', KEYS[1])"
    k = _PREFIX + "lua_k"
    c.set(k, "lua_val")

    assert c._r.eval("return KEYS[1]", 1, k) == k

    sha = c.cli("SCRIPT", "LOAD", script)
    assert len(sha) == 40

    res = c._r.evalsha(sha, 1, k)
    assert res == "lua_val"

    c.delete(k)


# @title MULTI/EXEC 事务与 WATCH 乐观锁 (MULTI, EXEC, WATCH, DISCARD)
def test_transaction_multi_exec_watch(svc):
    """MULTI/EXEC 事务打包执行与 WATCH 乐观锁隔离防并发冲突.

    1. 清理测试 Key k1, k2 | 成功
    2. 开启 MULTI 事务并连送 SET k1 "v1", SET k2 "v2" | 命令成功入队返回 OK
    3. 提交 EXEC 事务 | 批量返回 [OK, OK] 或 [True, True]
    4. 校验 k1, k2 的写入值 | 分别为 "v1", "v2"
    5. 开启 WATCH 监听 k1 | 监听成功
    6. 执行 MULTI 开启新事务块 | 成功
    7. 在事务提交前由另一连接修改 k1 内容 | 成功篡改
    8. 当前事务提交 EXEC | 判定 WATCH 冲突抛出 WatchError (放弃事务块)
    9. 校验 k2 内容未被修改 | 符合事务原子隔离
    10. 清理测试 Key k1, k2 | 成功
    """
    c = svc.client()
    k1 = _PREFIX + "tx1"
    k2 = _PREFIX + "tx2"
    c.delete(k1, k2)

    pipe = c._r.pipeline(transaction=True)
    pipe.set(k1, "v1")
    pipe.set(k2, "v2")
    res = pipe.execute()
    assert res in ([True, True], ["OK", "OK"], [b"OK", b"OK"])
    assert c.get(k1) == "v1"
    assert c.get(k2) == "v2"

    # WATCH & 冲突测试 (使用 Pipeline.watch 进行乐观锁)
    r1 = redis.Redis(host=svc.host, port=svc.port, decode_responses=True)
    r2 = redis.Redis(host=svc.host, port=svc.port, decode_responses=True)

    pipe1 = r1.pipeline(transaction=True)
    pipe1.watch(k1)
    pipe1.multi()
    pipe1.set(k2, "should_fail")

    # 外部连接修改 watched key
    r2.set(k1, "conflict_val")

    # execute 应触发 WatchError 放弃事务
    try:
        pipe1.execute()
        pytest.fail("WATCH 冲突未触发 WatchError 异常")
    except redis.WatchError:
        pass

    assert c.get(k2) == "v2"
    c.delete(k1, k2)
