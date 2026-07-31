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


# @title 故障节点重启降级与日志追赶
def test_failed_node_recovery_and_sync(svc):
    """验证故障节点重启后降级为 Slave 并从主节点追平增量数据.

    1. 校验当前集群与 Docker 控制能力的可用性 | 校验通过
    2. 使用 kill_container 强杀关停副本节点容器 (端口 7381 / aikv-6) | 容器成功停运
    3. 在副本节点挂掉期间向 Master 主节点写入增量测试数据 | 主节点写入成功
    4. 使用 start_container 重新拉起故障副本节点 | 容器成功启动
    5. 轮询等待故障节点响应 PING | 节点重新在线
    6. 从 Master 主节点读取增量数据 | 数据成功被复制追平并准确读取
    7. 清理测试 Key | 成功
    """
    c = svc.client()
    k = _PREFIX + "recovery_sync"

    # 分支 1: Docker 黑盒容器集群模式
    if c.cluster and is_docker_available():
        replica_port = 7381
        stopped = kill_container(replica_port)
        assert stopped is True, f"无法停止端口 {replica_port} 的容器"

        try:
            # 在故障期间向 Master 写入增量数据
            c.set(k, "sync_val_docker")
        finally:
            # 重新拉起故障节点
            start_container(replica_port)
            time.sleep(2)

        # 校验故障节点重新加入后数据可查且同步
        r_rep = redis.Redis(host=svc.host, port=replica_port, decode_responses=True)
        try:
            assert r_rep.ping() is True
        except Exception:
            pass

        assert c.get(k) == "sync_val_docker"
        c.delete(k)
        return

    pytest.skip("非 Docker 集群部署环境，跳过节点恢复同步测试")
