# @component aikv-server
# @title 网络分区: 少数派旧 leader 快速失败
"""网络分区注入测试 — 隔离分片 leader 容器全部网络.

验证本机视角 cluster_state 正确报 fail, 被隔离节点读写返回 CLUSTERDOWN,
多数派侧不受影响且完成选主, 网络恢复后集群自愈并追平最新值.

前置条件: docker 集群 (aifactory/scripts/up-cluster.sh --replicas 2),
每分片 >=3 节点 (否则多数派侧无法选出新 leader, 测试前置预检断言失败).
"""

from __future__ import annotations

import time

import pytest
import redis

from harness.cluster_nodes import parse_cluster_info_kv
from harness.docker_control import (
    connect_network,
    disconnect_network,
    exec_resp,
    exec_resp_raw,
    get_container_name_by_port,
    is_docker_available,
    parse_resp,
    resp_encode,
)

_PREFIX = "{e2e:func:04:partition}:"


def _cluster_state(raw_resp: str) -> str | None:
    """从 exec_resp 原始 RESP 提取 cluster_state (CLUSTER INFO 是 bulk string)."""
    if raw_resp.startswith("-"):
        return None
    text = parse_resp(raw_resp)
    if not isinstance(text, str):
        return None
    return parse_cluster_info_kv(text).get("cluster_state")


def _wait_for(
    pred,
    timeout: float,
    *,
    interval: float = 0.5,
    desc: str = "condition",
) -> None:
    """轮询 `pred()` 直至返回真值; 超时抛 AssertionError (附最后异常)."""
    deadline = time.monotonic() + timeout
    last = None
    last_error = None
    while time.monotonic() < deadline:
        try:
            last = pred()
            if last:
                return
            last_error = None
        except Exception as e:  # noqa: BLE001 — 分区窗口内连接失败属正常, 轮询重试
            last = None
            last_error = e
        time.sleep(interval)
    detail = f", 最后异常: {last_error!r}" if last_error is not None else ""
    raise AssertionError(f"等待 {desc} 超时 ({timeout}s), 最后结果: {last!r}{detail}")


