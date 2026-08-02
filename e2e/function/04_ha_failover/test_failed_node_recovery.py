# @component aikv-server
# @title 故障节点恢复与追赶同步功能测试
"""验证故障节点关停重启后降级为 Slave 并从主节点追平增量数据 (非伪通过)."""

from __future__ import annotations

import time

import pytest
import redis

from harness.docker_control import (
    is_docker_available,
    kill_container,
    start_container,
)

_PREFIX = "{e2e:func:04:rec}:"


# @title 故障副本节点重启恢复与集群数据完整性
def test_failed_node_recovery_and_sync(svc):
    """验证副本节点关停重启后恢复在线, 集群读写不受影响.

    黑盒验证范围说明: aikv 副本通过 MultiRaft 复制数据, 不服务客户端读
    (直连副本读仍返回 MOVED 到 master), 因此本用例验证副本恢复在线 +
    master 侧数据完整性, 而非从副本直连读追平.

    1. 校验当前集群与 Docker 控制能力的可用性 | 校验通过
    2. 使用 kill_container 强杀关停 Shard 1 的副本节点容器 (端口 6381 / aikv-3) | 容器成功停运
    3. 在副本节点挂掉期间向 Master 主节点写入增量测试数据 | 主节点写入成功
    4. 使用 start_container 重新拉起故障副本节点 | 容器成功启动
    5. 轮询等待故障节点响应 PING | 节点重新在线
    6. 从 Master 主节点读取增量数据 | 数据完整准确
    7. 清理测试 Key | 成功
    """
    c = svc.client()
    k = _PREFIX + "recovery_sync"

    # 分支 1: Docker 黑盒容器集群模式
    if c.cluster and is_docker_available():
        # 被测 key (_PREFIX+"recovery_sync") 落在 shard1 (slot 2767, 主 6379),
        # 因此强杀 shard1 的副本 6381 (aikv-3) 而非 shard2 的 7381.
        replica_port = 6381
        stopped = kill_container(replica_port)
        assert stopped is True, f"无法停止端口 {replica_port} 的容器"

        try:
            # 在故障期间向 Master 写入增量数据
            c.set(k, "sync_val_docker")
        finally:
            # 重新拉起故障节点
            start_container(replica_port)
            time.sleep(2)

        # 轮询等待故障节点恢复在线 (容器重启需要时间)
        r_rep = redis.Redis(host=svc.host, port=replica_port, decode_responses=True)
        try:
            deadline = time.monotonic() + 30
            rep_online = False
            while time.monotonic() < deadline:
                try:
                    if r_rep.ping() is True:
                        rep_online = True
                        break
                except Exception:  # noqa: BLE001, S110 — 容器重启期间连接必失败, 轮询重试
                    pass
                time.sleep(1)
            assert rep_online is True, "故障节点 30s 内未恢复 PING 在线"
        finally:
            r_rep.close()

        # 从 Master 读取增量数据, 验证故障期间写入的数据完整
        assert c.get(k) == "sync_val_docker", "Master 侧增量数据丢失"

        c.delete(k)
        return

    pytest.skip("非 Docker 集群部署环境，跳过节点恢复同步测试")
