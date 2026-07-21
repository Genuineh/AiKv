# @component aikv-server
# @title SET 命令
"""黑盒 SET/GET: 仅连预部署外部 DUT (单机或集群拓扑均可)."""

from __future__ import annotations


def test_set_get(dut):
    """SET 后 GET 应读回相同值."""
    c = dut.client()
    assert c.set("e2e:set:k1", "v1") is True
    assert c.get("e2e:set:k1") == "v1"
