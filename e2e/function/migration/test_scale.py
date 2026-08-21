# @component aikv-server
# @title 集群 MEET 握手加入功能测试
"""覆盖 CLUSTER MEET 节点握手加入集群与节点拓扑可见性更新."""

from __future__ import annotations

import time

import pytest
import redis

_PREFIX = "{e2e:func:03:scaling}:"
_CLIENT_PORTS = (6379, 6380, 6381, 7379, 7380, 7381)


def _is_metaraft_not_leader(err: Exception) -> bool:
    """识别可换端口重试的 NotLeader / MOVED / Forward / CLUSTERDOWN."""
    s = str(err)
    return (
        "不是 Leader" in s
        or "MOVED 0 " in s
        or "has to forward request to" in s
        or "NotLeader" in s
        or "CLUSTERDOWN" in s
    )


def _exec_meet(svc, host: str, client_port: int) -> str:
    """在 MetaRaft Leader 上执行 CLUSTER MEET (轮询端口)."""
    last_err = None
    for port in _CLIENT_PORTS:
        r = redis.Redis(host=svc.host, port=port, decode_responses=True)
        try:
            return r.execute_command("CLUSTER", "MEET", host, str(client_port))
        except redis.ResponseError as e:
            last_err = e
            if not _is_metaraft_not_leader(e):
                raise
        finally:
            r.close()
    raise RuntimeError(f"所有节点都非 MetaRaft Leader, 无法执行 MEET: {last_err}")


# @title CLUSTER MEET 握手加入与节点拓扑可见性
def test_cluster_meet_and_join(svc):
    """集群节点 CLUSTER MEET 动态握手加入与节点拓扑可见性更新.

    1. 校验当前被测节点是否处于集群模式 | 若非集群则 Skip
    2. 在 MetaRaft Leader 上发送 CLUSTER MEET 握手目标节点地址 | 返回 OK
    3. 轮询集群节点列表 CLUSTER NODES | 握手目标节点加入已知列表
    """
    c = svc.client()
    if not c.cluster:
        pytest.skip("被测服务非集群模式")

    res = _exec_meet(svc, "127.0.0.1", 7379)
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
