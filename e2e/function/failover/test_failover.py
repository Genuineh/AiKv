# @component aikv-server
# @title Master 宕机与自动切主功能测试
"""强杀 Shard 2 Master 容器, 校验故障期间 TCP 可用并验证旧 Master 恢复."""

from __future__ import annotations

import time

import pytest
import redis

from harness.docker_control import (
    is_docker_available,
    kill_container,
    start_container,
)

_PREFIX = "{e2e:func:04:failover}:"


def _key_for_shard2(seed: redis.Redis) -> tuple[str, int]:
    for i in range(2000):
        key = f"{{failover{i}}}:{_PREFIX}availability"
        slot = int(seed.execute_command("CLUSTER", "KEYSLOT", key))
        if 8192 <= slot <= 16383:
            return key, slot
    raise AssertionError("无法找到 Shard 2 测试 key")


def _master_port_for_slot(nodes: str, slot: int) -> int:
    for line in nodes.splitlines():
        parts = line.split()
        if len(parts) < 9 or "master" not in parts[2]:
            continue
        for token in parts[8:]:
            if token.startswith("[") or "->" in token or "<-" in token:
                continue
            if "-" in token:
                start, end = map(int, token.split("-", 1))
                owns = start <= slot <= end
            else:
                owns = int(token) == slot
            if owns:
                address = parts[1].split("@", 1)[0]
                return int(address.rsplit(":", 1)[1])
    raise AssertionError(f"没有 master 承载 slot {slot}")


# @title Master 宕机期间集群可用性与节点恢复
def test_master_crash_auto_failover(svc):
    """验证 Shard 2 Master 强杀期间维持 TCP 可用, 旧 Master 恢复后重新在线.

    注意: aikv 的 slot→group 归属不随 Raft 选主变化 (由 MetaRaft 元数据
    管理), 本用例使用 CLUSTER NODES 校验故障 slot 的承载 master 已切换.

    1. 校验当前被测节点是否处于集群模式 | 若非集群则 Skip
    2. 校验 Docker 控管能力的可用性 | 校验通过
    3. 选取 Shard 2 的 Key 并写入 before_failover | TCP 写入与读取成功
    4. 强杀 Shard 2 的 Master 节点容器 (端口 7379 / aikv-4) | 容器被成功强杀
    5. 轮询 CLUSTER INFO 与 CLUSTER NODES | 新 Master 承载目标 slot
    6. 故障期间写入并读取 during_failover | Smart Client TCP 可用
    7. 重新拉起旧 Master 并轮询 PING | 旧节点恢复在线
    8. 恢复后读取 during_failover 并清理 Key | 数据完整准确
    """
    c = svc.client()
    if not c.cluster:
        pytest.skip("被测服务非集群模式")

    if not is_docker_available():
        pytest.skip("当前环境缺少 Docker 控制能力，跳过 Master 强杀选主测试")

    seed = redis.Redis(host=svc.host, port=6379, decode_responses=True)
    try:
        key, slot = _key_for_shard2(seed)
    finally:
        seed.close()

    cluster_client = c
    assert cluster_client.set(key, "before_failover") is True
    assert cluster_client.get(key) == "before_failover"

    target_master_port = 7379
    old_master = redis.Redis(
        host=svc.host,
        port=target_master_port,
        decode_responses=True,
        socket_connect_timeout=2.0,
    )
    try:
        assert old_master.ping() is True, "强杀前旧 Master TCP 不可用"
    finally:
        old_master.close()

    killed = kill_container(target_master_port)
    assert killed is True, f"无法强杀端口 {target_master_port} 的容器"

    try:
        deadline = time.monotonic() + 30
        cluster_ok = False
        promoted_port = None
        while time.monotonic() < deadline:
            try:
                if "cluster_state:ok" in c.cluster_info_raw():
                    cluster_ok = True
                    promoted_port = _master_port_for_slot(c.cluster_nodes_raw(), slot)
                    if promoted_port in {7380, 7381}:
                        break
            except Exception:  # noqa: BLE001, S110 — 强杀窗口内 CLUSTER 命令可能瞬断, 轮询重试
                pass
            time.sleep(1)
        assert cluster_ok is True, "强杀 Master 后集群未恢复 cluster_state:ok"
        assert promoted_port in {7380, 7381}, (
            f"slot {slot} 未切换到 Shard 2 新 Master: {promoted_port}"
        )

        assert cluster_client.get(key) == "before_failover"
        assert cluster_client.set(key, "during_failover") is True
        assert cluster_client.get(key) == "during_failover"
    finally:
        assert start_container(target_master_port) is True, (
            f"无法恢复端口 {target_master_port} 的容器"
        )

    r_old = redis.Redis(host=svc.host, port=target_master_port, decode_responses=True)
    try:
        deadline = time.monotonic() + 30
        back_online = False
        while time.monotonic() < deadline:
            try:
                if r_old.ping() is True:
                    back_online = True
                    break
            except Exception:  # noqa: BLE001, S110 — 容器重启期间连接必失败, 轮询重试
                pass
            time.sleep(1)
        assert back_online is True, "旧 Master 节点 30s 内未恢复 TCP 在线"
    finally:
        r_old.close()
        time.sleep(2)

    assert cluster_client.get(key) == "during_failover"
    assert cluster_client.delete(key) >= 0
