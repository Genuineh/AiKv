# @component aikv-server
# @title 集群在线切槽与所有权转移功能测试
"""覆盖 CLUSTER SETSLOT <slot> MIGRATING/STABLE 完整在线切槽: 数据自动迁移到目标分片后 STABLE 收尾, 所有权随之后移."""

from __future__ import annotations

import pytest
import redis

_PREFIX = "{tag0}:"
# 集群各节点客户端端口 (与 up-cluster.sh 2 分片 × 2 副本拓扑对应)
_CLIENT_PORTS = (6379, 6380, 6381, 7379, 7380, 7381)
# 目标分片 (不与当前被测分片同组), 使用一个落在源分片内、可迁移的 slot
_TARGET_SLOT = 500


def _exec_setslot(svc, *args):
    """遍历已知端口找到 MetaRaft Leader, 在 Leader 上执行 CLUSTER SETSLOT.

    MetaRaft Leader 是集群单一节点 (通常为 node 1); 对 follower 执行
    SETSLOT 会返回 "不是 Leader" 并附 Leader 信息, 这里直接轮询全部端口,
    不再依赖硬编码地址.
    """
    last_err = None
    for port in _CLIENT_PORTS:
        r = redis.Redis(host=svc.host, port=port, decode_responses=True)
        try:
            res = r.execute_command("CLUSTER SETSLOT", *args)
            return res
        except redis.ResponseError as e:
            last_err = e
            if "不是 Leader" not in str(e):
                raise
        finally:
            r.close()
    raise RuntimeError(f"所有节点都非 MetaRaft Leader, 无法执行 SETSLOT: {last_err}")


def _find_key_in_slot(c, slot: int) -> str:
    """探测一个 CRC16 落入指定 slot 的测试 key."""
    for i in range(200):
        k = f"{_PREFIX}slot{slot}:probe{i}"
        if c.cli("CLUSTER", "KEYSLOT", k) == str(slot):
            return k
    raise RuntimeError(f"无法找到 slot {slot} 的探测 key")


def _find_other_master(svc, slot: int) -> tuple[str, int]:
    """从 CLUSTER NODES 解析不持有指定 slot 的 master (迁移目标)."""
    lines = svc.cluster_nodes_raw().splitlines()
    for line in lines:
        parts = line.split()
        if len(parts) < 9 or "master" not in parts[2]:
            continue
        owns = False
        for tok in parts[8:]:
            if tok.startswith("[") or "->" in tok or "<-" in tok:
                continue
            if "-" in tok:
                start, end = map(int, tok.split("-"))
                if start <= slot <= end:
                    owns = True
                    break
            elif int(tok) == slot:
                owns = True
                break
        if owns:
            continue
        addr = parts[1].split("@", 1)[0]
        host, _, port_s = addr.partition(":")
        return parts[0], int(port_s)
    raise RuntimeError(f"未找到不持有 slot {slot} 的 master")


# @title 在线切槽: SETSLOT MIGRATING 迁移数据后 STABLE 完成所有权转移
def test_slot_migration_and_ask_redirect(svc):
    """验证在线切槽: MIGRATING 自动完成数据拷贝, STABLE 收尾并完成所有权转移.

    1. 校验当前被测节点是否处于集群模式 | 若非集群则 Skip
    2. 探测 slot 500 的测试 Key 并写入 | 写入成功
    3. 从 CLUSTER NODES 解析不持有 slot 500 的目标 Master | 提取成功
    4. 在 MetaRaft Leader 上执行 SETSLOT 500 MIGRATING <目标节点> | 返回 OK (数据已自动迁移)
    5. 在 MetaRaft Leader 上执行 SETSLOT 500 STABLE | 返回 OK (收尾并提交)
    6. 从目标 Master 直连读取该 Key | 返回原值 (数据已落目标分片)
    7. 清理测试 Key | 成功
    """
    c = svc.client()
    if not c.cluster:
        pytest.skip("被测服务非集群模式")

    k = _find_key_in_slot(c, _TARGET_SLOT)
    c.delete(k)
    assert c.set(k, "migrated_val") is True
    assert c.get(k) == "migrated_val"

    target_id, target_port = _find_other_master(svc, _TARGET_SLOT)

    res = _exec_setslot(svc, str(_TARGET_SLOT), "MIGRATING", target_id)
    assert "OK" in str(res), f"MIGRATING 应返回 OK, 实际: {res!r}"

    res_clean = _exec_setslot(svc, str(_TARGET_SLOT), "STABLE")
    assert "OK" in str(res_clean), f"STABLE 应返回 OK, 实际: {res_clean!r}"

    # 数据已拷贝到目标分片且所有权转移: 目标 Master 直连可读
    r_target = redis.Redis(host=svc.host, port=target_port, decode_responses=True)
    try:
        assert r_target.get(k) == "migrated_val"
    finally:
        r_target.close()

    c.delete(k)
