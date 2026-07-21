# @component aikv-server
# @title 数据正确性
"""覆盖写、缺键/缺 field、双连接不串数据."""

from __future__ import annotations

_PREFIX = "e2e:func:correct:"


def test_string_overwrite(dut):
    """同一 key 覆盖写后 GET 为新值."""
    c = dut.client()
    key = _PREFIX + "ow"
    c.delete(key)
    assert c.set(key, "v1") is True
    assert c.set(key, "v2") is True
    assert c.get(key) == "v2"
    c.delete(key)


def test_missing_key_and_field(dut):
    """缺键 GET 为 None; 缺 field HGET 为 None."""
    c = dut.client()
    assert c.get(_PREFIX + "missing") is None
    key = _PREFIX + "hash-miss"
    c.delete(key)
    c.hset(key, "exists", "1")
    assert c.hget(key, "nope") is None
    c.delete(key)


def test_two_clients_isolated_keys(dut):
    """两个连接写不同 key, 互不串读."""
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
