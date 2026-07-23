# @component aikv-server
# @title 结构化类型
"""Hash / List / Set / ZSet 浅 CRUD (不含 Stream / Array / JSON 等)."""

from __future__ import annotations

_PREFIX = "e2e:func:type:"


# @title Hash CRUD
def test_hash_crud(svc):
    """Hash 浅 CRUD.

    1. 将 hash key 的 field f1 HSET 为 "v1" | 成功
    2. HGET 该 key 的 f1 | 返回 "v1"
    3. 将同一 field f1 HSET 为 "v2" | 成功
    4. HGET 该 key 的 f1 | 返回 "v2"
    5. HDEL 该 key 的 f1 | 删除数 1
    6. HGET 该 key 的 f1 | 缺 field (None)
    """
    c = svc.client()
    key = _PREFIX + "hash"
    c.delete(key)

    assert c.hset(key, "f1", "v1") >= 0
    assert c.hget(key, "f1") == "v1"

    assert c.hset(key, "f1", "v2") >= 0
    assert c.hget(key, "f1") == "v2"

    assert c.hdel(key, "f1") == 1
    assert c.hget(key, "f1") is None
    c.delete(key)


# @title List CRUD
def test_list_crud(svc):
    """List 浅 CRUD.

    1. 向 list key LPUSH "a" | 长度 >= 1
    2. LRANGE 该 key | 返回 ["a"]
    3. 再向该 key LPUSH "b" | 长度 >= 2
    4. LRANGE 该 key | 返回 ["b", "a"]
    5. LLEN 该 key | 返回 2
    6. DEL 该 key | 删除数 1
    7. LRANGE 该 key | 返回 []
    """
    c = svc.client()
    key = _PREFIX + "list"
    c.delete(key)

    assert c.lpush(key, "a") >= 1
    assert c.lrange(key, 0, -1) == ["a"]

    assert c.lpush(key, "b") >= 2
    assert c.lrange(key, 0, -1) == ["b", "a"]
    assert c.llen(key) == 2

    assert c.delete(key) == 1
    assert c.lrange(key, 0, -1) == []


# @title Set CRUD
def test_set_crud(svc):
    """Set 浅 CRUD.

    1. 向 set key SADD 成员 "m1" | 新增 1
    2. SMEMBERS 该 key | 返回 {"m1"}
    3. 再 SADD 成员 "m2" | 新增 1
    4. SMEMBERS 该 key | 返回 {"m1", "m2"}
    5. 从该 key SREM 成员 "m1" | 删除数 1
    6. SMEMBERS 该 key | 返回 {"m2"}
    """
    c = svc.client()
    key = _PREFIX + "set"
    c.delete(key)

    assert c.sadd(key, "m1") == 1
    assert c.smembers(key) == {"m1"}

    assert c.sadd(key, "m2") == 1
    assert c.smembers(key) == {"m1", "m2"}

    assert c.srem(key, "m1") == 1
    assert c.smembers(key) == {"m2"}
    c.delete(key)


# @title ZSet CRUD
def test_zset_crud(svc):
    """ZSet 浅 CRUD.

    1. 向 zset key ZADD 成员 a=1、b=2 | 新增 2
    2. ZRANGE 该 key | 返回 ["a", "b"] (按分升序)
    3. 将同一成员 a 的分 ZADD 为 3 | 成功
    4. ZRANGE 该 key | 返回 ["b", "a"]
    5. 从该 key ZREM 成员 "b" | 删除数 1
    6. ZRANGE 该 key | 返回 ["a"]
    """
    c = svc.client()
    key = _PREFIX + "zset"
    c.delete(key)

    assert c.zadd(key, {"a": 1.0, "b": 2.0}) == 2
    assert c.zrange(key, 0, -1) == ["a", "b"]

    assert c.zadd(key, {"a": 3.0}) >= 0
    assert c.zrange(key, 0, -1) == ["b", "a"]

    assert c.zrem(key, "b") == 1
    assert c.zrange(key, 0, -1) == ["a"]
    c.delete(key)
