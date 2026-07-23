# @component aikv-server
# @title 持久与重启
"""客户端写探针; 部署侧重启 / kill 后再跑读回用例.

编排 (人/部署侧操作, 非单条用例内完成):
- 跑 seed 写入
- 优雅停再起 → 设 `AIKV_E2E_AFTER_RESTART=1` 跑再读
- kill 探针写入 → `kill -9` 再起 → 设 `AIKV_E2E_AFTER_KILL=1` 跑再读
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


# @title 优雅重启探针写入
def test_persist_seed_graceful(svc):
    """优雅重启前写入探针 (部署侧重启前跑).

    1. 将探针 key SET 为 "alive-after-restart" | 成功
    2. GET 该探针 key | 返回 "alive-after-restart"
    """
    c = svc.client()
    assert c.set(_KEY_GRACEFUL, _VAL_GRACEFUL) is True
    assert c.get(_KEY_GRACEFUL) == _VAL_GRACEFUL


# @title 优雅重启后再读
@pytest.mark.skipif(
    not _env_truthy("AIKV_E2E_AFTER_RESTART"),
    reason="部署侧优雅重启后再设 AIKV_E2E_AFTER_RESTART=1 跑本用例",
)
def test_persist_after_graceful_restart(svc):
    """优雅重启后再读 (需 AIKV_E2E_AFTER_RESTART=1).

    1. GET 探针 key | 返回 "alive-after-restart"
    """
    c = svc.client()
    assert c.get(_KEY_GRACEFUL) == _VAL_GRACEFUL


# @title kill 探针写入
def test_persist_seed_kill(svc):
    """kill 前写入探针 (部署侧 kill -9 前跑).

    1. 将探针 key SET 为 "alive-after-kill" | 成功
    2. GET 该探针 key | 返回 "alive-after-kill"
    """
    c = svc.client()
    assert c.set(_KEY_KILL, _VAL_KILL) is True
    assert c.get(_KEY_KILL) == _VAL_KILL


# @title kill 后再读
@pytest.mark.skipif(
    not _env_truthy("AIKV_E2E_AFTER_KILL"),
    reason="部署侧 kill -9 恢复后再设 AIKV_E2E_AFTER_KILL=1 跑本用例",
)
def test_persist_after_kill(svc):
    """kill -9 恢复后再读 (需 AIKV_E2E_AFTER_KILL=1).

    1. GET 探针 key | 返回 "alive-after-kill"
    """
    c = svc.client()
    assert c.get(_KEY_KILL) == _VAL_KILL
