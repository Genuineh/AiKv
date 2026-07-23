# @component aikv-server
# @title SET 命令
"""黑盒 SET/GET: 仅连预部署外部 被测服务 (单机或集群拓扑均可)."""

from __future__ import annotations


# @title SET/GET
def test_set_get(svc):
    """SET/GET 样板.

    1. 将 key k1 SET 为 "v1" | 成功
    2. GET key k1 | 返回 "v1"
    """
    c = svc.client()
    assert c.set("e2e:set:k1", "v1") is True
    assert c.get("e2e:set:k1") == "v1"
