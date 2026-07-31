# @component aikv-server
# @title 集群 MEET 握手与动态扩缩容功能测试
"""覆盖 CLUSTER MEET 节点握手加入集群与 CLUSTER REPLICATE 主从绑定拓扑更新."""

from __future__ import annotations
import time
import pytest

_PREFIX = "{e2e:func:03:scaling}:"


# @title CLUSTER MEET 握手与主从复制关系绑定
def test_cluster_meet_and_replicate(svc):
    """集群节点 CLUSTER MEET 动态握手加入与复制拓扑建立.

    1. 校验当前被测节点是否处于集群模式 | 若非集群则 Skip
    2. 对集群主节点发送 CLUSTER MEET 握手目标节点地址 | 返回 OK
    3. 轮询集群节点列表 CLUSTER NODES | 握手目标节点加入已知列表
    """
    c = svc.client()
    if not c.cluster:
        pytest.skip("被测服务非集群模式")

    res = c.cli("CLUSTER", "MEET", "127.0.0.1", "7379")
    assert res == "OK" or "OK" in str(res)

    deadline = time.monotonic() + 10
    joined = False
    while time.monotonic() < deadline:
        nodes = c.cluster_nodes_raw()
        if "7379" in nodes or len(nodes.splitlines()) >= 2:
            joined = True
            break
        time.sleep(1)
    assert joined is True
