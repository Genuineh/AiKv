"""aikv 端到端 pytest fixtures (基于 harness)."""

from __future__ import annotations

import os
import sys
from collections.abc import Iterator
from pathlib import Path

import pytest

_E2E = Path(__file__).resolve().parent
if str(_E2E) not in sys.path:
    sys.path.insert(0, str(_E2E))

# 不收集 e2e/old/ 下的旧资产
collect_ignore_glob = ["old/*"]

from harness.binary import DEFAULT_BIN, ensure_release_binary  # noqa: E402
from harness.client import require_redis_cli  # noqa: E402
from harness.node import Node, connect_external, start_node  # noqa: E402


def _external_dut_enabled() -> bool:
    return os.environ.get("WIKV_EXTERNAL_DUT", "").strip().lower() in (
        "1",
        "true",
        "yes",
    )


@pytest.fixture(scope="session", autouse=True)
def _require_redis_cli() -> None:
    try:
        require_redis_cli()
    except RuntimeError as exc:
        pytest.exit(str(exc), returncode=1)


@pytest.fixture(scope="session")
def aikv_binary() -> Path:
    """本机 release 二进制; 外部 DUT 模式下不强制构建."""
    if _external_dut_enabled():
        return DEFAULT_BIN
    return ensure_release_binary()


@pytest.fixture
def memory_node(aikv_binary: Path) -> Iterator[Node]:
    """本机 memory 节点; `WIKV_EXTERNAL_DUT=1` 时改为连接外部 DUT."""
    if _external_dut_enabled():
        host = os.environ.get("WIKV_HOST", "").strip()
        port_s = os.environ.get("WIKV_PORT", "").strip()
        if not host or not port_s:
            pytest.fail(
                "WIKV_EXTERNAL_DUT=1 需要同时设置 WIKV_HOST 与 WIKV_PORT "
                "(远程 DUT 须已手起)"
            )
        node = connect_external(host, int(port_s))
        yield node
        return

    node = start_node(binary=aikv_binary, engine="memory")
    try:
        yield node
    finally:
        node.stop()


@pytest.fixture
def aidb_node(aikv_binary: Path) -> Iterator[Node]:
    """本机 aidb 引擎节点; 外部 DUT 模式下 skip."""
    if _external_dut_enabled():
        pytest.skip("aidb_node 需要本机 spawn, 外部 DUT 模式下跳过")

    node = start_node(binary=aikv_binary, engine="aidb")
    try:
        yield node
    finally:
        node.stop()
