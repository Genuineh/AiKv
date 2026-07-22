# @component aikv-server
# @title 基础命令
"""冒烟 PING, String CRUD, INFO 浅查, 集群浅探测 (非集群自动 skip)."""

from __future__ import annotations

import pytest

_PREFIX = "e2e:func:cmd:"


# @title Smoke
def test_ping(dut):
    """1. 客户端对 DUT 发 PING | 成功 (True)"""
    assert dut.client().ping() is True


# @title String
def test_string_crud(dut):
    """String 的增删改查.

    1. 将 key SET 为 "hello" | 成功
    2. GET 该 key | 返回 "hello"
    3. 将同一 key SET 为 "world" | 成功
    4. GET 该 key | 返回 "world"
    5. DEL 该 key | 删除数 1
    6. GET 该 key | 缺键 (None)
    """
    c = dut.client()
    key = _PREFIX + "str"
    c.delete(key)

    assert c.set(key, "hello") is True
    assert c.get(key) == "hello"

    assert c.set(key, "world") is True
    assert c.get(key) == "world"

    assert c.delete(key) == 1
    assert c.get(key) is None


# @title INFO
def test_info(dut):
    """INFO 浅查.

    1. 向 DUT 发 INFO | 返回非空 dict
    2. INFO 结果含字段 redis_version | 存在
    """
    c = dut.client()
    info = c.info()
    assert isinstance(info, dict) and info
    assert "redis_version" in info


# @title CLUSTER INFO
def test_cluster_info(dut):
    """集群浅探测 CLUSTER INFO (非集群 skip).

    1. 客户端探测 DUT 拓扑 | 为集群, 否则 skip
    2. 向 DUT 发 CLUSTER INFO | 响应含 cluster_state
    """
    c = dut.client()
    if not c.cluster:
        pytest.skip("DUT 非集群模式")
    raw = c.execute("CLUSTER", "INFO")
    text = raw if isinstance(raw, str) else str(raw)
    assert "cluster_state" in text.lower() or "cluster_state" in text


# @title CLUSTER NODES
def test_cluster_nodes(dut):
    """集群浅探测 CLUSTER NODES (非集群 skip).

    1. 客户端探测 DUT 拓扑 | 为集群, 否则 skip
    2. 向 DUT 发 CLUSTER NODES | 返回非空节点表
    """
    c = dut.client()
    if not c.cluster:
        pytest.skip("DUT 非集群模式")
    raw = c.execute("CLUSTER", "NODES")
    text = raw if isinstance(raw, str) else str(raw)
    assert text.strip()
