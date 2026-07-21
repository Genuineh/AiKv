"""aikv 端到端 pytest fixtures — 仅黑盒 DUT (不部署、不选引擎)."""

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

from harness.client import require_redis_cli  # noqa: E402
from harness.node import Node, connect_external  # noqa: E402


@pytest.fixture(scope="session", autouse=True)
def _require_redis_cli() -> None:
    try:
        require_redis_cli()
    except RuntimeError as exc:
        pytest.exit(str(exc), returncode=1)


@pytest.fixture
def dut() -> Iterator[Node]:
    """预部署 DUT (黑盒). 地址来自 `WIKV_HOST` / `WIKV_PORT` (test-ui 环境条自动注入)."""
    host = os.environ.get("WIKV_HOST", "").strip()
    port_s = os.environ.get("WIKV_PORT", "").strip()
    if not host or not port_s:
        pytest.fail(
            "黑盒 e2e 需要 DUT 地址: 设置 WIKV_HOST 与 WIKV_PORT "
            "(在 test-ui 中使用顶部环境条即可)"
        )
    node = connect_external(host, int(port_s))
    yield node
