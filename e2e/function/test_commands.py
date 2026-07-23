# @component aikv-server
# @title 基础命令
"""冒烟 PING, String CRUD, INFO Redis 8.8 全字段, 集群浅探测 (非集群自动 skip)."""

from __future__ import annotations

import pytest

_PREFIX = "e2e:func:cmd:"


# @title Smoke
def test_ping(svc):
    """1. 客户端对被测服务发 PING | 成功 (True)"""
    assert svc.client().ping() is True


# @title String
def test_string_crud(svc):
    """String 的增删改查.

    1. 将 key SET 为 "hello" | 成功
    2. GET 该 key | 返回 "hello"
    3. 将同一 key SET 为 "world" | 成功
    4. GET 该 key | 返回 "world"
    5. DEL 该 key | 删除数 1
    6. GET 该 key | 缺键 (None)
    """
    c = svc.client()
    key = _PREFIX + "str"
    c.delete(key)

    assert c.set(key, "hello") is True
    assert c.get(key) == "hello"

    assert c.set(key, "world") is True
    assert c.get(key) == "world"

    assert c.delete(key) == 1
    assert c.get(key) is None


# @title INFO
def test_info(svc):
    """INFO everything 与 Redis 8.8 全字段键名 + 关键格式.

    1. 向被测服务发 INFO everything | 返回非空文本
    2. 对照 tests/fixtures/redis88_info_full_fields.txt | 每个键名均以 key: 行出现
    3. 校验关键字段格式 | redis_compatible_version=8.8 等
    """
    from pathlib import Path

    text = svc.client().info_raw("everything")
    assert text.strip()

    fixture = (
        Path(__file__).resolve().parents[2]
        / "tests"
        / "fixtures"
        / "redis88_info_full_fields.txt"
    )
    fields = [
        line.strip()
        for line in fixture.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.strip().startswith("#")
    ]
    lines = text.replace("\r\n", "\n").splitlines()
    missing = [f for f in fields if not any(line.startswith(f"{f}:") for line in lines)]
    assert not missing, f"INFO everything 缺少字段: {', '.join(missing)}"

    kv = {}
    for line in lines:
        if ":" not in line or line.startswith("#"):
            continue
        k, _, v = line.partition(":")
        kv[k] = v

    assert kv.get("redis_compatible_version") == "8.8"
    assert kv.get("redis_version")
    assert kv.get("arch_bits") in {"32", "64"}
    for key in (
        "uptime_in_seconds",
        "process_id",
        "tcp_port",
        "connected_clients",
        "used_memory",
        "cluster_enabled",
    ):
        assert key in kv and kv[key].lstrip("-").isdigit(), f"{key} 应为整数, 实际={kv.get(key)!r}"
    assert kv.get("used_memory_human")
    assert kv.get("role") in {"master", "slave"}


# @title CLUSTER INFO
def test_cluster_info(svc):
    """CLUSTER INFO 与 Redis 8.8 全字段键名 + 关键格式 (非集群 skip).

    1. 客户端探测被测服务拓扑 | 为集群, 否则 skip
    2. 向被测服务发 CLUSTER INFO | 对照 redis88_cluster_info_fields.txt 键名均出现
    3. 校验关键字段格式 | cluster_state 为 ok/fail 等
    """
    from pathlib import Path

    c = svc.client()
    if not c.cluster:
        pytest.skip("被测服务非集群模式")

    text = c.cluster_info_raw()
    assert text.strip()

    fixture = (
        Path(__file__).resolve().parents[2]
        / "tests"
        / "fixtures"
        / "redis88_cluster_info_fields.txt"
    )
    fields = [
        line.strip()
        for line in fixture.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.strip().startswith("#")
    ]
    lines = text.replace("\r\n", "\n").splitlines()
    missing = [f for f in fields if not any(line.startswith(f"{f}:") for line in lines)]
    assert not missing, f"CLUSTER INFO 缺少字段: {', '.join(missing)}"

    kv = {}
    for line in lines:
        if ":" not in line or line.startswith("#"):
            continue
        k, _, v = line.partition(":")
        kv[k] = v

    assert kv.get("cluster_state") in {"ok", "fail"}
    for key in (
        "cluster_slots_assigned",
        "cluster_slots_ok",
        "cluster_known_nodes",
        "cluster_size",
        "cluster_current_epoch",
        "cluster_my_epoch",
        "cluster_stats_messages_sent",
        "cluster_stats_messages_received",
        "total_cluster_links_buffer_limit_exceeded",
    ):
        assert key in kv and kv[key].lstrip("-").isdigit(), f"{key} 应为整数, 实际={kv.get(key)!r}"


# @title CLUSTER NODES
def test_cluster_nodes(svc):
    """CLUSTER NODES L1 拓扑不变量 (非集群 skip; 不硬编码实验室拓扑).

    1. 客户端探测被测服务拓扑 | 为集群, 否则 skip
    2. 解析 CLUSTER NODES | 格式合法, 恰好 1 个 myself
    3. master/slave 引用一致; slave 不带 slot; master primary 为 -
    4. 全部 master slot 并集覆盖 0–16383 且无重叠
    5. 与 CLUSTER INFO 交叉 | known_nodes / cluster_size / state=ok
    """
    from harness.cluster_nodes import (
        assert_cluster_nodes_invariants,
        parse_cluster_info_kv,
        parse_cluster_nodes,
    )

    c = svc.client()
    if not c.cluster:
        pytest.skip("被测服务非集群模式")

    nodes_text = c.cluster_nodes_raw()
    info_kv = parse_cluster_info_kv(c.cluster_info_raw())
    nodes = parse_cluster_nodes(nodes_text)
    assert_cluster_nodes_invariants(
        nodes,
        info=info_kv,
        expect_host=svc.host,
        expect_port=svc.port,
    )
