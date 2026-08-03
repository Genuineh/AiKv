# @component aikv-server
# @title Master 宕机与自动切主功能测试
"""强杀 Master 节点进程/容器, 校验集群在 Master 宕机期间维持 cluster_state:ok, 并验证旧 Master 恢复后重新在线."""

from __future__ import annotations

import os
import signal
import time

import pytest
import redis

from harness.docker_control import (
    is_docker_available,
    kill_container,
    start_container,
)

_PREFIX = "{e2e:func:04:failover}:"


# @title Master 宕机期间集群可用性与节点恢复
def test_master_crash_auto_failover(svc):
    """验证 Master 节点强杀后集群状态维持可用, 旧 Master 恢复后重新在线.

    Docker 分支执行容器强杀 (端口 7379 / aikv-4) 并轮询 cluster_state;
    本地进程分支对 svc 进程 SIGKILL 后校验节点列表仍可见 master/myself.

    注意: aikv 的 slot→group 归属不随 Raft 选主变化 (由 MetaRaft 元数据
    管理), 因此本用例不校验 slot 归属转移, 只校验集群状态与节点恢复.

    1. 校验当前被测节点是否处于集群模式 | 若非集群则 Skip
    2. 校验 Docker 控管能力的可用性 | 校验通过
    3. 目标强杀 Shard 2 的 Master 节点容器 (端口 7379 / aikv-4) | 容器被成功强杀
    4. 轮询集群状态 CLUSTER INFO | 集群维持 cluster_state:ok
    5. 重新拉起被打掉的旧 Master 节点并轮询 PING | 旧节点恢复在线
    """
    c = svc.client()
    if not c.cluster:
        pytest.skip("被测服务非集群模式")

    # 分支 1: 本地进程控制模式
    if svc.proc is not None:
        master_pid = svc.proc.pid
        os.kill(master_pid, signal.SIGKILL)
        svc.proc.wait()
        svc.proc = None
        deadline = time.monotonic() + 15
        promoted = False
        while time.monotonic() < deadline:
            try:
                nodes_after = c.cluster_nodes_raw()
                if "master" in nodes_after and "myself" in nodes_after:
                    promoted = True
                    break
            except Exception:  # noqa: BLE001, S110 — Master 被 SIGKILL 期间连接必失败, 轮询重试
                pass
            time.sleep(1)
        assert promoted is True
        return

    # 分支 2: Docker 黑盒容器控制模式
    if not is_docker_available():
        pytest.skip("当前环境缺少 Docker 控制能力，跳过 Master 强杀选主测试")

    # 针对 Shard 2 的 Master 节点 7379 (aikv-4) 进行故障强杀注入
    target_master_port = 7379

    killed = kill_container(target_master_port)
    assert killed is True, f"无法强杀端口 {target_master_port} 的容器"

    try:
        deadline = time.monotonic() + 30
        cluster_ok = False
        while time.monotonic() < deadline:
            try:
                if "cluster_state:ok" in c.cluster_info_raw():
                    cluster_ok = True
                    break
            except Exception:  # noqa: BLE001, S110 — 强杀窗口内 CLUSTER 命令可能瞬断, 轮询重试
                pass
            time.sleep(1)
        assert cluster_ok is True, "强杀 Master 后集群未恢复 cluster_state:ok"
    finally:
        # 恢复拉起故障节点，保障后续测试环境健康
        start_container(target_master_port)
        # 轮询等待旧 Master 容器恢复在线
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
            assert back_online is True, "旧 Master 节点 30s 内未恢复在线"
        finally:
            r_old.close()
            time.sleep(2)
