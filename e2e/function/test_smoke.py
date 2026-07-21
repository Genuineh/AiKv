# @component aikv-server
# @title 冒烟
"""已部署实例可达."""

from __future__ import annotations


def test_ping(dut):
    """PING 应返回成功."""
    assert dut.client().ping() is True
