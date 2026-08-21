# @component aikv-server
# @title 集群在线切槽与所有权转移功能测试
"""覆盖 CLUSTER SETSLOT <slot> MIGRATING/STABLE 完整在线切槽: 数据自动迁移到目标分片后 STABLE 收尾, 所有权随之转移."""

from __future__ import annotations

import time

import pytest
import redis

_CLIENT_PORTS = (6379, 6380, 6381, 7379, 7380, 7381)
# 目标迁移 slot (落在源分片内, 待迁移到其他分片)
_TARGET_SLOT = 500


def _is_metaraft_not_leader(err: Exception) -> bool:
    """识别可换端口重试的 NotLeader / MOVED / ForwardToLeader / CLUSTERDOWN.

    含数据面 source-leader 要求返回的「不是 Leader」与 MOVED 0.
    NotLeader 无地址时 map_propose_error 会变成 CLUSTERDOWN, 也应换端口.
    排除 migration already in progress (那是脏态, 应 CANCEL 而非换端口).
    """
    s = str(err)
    if "migration already" in s:
        return False
    return (
        "不是 Leader" in s
        or "MOVED 0 " in s
        or "has to forward request to" in s
        or "NotLeader" in s
        or "CLUSTERDOWN" in s
    )


def _exec_setslot_on(svc, port: int, *args):
    """在指定端口执行 CLUSTER SETSLOT."""
    r = redis.Redis(host=svc.host, port=port, decode_responses=True)
    try:
        return r.execute_command("CLUSTER SETSLOT", *args)
    finally:
        r.close()


def _exec_setslot(svc, *args):
    """遍历已知端口找到可执行节点, 返回 (result, port).

    MIGRATING 必须在 source group data Leader 上跑完拷贝并落本地 last_run;
    随后 STABLE/CANCEL 须打同一端口, 否则 finish 会报 progress not verified.
    """
    last_err = None
    for port in _CLIENT_PORTS:
        try:
            res = _exec_setslot_on(svc, port, *args)
            return res, port
        except redis.ResponseError as e:
            last_err = e
            if not _is_metaraft_not_leader(e):
                raise
    raise RuntimeError(f"所有节点都无法执行 SETSLOT: {last_err}")


def _clear_stale_migration(svc) -> None:
    """若遗留 slots_migrating, 尝试 CANCEL 清掉半截迁移."""
    c = svc.client()
    info = c.cluster_info_raw()
    if "cluster_slots_migrating:0" in info or "cluster_slots_migrating:" not in info:
        return
    _exec_setslot(svc, str(_TARGET_SLOT), "CANCEL")


def _find_key_in_slot(c, slot: int) -> str:
    """探测一个 CRC16 落入指定 slot 的测试 key.

    通过可变 hash tag (`{ht0}` … `{htN}`) 扫描: 不同 tag 的 keyslot 不同,
    固定前缀 `{tag0}` 会让所有候选 key 恒落同一 slot, 永远无法命中目标 slot.
    """
    for i in range(2000):
        k = f"{{ht{i}}}:slot{slot}:probe0"
        if c.cli("CLUSTER", "KEYSLOT", k) == str(slot):
            return k
    raise RuntimeError(f"无法找到 slot {slot} 的探测 key")


def _slot_owner_id(svc, slot: int) -> str:
    """解析 CLUSTER NODES 中当前持有指定 slot 的 master 节点 id."""
    lines = svc.client().cluster_nodes_raw().splitlines()
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
            return parts[0]
    raise RuntimeError(f"未找到持有 slot {slot} 的 master")


def _find_other_master(svc, slot: int) -> tuple[str, int]:
    """从 CLUSTER NODES 解析不持有指定 slot 的 master (迁移目标)."""
    lines = svc.client().cluster_nodes_raw().splitlines()
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
        _, _, port_s = addr.partition(":")
        return parts[0], int(port_s)
    raise RuntimeError(f"未找到不持有 slot {slot} 的 master")


def _wait_served_on_target(svc, key: str, target_port: int, expected: str, timeout: float = 15.0):
    """STABLE 后目标节点路由表经 LifecycleManager 异步同步, 直连读取可能短暂
    返回 MOVED/None; 轮询直到目标节点直连可读到期望值."""
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        r = redis.Redis(host=svc.host, port=target_port, decode_responses=True)
        try:
            try:
                last = r.get(key)
            except redis.ResponseError as exc:
                last = exc
            if last == expected:
                return
        finally:
            r.close()
        time.sleep(0.5)
    raise AssertionError(
        f"{timeout}s 内目标节点 :{target_port} 直连读 {key!r} 未返回 {expected!r}, 最后: {last!r}"
    )


