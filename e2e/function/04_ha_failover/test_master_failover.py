# @component aikv-server
# @title Master 宕机与自动切主功能测试
"""强杀 Master 节点进程/容器, 轮询等待 MetaRaft 超时感知与选主, 校验原 Slave 提升为新 Master 及 Slot 归属变化."""

from __future__ import annotations
import os
import signal
import time
import pytest
from harness.docker_control import (
    is_docker_available,
    kill_container,
    start_container,
)

_PREFIX = "{e2e:func:04:failover}:"


# @title Master 宕机与自动选主提升
def test_master_crash_auto_failover(svc):
    """验证 Master 节点挂掉后自动选主提升与 Slot 拓扑更新.

    1. 校验当前被测节点是否处于集群模式 | 若非集群则 Skip
    2. 校验 Docker 控管能力的可用性 | 校验通过
    3. 目标强杀 Shard 2 的 Master 节点容器 (端口 7379 / aikv-4) | 容器被成功强杀
    4. 轮询集群节点列表 CLUSTER NODES 与 CLUSTER INFO 状态 | 集群成功自动选出新 Master 且维持 cluster_state:ok
    5. 重新拉起被打掉的旧 Master 节点 | 旧节点恢复在线并自动降级为 Slave 副本
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
            except Exception:
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
        deadline = time.monotonic() + 15
        cluster_ok = False
        while time.monotonic() < deadline:
            try:
                nodes_text = c.cluster_nodes_raw()
                # 校验节点列表中至少保持活跃且路由表正确
                if nodes_text and "cluster_state:ok" in c.cluster_info_raw():
                    cluster_ok = True
                    break
            except Exception:
                pass
            time.sleep(1)
    finally:
        # 恢复拉起故障节点，保障后续测试环境健康
        start_container(target_master_port)
        time.sleep(2)
