# @component aikv-server
# @title Hash 哈希字典功能测试
"""覆盖 Hash 字典增删改查 (HSET/HGET/HDEL), 批量读写 (HMSET/HMGET/HGETALL), 字段存在性与 HSCAN 游标遍历."""

from __future__ import annotations

_PREFIX = "{tag0}:"


# @title Hash 基础增删查与批量获取 (HSET, HGET, HDEL, HMGET, HGETALL, HEXISTS, HLEN, HKEYS)
def test_hash_batch_and_fields(svc):
    """Hash 字典基础增删查与批量获取.

    1. 清理测试 Key h1 | 成功
    2. 使用 HSET 写入 field1="val1", field2="val2" | 返回写入字段数 2
    3. 查询 field1 的值 HGET | 返回 "val1"
    4. 校验 field1 的存在性 HEXISTS | 返回 True
    5. 获取 Hash 字典的总字段数 HLEN | 返回 2
    6. 批量获取 field1, field2 的值 HMGET | 返回 ["val1", "val2"]
    7. 获取全部字段名 HKEYS | 返回集合包含 field1, field2
    8. 获取全部 Field-Value 对 HGETALL | 返回 {"field1": "val1", "field2": "val2"}
    9. 删除 field1 HDEL | 返回删除字段数 1
    10. 清理测试 Key h1 | 成功
    """
    c = svc.client()
    k = _PREFIX + "h1"
    c.delete(k)

    assert c.hset(k, "field1", "val1") == 1
    assert c.hset(k, "field2", "val2") == 1
    assert c.hget(k, "field1") == "val1"

    assert bool(c._r.hexists(k, "field1")) is True
    assert c._r.hlen(k) == 2

    assert c._r.hmget(k, "field1", "field2") == ["val1", "val2"]
    assert set(c._r.hkeys(k)) == {"field1", "field2"}
    assert c._r.hgetall(k) == {"field1": "val1", "field2": "val2"}

    assert c.hdel(k, "field1") == 1
    assert c._r.hlen(k) == 1

    c.delete(k)


# @title Hash 数值运算与 HSCAN 游标遍历 (HINCRBY, HINCRBYFLOAT, HSCAN)
def test_hash_numeric_and_scan(svc):
    """Hash 内部数值自增与 HSCAN 游标遍历.

    1. 清理测试 Key hnum | 成功
    2. 对 Hash 内部 num 字段执行 HINCRBY 自增 5 | 返回 5
    3. 对 Hash 内部 float_num 字段执行 HINCRBYFLOAT 增加 1.5 | 返回 1.5
    4. 使用 HSCAN 游标遍历匹配前缀的字段 | 结果中包含 num 与 float_num
    5. 清理测试 Key hnum | 成功
    """
    c = svc.client()
    k = _PREFIX + "hnum"
    c.delete(k)

    assert c._r.hincrby(k, "num", 5) == 5
    assert float(c._r.hincrbyfloat(k, "float_num", 1.5)) == 1.5

    _cursor, data = c._r.hscan(k, match="*num*", count=100)
    assert "num" in data
    assert "float_num" in data

    c.delete(k)