# @title 在线切槽: SETSLOT MIGRATING 迁移数据后 STABLE 完成所有权转移
def test_slot_migration_and_transfer(svc):
    """验证在线切槽: MIGRATING 自动完成数据拷贝, STABLE 收尾并完成所有权转移.

    1. 校验当前被测节点是否处于集群模式 | 若非集群则 Skip
    2. 若有残留 migrating 槽, 先 CANCEL 清理 | 清不掉则 Skip
    3. 校验 slot 500 初始归属 (干净拓扑, 不在其他 master) | 否则 Skip 提示重建
    4. 探测 slot 500 的测试 Key 并写入 | 写入成功
    5. 从 CLUSTER NODES 解析不持有 slot 500 的目标 Master | 提取成功
    6. 在 source group data Leader 上执行 SETSLOT 500 MIGRATING <目标节点> | 返回 OK (数据已自动迁移)
    7. 在同一端口执行 SETSLOT 500 STABLE | 返回 OK (收尾并提交; finish 校验本机 last_run)
    8. 轮询从目标 Master 直连读取该 Key | 返回原值 (数据已落目标分片)
    9. 清理测试 Key | 成功
    """
    c = svc.client()
    if not c.cluster:
        pytest.skip("被测服务非集群模式")

    try:
        _clear_stale_migration(svc)
    except Exception as e:  # noqa: BLE001
        pytest.skip(f"无法清理残留 migration: {e}")

    info = c.cluster_info_raw()
    if "cluster_slots_migrating:0" not in info and "cluster_slots_migrating:" in info:
        pytest.skip("slots_migrating>0 且 CANCEL 未清干净, 需重建集群")

    # 幂等保护: 迁移改变 slot 所有权, 重跑时初始归属已变, 反向迁移
    # (source group 不在 MetaRaft leader 本地) 会失败; 需要重建集群.
    initial_owner = _slot_owner_id(svc, _TARGET_SLOT)
    expected_owner = _slot_owner_id(svc, 0)  # 初始 slot 0 与 slot 500 同在 group 1
    if initial_owner != expected_owner:
        pytest.skip(
            f"slot {_TARGET_SLOT} 已被迁移到 {initial_owner}, 需重建集群恢复初始拓扑"
        )

    k = _find_key_in_slot(c, _TARGET_SLOT)
    c.delete(k)
    assert c.set(k, "migrated_val") is True
    assert c.get(k) == "migrated_val"

    target_id, target_port = _find_other_master(svc, _TARGET_SLOT)

    res, exec_port = _exec_setslot(svc, str(_TARGET_SLOT), "MIGRATING", target_id)
    assert res is True or "OK" in str(res), f"MIGRATING 应返回 OK, 实际: {res!r}"

    # STABLE 必须打在同一节点: finish_migration 校验本机 last_run.
    res_clean = _exec_setslot_on(svc, exec_port, str(_TARGET_SLOT), "STABLE")
    assert res_clean is True or "OK" in str(res_clean), f"STABLE 应返回 OK, 实际: {res_clean!r}"

    # 数据已拷贝到目标分片且所有权转移: 轮询直连目标 Master 直到可读
    # (LifecycleManager 每秒 tick 刷新路由, STABLE 后存在短暂异步窗口).
    _wait_served_on_target(svc, k, target_port, "migrated_val")

    # 清理: 数据已落目标分片, 直连目标节点删除; 避免集群客户端在
    # 路由同步窗口期对刚迁移 slot 的请求报 migration target unknown.
    r_target = redis.Redis(host=svc.host, port=target_port, decode_responses=True)
    try:
        deadline = time.monotonic() + 10
        deleted = False
        while time.monotonic() < deadline:
            try:
                if r_target.delete(k):
                    deleted = True
                    break
            except redis.ResponseError:
                time.sleep(0.5)
            else:
                break
        assert deleted is True, f"目标节点 :{target_port} 清理测试 Key 失败"
    finally:
        r_target.close()
