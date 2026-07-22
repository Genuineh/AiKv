# @component aikv-server
# @title 错误不崩
"""类型错误后服务仍可用."""

from __future__ import annotations

import pytest
import redis

_PREFIX = "e2e:func:wrongtype:"


# @title WRONGTYPE 后仍可用
def test_wrongtype_then_ping(dut):
    """类型错误不拖死服务.

    1. 将 key SET 为 "plain" (String) | 成功
    2. 对同一 key 发 HGET | ResponseError 含 WRONGTYPE
    3. 客户端再发 PING | 成功 (True)
    """
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