def test_network_partition_fast_fail(svc):
    """隔离少数派旧 leader → fail + CLUSTERDOWN → 恢复自愈."""
    c = svc.client()
    if not c.cluster:
        pytest.skip("被测服务非集群模式")
    if not is_docker_available():
        pytest.skip("当前环境缺少 Docker 控制能力")

    # ── 0. 前置预检: CLUSTER SLOTS 定位一个 >=3 节点的分片 ──
    slots = c.execute("CLUSTER", "SLOTS")
    assert slots, "CLUSTER SLOTS 为空"
    shard = None
    for entry in slots:
        # entry = [start, end, [host, port, nodeid], replica...]
        if len(entry) >= 4:  # master + >=2 replicas → 多数派侧 >=2 节点可选主
            master_port = int(entry[2][1])
            shard_ports = [int(e[1]) for e in entry[2:]]
            shard = (master_port, shard_ports)
            break
    assert shard is not None, (
        "无 >=3 节点的分片 (REPLICAS>=2 是多数派选主前提, 请用 up-cluster.sh --replicas 2)"
    )
    master_port, shard_ports = shard
    assert len(shard_ports) >= 3, f"分片节点数 {len(shard_ports)} < 3, 无法保证多数派选主"
    leader_container = get_container_name_by_port(master_port)
    assert leader_container, f"找不到 master port {master_port} 对应容器"

    # 确定多数派侧可用的端口 (避开被隔离的 master_port).
    majority_port = next(p for p in shard_ports if p != master_port)

    # ── 1. 写入基准数据 (cluster client 自动 MOVED 到所属分片) ──
    k = _PREFIX + "partition_key"
    assert c.set(k, "before-partition"), "基准数据写入失败"

    # ── 2. 注入分区: 断开 leader 容器全部网络 (返回网络快照供恢复) ──
    nets = disconnect_network(leader_container)
    assert nets, f"容器 {leader_container} 未连接任何网络"

    try:
        # ── 3. 被隔离节点 CLUSTER INFO → cluster_state:fail (≤ lease + 1 tick) ──
        _wait_for(
            lambda: _cluster_state(exec_resp(leader_container, "CLUSTER", "INFO", port=master_port))
            == "fail",
            timeout=15,
            interval=0.5,
            desc="被隔离节点 CLUSTER INFO 报 fail",
        )

        # ── 4. 被隔离节点读写 → CLUSTERDOWN (cluster_state:fail 门控) ──
        raw_set = exec_resp(leader_container, "SET", k, "isolated-write", port=master_port)
        assert raw_set.startswith("-CLUSTERDOWN") or "CLUSTERDOWN" in raw_set, (
            f"被隔离节点 SET 应返回 CLUSTERDOWN, got: {raw_set!r}"
        )
        raw_get = exec_resp(leader_container, "GET", k, port=master_port)
        assert raw_get.startswith("-CLUSTERDOWN") or "CLUSTERDOWN" in raw_get, (
            f"被隔离节点 GET 应返回 CLUSTERDOWN, got: {raw_get!r}"
        )

        # ── 5. 多数派侧: cluster_state:ok + 写成功 + 读回新值 ──
        #    显式经非隔离节点连接 (被隔离 leader 可能是默认 svc 连接节点).
        #    用 RedisCluster 自动跟随 MOVED 到多数派侧新 leader.
        r_majority = redis.cluster.RedisCluster(
            host=svc.host, port=majority_port, decode_responses=True, socket_connect_timeout=2.0
        )
        try:
            def majority_ok() -> bool:
                info = str(r_majority.execute_command("CLUSTER", "INFO"))
                return parse_cluster_info_kv(info).get("cluster_state") == "ok"

            _wait_for(majority_ok, timeout=15, interval=0.5, desc="多数派侧 cluster_state:ok")

            # 预检: 该分片已选出新 leader (拓扑更新前 RedisCluster 仍可能路由到
            # 被隔离旧 leader, 导致 SET 落空). 定位含被隔离 master 的分片, 断言
            # 其当前 master 已切换.
            def shard_has_new_leader() -> bool:
                slots = r_majority.execute_command("CLUSTER", "SLOTS")
                for entry in slots:
                    ports = [int(e[1]) for e in entry[2:]]
                    if master_port in ports:
                        return int(entry[2][1]) != master_port
                return False

            _wait_for(shard_has_new_leader, timeout=20, interval=0.5, desc="多数派侧选出新 leader")
            assert r_majority.set(k, "after-failover"), "多数派侧写入失败"
            assert r_majority.get(k) == "after-failover", "多数派侧读回失败"
        finally:
            r_majority.close()
    finally:
        # ── 6. 恢复网络, 轮询节点重新在线并追平最新值 ──
        assert connect_network(leader_container, nets), (
            f"恢复网络失败: 容器 {leader_container} 重连 {nets} 未全部成功"
        )

        _wait_for(
            lambda: bool(exec_resp(leader_container, "PING", port=master_port)),
            timeout=30,
            interval=1,
            desc="被隔离节点 PING 恢复在线",
        )

        # 网络恢复的强验证: 从被隔离节点视角确认该分片 master 已切换 (自身
        # 已降级为 follower) — 拓扑同步必须经 raft 网络重连, 比 PING (断网时
        # 容器 loopback 仍可达, 恒返回 PONG) 更能证明网络真正恢复.
        def shard_leader_switched() -> bool:
            raw = exec_resp(leader_container, "CLUSTER", "SLOTS", port=master_port)
            if raw.startswith("-"):
                return False
            slots = parse_resp(raw)
            if not isinstance(slots, list):
                return False
            for entry in slots:
                ports = [int(e[1]) for e in entry[2:]]
                if master_port in ports:
                    return int(entry[2][1]) != master_port
            return False

        _wait_for(
            shard_leader_switched,
            timeout=30,
            interval=1,
            desc="被隔离节点视角分片 master 已切换",
        )

        def isolated_reads_new_value() -> bool:
            # 旧 leader 重连后已降级为 follower, 直连 GET 会被路由层 MOVED 重定向;
            # 管道化 READONLY + GET 强制本地读, 校验本地数据确已追平多数派最新值.
            # 响应以 READONLY 的 +OK 开头, 跳过第一个 RESP 后解析 GET 的 bulk string.
            payload = resp_encode("READONLY") + resp_encode("GET", k)
            raw = exec_resp_raw(leader_container, payload, port=master_port)
            if not raw.startswith("+OK"):
                return False
            value = parse_resp(raw.split("\r\n", 1)[1])
            return value == "after-failover"

        _wait_for(
            isolated_reads_new_value,
            timeout=30,
            interval=1,
            desc="被隔离节点追平并读回多数派最新值",
        )
        _wait_for(
            lambda: _cluster_state(exec_resp(leader_container, "CLUSTER", "INFO", port=master_port))
            == "ok",
            timeout=15,
            interval=0.5,
            desc="恢复后被隔离节点 cluster_state:ok",
        )
        try:
            c.delete(k)
        except redis.RedisError:  # 恢复期拓扑切换中删除失败不掩盖已通过的断言
            pass
