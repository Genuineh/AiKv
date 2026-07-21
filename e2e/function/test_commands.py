# @component aikv-server
# @title 基础命令
"""STRING 与 HASH 基础写读删."""

from __future__ import annotations

_PREFIX = "e2e:func:cmd:"


def test_string_set_get_del(dut):
    """STRING: SET / GET / DEL."""
    c = dut.client()
    key = _PREFIX + "str"
    c.delete(key)
    assert c.set(key, "hello") is True
    assert c.get(key) == "hello"
    assert c.delete(key) == 1
    assert c.get(key) is None


def test_hash_hset_hget_hdel(dut):
    """HASH: HSET / HGET / HDEL."""
    c = dut.client()
    key = _PREFIX + "hash"
    c.delete(key)
    assert c.hset(key, "f1", "v1") >= 0
    assert c.hget(key, "f1") == "v1"
    assert c.hdel(key, "f1") == 1
    assert c.hget(key, "f1") is None
    c.delete(key)
