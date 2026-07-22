# @component aikv-server
# @title 数据正确性
"""覆盖写、缺键/缺 field、双连接不串数据."""

from __future__ import annotations

_PREFIX = "e2e:func:correct:"


# @title 覆盖写
def test_string_overwrite(dut):
    """同一 key 覆盖写.

    1. 将 key SET 为 "v1" | 成功
    2. 将同一 key SET 为 "v2" | 成功
    3. GET 该 key | 返回 "v2"
    """
    c = dut.client()
    key = _PREFIX + "ow"
    c.delete(key)
    assert c.set(key, "v1") is True
    assert c.set(key, "v2") is True
    assert c.get(key) == "v2"
    c.delete(key)


# @title 缺键与缺 field
def test_missing_key_and_field(dut):
    """缺键 / 缺 field 读空.

    1. GET 不存在的 key | 返回 None
    2. 将 hash key 的 field exists HSET 为 "1" | 成功
    3. HGET 该 key 上不存在的 field | 返回 None
    """
    c = dut.client()
    assert c.get(_PREFIX + "missing") is None
    key = _PREFIX + "hash-miss"
    c.delete(key)
    c.hset(key, "exists", "1")
    assert c.hget(key, "nope") is None
    c.delete(key)


# @title 双连接隔离
def test_two_clients_isolated_keys(dut):
    """双连接写不同 key, 互不串读.

    1. 连接 A 将 ka SET 为 "from-a" | 成功
    2. 连接 B 将 kb SET 为 "from-b" | 成功
    3. 连接 A/B 分别 GET ka | 均返回 "from-a"
    4. 连接 A/B 分别 GET kb | 均返回 "from-b"
    """
    a = dut.client()
    b = dut.client()
    ka = _PREFIX + "a"
    kb = _PREFIX + "b"
    a.delete(ka, kb)
    b.delete(ka, kb)
    assert a.set(ka, "from-a") is True
    assert b.set(kb, "from-b") is True
    assert a.get(ka) == "from-a"
    assert a.get(kb) == "from-b"
    assert b.get(ka) == "from-a"
    assert b.get(kb) == "from-b"
    a.delete(ka, kb)
