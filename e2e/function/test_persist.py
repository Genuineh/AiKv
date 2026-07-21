# @component aikv-server
# @title 持久与重启
"""客户端写探针; 部署侧重启 / kill 后再跑读回用例.

编排:
1. 跑 `test_persist_seed_*` (写入)
2. 部署侧优雅停再起 → 设 `WIKV_E2E_AFTER_RESTART=1` 跑 `test_persist_after_graceful_restart`
3. 再跑 kill 探针写入 → `kill -9` 再起 → 设 `WIKV_E2E_AFTER_KILL=1` 跑 `test_persist_after_kill`
"""

from __future__ import annotations

import os

import pytest

_PREFIX = "e2e:func:persist:"
_KEY_GRACEFUL = _PREFIX + "graceful"
_KEY_KILL = _PREFIX + "kill"
_VAL_GRACEFUL = "alive-after-restart"
_VAL_KILL = "alive-after-kill"


def _env_truthy(name: str) -> bool:
    return os.environ.get(name, "").strip().lower() in ("1", "true", "yes")


def test_persist_seed_graceful(dut):
    """写入优雅重启探针键 (重启前跑)."""
    c = dut.client()
    assert c.set(_KEY_GRACEFUL, _VAL_GRACEFUL) is True
    assert c.get(_KEY_GRACEFUL) == _VAL_GRACEFUL


@pytest.mark.skipif(
    not _env_truthy("WIKV_E2E_AFTER_RESTART"),
    reason="部署侧优雅重启后再设 WIKV_E2E_AFTER_RESTART=1 跑本用例",
)
def test_persist_after_graceful_restart(dut):
    """优雅重启后再读探针键仍在."""
    c = dut.client()
    assert c.get(_KEY_GRACEFUL) == _VAL_GRACEFUL


def test_persist_seed_kill(dut):
    """写入非优雅中断探针键 (kill 前跑)."""
    c = dut.client()
    assert c.set(_KEY_KILL, _VAL_KILL) is True
    assert c.get(_KEY_KILL) == _VAL_KILL


@pytest.mark.skipif(
    not _env_truthy("WIKV_E2E_AFTER_KILL"),
    reason="部署侧 kill -9 恢复后再设 WIKV_E2E_AFTER_KILL=1 跑本用例",
)
def test_persist_after_kill(dut):
    """kill -9 恢复后再读探针键仍在."""
    c = dut.client()
    assert c.get(_KEY_KILL) == _VAL_KILL
