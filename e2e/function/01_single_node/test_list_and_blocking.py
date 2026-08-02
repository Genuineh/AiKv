# @component aikv-server
# @title List 列表与阻塞弹栈功能测试
"""覆盖 List 列表双端推拉 (LPUSH/RPUSH, LPOP/RPOP), 切片获取 (LRANGE), 移动元素 (LMOVE) 及阻塞超时 (BLPOP)."""

from __future__ import annotations
import pytest

_PREFIX = "{tag0}:"


# @title List 双端推拉与 LMOVE 跨列表移动 (LPUSH, RPUSH, LPOP, RPOP, LRANGE, LMOVE)
def test_list_ops_and_move(svc):
    """List 双端推拉、范围截取与 LMOVE 跨列表元素移动.

    1. 清理测试 Key list1, list2 | 成功
    2. 从左侧 LPUSH 推入 "a", "b" 到 list1 | 返回列表长度 2
    3. 从右侧 RPUSH 推入 "c" 到 list1 | 返回列表长度 3
    4. 读取 list1 全量切片 LRANGE | 返回 ["b", "a", "c"]
    5. 查询 list1 长度 LLEN | 返回 3
    6. 执行 LMOVE 将 list1 左端弹出推入 list2 右端 | 返回移动的元素 "b"
    7. 读取 list1 切片 | 返回 ["a", "c"]
    8. 读取 list2 切片 | 返回 ["b"]
    9. 从 list1 左端 LPOP 弹出 | 返回 "a"
    10. 从 list1 右端 RPOP 弹出 | 返回 "c"
    11. 清理测试 Key list1, list2 | 成功
    """
    c = svc.client()
    k1 = _PREFIX + "l1"
    k2 = _PREFIX + "l2"
    c.delete(k1, k2)

    assert c._r.lpush(k1, "a", "b") == 2
    assert c._r.rpush(k1, "c") == 3

    assert c.lrange(k1, 0, -1) == ["b", "a", "c"]
    assert c._r.llen(k1) == 3

    assert c._r.lmove(k1, k2, src="LEFT", dest="RIGHT") == "b"
    assert c.lrange(k1, 0, -1) == ["a", "c"]
    assert c.lrange(k2, 0, -1) == ["b"]

    assert c.lpop(k1) == "a"
    assert c._r.rpop(k1) == "c"

    c.delete(k1, k2)


# @title List 阻塞超时弹栈 (BLPOP Timeout)
def test_list_blocking_timeout(svc):
    """BLPOP 在空列表上的超时返回逻辑.

    1. 清理测试 Key empty_list | 成功
    2. 对空列表执行 BLPOP 阻塞 1 秒 | 超时返回 None
    3. 清理测试 Key empty_list | 成功
    """
    c = svc.client()
    k = _PREFIX + "empty_list"
    c.delete(k)

    res = c.blpop(k, timeout=1)
    assert res is None

    c.delete(k)
