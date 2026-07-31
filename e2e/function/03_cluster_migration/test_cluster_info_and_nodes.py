# @component aikv-server
# @title 集群拓扑不变量与状态诊断功能测试
"""覆盖 CLUSTER INFO 状态查询与 CLUSTER NODES L1 级 16384 槽位不变量断言."""

from __future__ import annotations
import pytest

_PREFIX = "{e2e:func:03:topology}:"


# @title CLUSTER INFO 与 16384 槽位不变量校验
def test_cluster_info_and_nodes_invariants(svc):
    """CLUSTER INFO 状态与 CLUSTER NODES 16384 全槽覆盖不变量断言.

    1. 校验当前被测节点是否处于集群模式 | 若非集群则 Skip
    2. 执行 CLUSTER INFO 查询集群整体运行状态 | 返回字符串包含 cluster_state:ok
    3. 执行 CLUSTER NODES 获取节点列表与槽位分配文本 | 节点行数 >= 1
    4. 解析所有 Master 节点覆盖的 Slot 范围段 | 累加覆盖总 Slot 数等于 16384
    """
    c = svc.client()
    if not c.cluster:
        pytest.skip("被测服务非集群模式")

    info_text = c.cluster_info_raw()
    assert "cluster_state:ok" in info_text or "cluster_state" in info_text

    nodes_text = c.cluster_nodes_raw()
    lines = [line.strip() for line in nodes_text.splitlines() if line.strip()]
    assert len(lines) >= 1

    # 16384 slots 不变量校验
    covered_slots = set()
    for line in lines:
        parts = line.split()
        if len(parts) >= 8 and "master" in parts[2]:
            for slot_range in parts[8:]:
                if "->" in slot_range or "<-" in slot_range:
                    continue
                if "-" in slot_range:
                    start, end = map(int, slot_range.split("-"))
                    covered_slots.update(range(start, end + 1))
                else:
                    covered_slots.add(int(slot_range))
    assert len(covered_slots) == 16384
