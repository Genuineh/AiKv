# @component aikv-server
# @title 崩溃恢复 (failpoint)
"""故障注入 → 强制 kill → 重启 → 数据一致性验证.

需要:
- 本地 docker 环境
- `aikv:dev-test-util` 镜像 (由 aifactory/Dockerfile.test-util 构建)
- 设置 `AIKV_E2E_CRASH_RECOVERY=1`

编排 (本用例内完成):
- docker compose up 3 节点集群
- CLUSTER MEET + ADDSLOTS
- 写 100 baseline keys
- ARM ApplyBeforePersist failpoint (once)
- SET crash-key → 触发 panic
- docker kill --signal=KILL 节点 1
- docker compose up -d 重启
- 验证 baseline 数据完整性
"""

from __future__ import annotations

import os
import subprocess
import time
from pathlib import Path

import pytest
from redis import Redis

_E2E = Path(__file__).resolve().parent.parent
_COMPOSE_FILE = _E2E / "docker-compose.crash-test.yaml"
_DEFAULT_IMAGE = "aikv:dev-test-util"
_PREFIX = "e2e:func:crash:"

# ── helpers ──────────────────────────────────────────────────────────


def _env_truthy(name: str) -> bool:
    return os.environ.get(name, "").strip().lower() in ("1", "true", "yes")


def _r(host="127.0.0.1", port=6379, timeout=5) -> Redis:
    return Redis(
        host=host, port=port, socket_connect_timeout=timeout, decode_responses=True
    )


def _wait_ping(port: int, timeout: int = 60) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            if _r(port=port, timeout=2).ping():
                return True
        except Exception:
            pass
        time.sleep(1)
    return False


def _cli(port: int, *args: str) -> str:
    """通过 redis-cli 跑命令并返回输出 (strip)."""
    cmd = ["redis-cli", "-p", str(port)]
    cmd.extend(args)
    out = subprocess.check_output(cmd, timeout=15, text=True)
    return out.strip()


def _dc(*args: str) -> str:
    """docker compose 快捷."""
    cmd = ["docker", "compose", "-f", str(_COMPOSE_FILE)]
    cmd.extend(args)
    out = subprocess.check_output(cmd, timeout=120, text=True)
    return out.strip()


_IMAGE = os.environ.get("AIKV_IMAGE", _DEFAULT_IMAGE)


# ── fixture ───────────────────────────────────────────────────────────


@pytest.fixture(scope="module", autouse=True)
def _ensure_image() -> None:
    """确保 test-util 镜像存在, 否则跳过."""
    ret = subprocess.run(
        ["docker", "image", "inspect", _IMAGE],
        capture_output=True,
        timeout=30,
    )
    if ret.returncode != 0:
        pytest.skip(f"image {_IMAGE} not found; build first via aifactory/Dockerfile.test-util")


@pytest.fixture
def cluster() -> None:
    """启动 3 节点集群, 组建, 清理."""
    # setup
    _dc("down", "--volumes")
    _dc("up", "-d")
    for port in (6379, 6380, 6381):
        assert _wait_ping(port), f"node :{port} did not become ready"
    _cli(6379, "CLUSTER", "MEET", "127.0.0.1", "6380")
    _cli(6379, "CLUSTER", "MEET", "127.0.0.1", "6381")
    time.sleep(3)

    yield

    # teardown
    _dc("down", "--volumes")


# ── tests ─────────────────────────────────────────────────────────────


@pytest.mark.skipif(
    not _env_truthy("AIKV_E2E_CRASH_RECOVERY"),
    reason="需手动设置 AIKV_E2E_CRASH_RECOVERY=1 (需要 docker + test-util 镜像)",
)
# @title 崩溃恢复: ApplyBeforePersist
def test_crash_recovery_apply_before_persist(cluster: None) -> None:
    """ApplyBeforePersist failpoint → kill → 重启 → 数据一致性验证.

    1. 分配所有 slot 到节点 1 | 成功
    2. 写入 100 baseline keys | 全部成功
    3. 读取验证 100 keys | 全部正确
    4. ARM ApplyBeforePersist failpoint (once) | 成功
    5. SET crash-key 触发 panic | 请求超时或失败 (预期)
    6. docker kill --signal=KILL 节点 1 | 容器终止
    7. docker compose up -d 重启节点 1 | 启动成功
    8. 等待节点 1 恢复 | PONG
    9. 读取验证 100 baseline keys | 全部正确
    10. 验证 failpoint 已自动解除 | STATUS 显示 release
    """
    c = _r(port=6379)

    # ADDSLOTS: 分批写入避免单命令过长
    slots = list(range(16384))
    for i in range(0, len(slots), 1000):
        chunk = slots[i : i + 1000]
        _cli(6379, "CLUSTER", "ADDSLOTS", *(str(s) for s in chunk))
    time.sleep(2)

    # 写入 baseline
    for i in range(1, 101):
        assert c.set(f"{_PREFIX}baseline-{i}", f"value-{i}") is True

    # 验证 baseline
    for i in range(1, 101):
        val = c.get(f"{_PREFIX}baseline-{i}")
        assert val == f"value-{i}", (
            f"baseline key-{i}: expected value-{i}, got {val}"
        )

    # 武装 failpoint
    out = _cli(6379, "CLUSTER", "FAILPOINT", "ARM", "ApplyBeforePersist", "once")
    assert "armed" in out, f"arm failpoint failed: {out}"

    # 触发 panic — SET 可能返回超时或连接断开
    try:
        c.set(f"{_PREFIX}crash-trigger", "x", socket_timeout=5)
    except Exception:
        pass  # 预期

    # 强制 kill
    _dc("kill", "--signal=KILL", "node-1")

    # 等待容器退出
    time.sleep(3)

    # 重启
    _dc("up", "-d", "node-1")
    assert _wait_ping(6379, timeout=90), "node 1 did not recover"
    time.sleep(3)

    # 验证数据
    failures = []
    for i in range(1, 101):
        val = c.get(f"{_PREFIX}baseline-{i}")
        if val != f"value-{i}":
            failures.append(f"key-{i}: expected value-{i}, got {val}")
    assert not failures, (
        f"data consistency check failed: {len(failures)}/100 keys mismatch\n"
        + "\n".join(failures[:5])
    )

    # 验证 failpoint 已解除 (once mode)
    status = _cli(6379, "CLUSTER", "FAILPOINT", "STATUS")
    assert "ApplyBeforePersist: arm" not in status, (
        "failpoint should have been auto-disarmed after once trigger"
    )
