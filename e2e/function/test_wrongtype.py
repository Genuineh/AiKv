# @component aikv-server
# @title 错误不崩
"""类型错误后服务仍可用."""

from __future__ import annotations

import pytest
import redis

_PREFIX = "e2e:func:wrongtype:"


def test_wrongtype_then_ping(dut):
    """对 STRING key 发 HASH 命令应报 WRONGTYPE, 之后仍能 PING."""
    c = dut.client()
    key = _PREFIX + "k"
    c.delete(key)
    assert c.set(key, "plain") is True
    with pytest.raises(redis.ResponseError) as ei:
        c.execute("HGET", key, "f")
    msg = str(ei.value).upper()
    assert "WRONGTYPE" in msg or "WRONG TYPE" in msg
    assert c.ping() is True
    c.delete(key)
