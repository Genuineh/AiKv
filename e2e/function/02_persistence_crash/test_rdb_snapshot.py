# @component aikv-server
# @title RDB 快照落盘与状态感知功能测试
"""覆盖 SAVE 阻塞式同步快照落盘、BGSAVE 异步后台落盘及 LASTSAVE 时间戳更新校验."""

from __future__ import annotations
import time
import pytest

_PREFIX = "{tag0}:"


# @title SAVE 同步快照与 BGSAVE 异步落盘 (SAVE, BGSAVE, LASTSAVE)
def test_save_and_bgsave_snapshot(svc):
    """SAVE 同步快照与 BGSAVE 异步快照落盘触发及 LASTSAVE 更新.

    1. 执行 LASTSAVE 获取旧快照时间戳 | 返回整数时间戳
    2. 写入测试数据 Key | 写入成功
    3. 执行 SAVE 同步快照落盘 | 返回 OK
    4. 执行 BGSAVE 触发后台异步快照 | 返回 OK 或 Background saving started
    5. 再次查询 LASTSAVE 时间戳 | 时间戳不小于初始时间戳
    """
    c = svc.client()
    k = _PREFIX + "snapshot_k"
    c.set(k, "snapshot_val")

    # LASTSAVE 用 redis-cli 取原始整数 (redis-py 会转 datetime, 首次保存前可能返回 0/None)
    t1 = int(c.cli("LASTSAVE") or 0)
    time.sleep(1)

    assert c.save() is True
    res_bg = c.cli("BGSAVE")
    assert "OK" in str(res_bg) or "Background" in str(res_bg) or "started" in str(res_bg)

    t2 = int(c.cli("LASTSAVE") or 0)
    assert t2 >= t1

    c.delete(k)
