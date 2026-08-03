# @component aikv-server
# @title ZSet 有序集合功能测试
"""覆盖 ZSet 有序集合分数值写入 (ZADD), 分数/排名查询 (ZSCORE/ZRANK/ZREVRANK), 正反向切片 (ZRANGE), 及按成员移除 (ZREM)."""

from __future__ import annotations

_PREFIX = "{tag0}:"


# @title ZSet 分数写入、排名查询与切片 (ZADD, ZSCORE, ZRANK, ZREVRANK, ZRANGE)
def test_zset_score_and_rank(svc):
    """ZSet 分数写入、排名与正反向区间切片.

    1. 清理测试 Key z1 | 成功
    2. 写入元素 "alice":100, "bob":80, "charlie":90 ZADD | 返回新增元素数 3
    3. 查询 "alice" 的分数 ZSCORE | 返回 100.0
    4. 查询 "bob" 的升序排名 ZRANK | 返回 0 (最少分)
    5. 查询 "alice" 的降序排名 ZREVRANK | 返回 0 (最高分)
    6. 按分升序获取全量切片 ZRANGE | 返回 ["bob", "charlie", "alice"]
    7. 带分数 WITHSCORES 获取切片 ZRANGE | 返回 [(成员, 分数)] 形式的列表
    8. 清理测试 Key z1 | 成功
    """
    c = svc.client()
    k = _PREFIX + "z1"
    c.delete(k)

    assert c.zadd(k, {"alice": 100, "bob": 80, "charlie": 90}) == 3
    assert c._r.zscore(k, "alice") == 100.0

    assert c._r.zrank(k, "bob") == 0
    assert c._r.zrevrank(k, "alice") == 0

    assert c.zrange(k, 0, -1) == ["bob", "charlie", "alice"]
    assert c._r.zrange(k, 0, -1, withscores=True) == [
        ("bob", 80.0),
        ("charlie", 90.0),
        ("alice", 100.0),
    ]

    c.delete(k)


# @title ZSet 按成员批量删除 (ZREM)
def test_zset_rem_by_member(svc):
    """ZSet 按成员名称批量删除元素.

    1. 清理测试 Key zrem | 成功
    2. 写入 5 个元素 a:1, b:2, c:3, d:4, e:5 ZADD | 返回 5
    3. 移除指定成员 "b" 与 "c" ZREM | 返回移除元素数量 2
    4. 读取剩余切片 ZRANGE | 返回 ["a", "d", "e"]
    5. 移除指定成员 "a" ZREM | 返回移除元素数量 1
    6. 读取剩余切片 ZRANGE | 返回 ["d", "e"]
    7. 清理测试 Key zrem | 成功
    """
    c = svc.client()
    k = _PREFIX + "zrem"
    c.delete(k)

    assert c.zadd(k, {"a": 1, "b": 2, "c": 3, "d": 4, "e": 5}) == 5
    assert c.zrem(k, "b", "c") == 2
    assert c.zrange(k, 0, -1) == ["a", "d", "e"]

    assert c.zrem(k, "a") == 1
    assert c.zrange(k, 0, -1) == ["d", "e"]

    c.delete(k)
