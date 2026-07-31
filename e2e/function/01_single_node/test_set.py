# @component aikv-server
# @title Set 无序集合功能测试
"""覆盖 Set 集合增删 (SADD/SREM), 成员判定 (SISMEMBER), 随机弹栈 (SPOP), 及多集合交并差集运算 (SINTER/SUNION/SDIFF/SINTERSTORE)."""

from __future__ import annotations
import pytest

_PREFIX = "{tag0}:"


# @title Set 基础增删与随机弹栈 (SADD, SREM, SISMEMBER, SCARD, SPOP)
def test_set_basic_ops(svc):
    """Set 集合元素添加、成员判定与随机弹栈.

    1. 清理测试 Key s1 | 成功
    2. 向集合 s1 写入 "m1", "m2", "m3" SADD | 返回新增成员数 3
    3. 查询集合基数 SCARD | 返回 3
    4. 校验 "m1" 是否为集合成员 SISMEMBER | 返回 True
    5. 校验 "unknown" 是否为集合成员 SISMEMBER | 返回 False
    6. 从集合 s1 中随机 SPOP 弹出 1 个元素 | 返回弹出的元素名称
    7. 校验集合剩余基数 SCARD | 返回 2
    8. 从剩余成员中移除 1 个 SREM | 返回 1
    9. 清理测试 Key s1 | 成功
    """
    c = svc.client()
    k = _PREFIX + "s1"
    c.delete(k)

    assert c.sadd(k, "m1", "m2", "m3") == 3
    assert c._r.scard(k) == 3

    assert bool(c._r.sismember(k, "m1")) is True
    assert bool(c._r.sismember(k, "unknown")) is False

    popped = c.spop(k)
    assert popped in {"m1", "m2", "m3"}
    assert c._r.scard(k) == 2

    # SPOP 弹出是随机的, 从剩余成员中确定性删除一个
    remaining = c.smembers(k)
    assert len(remaining) == 2
    victim = next(iter(remaining))
    assert c.srem(k, victim) == 1
    assert c._r.scard(k) == 1

    c.delete(k)


# @title Set 集合运算与结果存储 (SINTER, SUNION, SDIFF, SINTERSTORE)
def test_set_inter_union_diff_store(svc):
    """Set 多集合交集、并集、差集计算与 SINTERSTORE 结果持久存储.

    1. 清理测试 Key set_a, set_b, set_dest | 成功
    2. 向 set_a 添加 1, 2, 3 | 返回 3
    3. 向 set_b 添加 2, 3, 4 | 返回 3
    4. 计算 set_a 与 set_b 的交集 SINTER | 返回 {"2", "3"}
    5. 计算 set_a 与 set_b 的并集 SUNION | 返回 {"1", "2", "3", "4"}
    6. 计算 set_a 与 set_b 的差集 SDIFF | 返回 {"1"}
    7. 将交集结果存入 set_dest SINTERSTORE | 返回写入存储的基数 2
    8. 校验 set_dest 集合成员 SMEMBERS | 返回 {"2", "3"}
    9. 清理测试 Key set_a, set_b, set_dest | 成功
    """
    c = svc.client()
    ka = _PREFIX + "set_a"
    kb = _PREFIX + "set_b"
    kdest = _PREFIX + "set_dest"
    c.delete(ka, kb, kdest)

    assert c.sadd(ka, "1", "2", "3") == 3
    assert c.sadd(kb, "2", "3", "4") == 3

    assert set(c._r.sinter(ka, kb)) == {"2", "3"}
    assert set(c._r.sunion(ka, kb)) == {"1", "2", "3", "4"}
    assert set(c._r.sdiff(ka, kb)) == {"1"}

    assert c._r.sinterstore(kdest, ka, kb) == 2
    assert set(c.smembers(kdest)) == {"2", "3"}

    c.delete(ka, kb, kdest)
