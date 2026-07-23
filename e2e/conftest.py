"""aikv 端到端 pytest fixtures — 仅黑盒被测服务 (不部署、不选引擎)."""

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
def svc() -> Iterator[Node]:
    """预部署被测服务 (黑盒). 地址来自 `AIKV_HOST` / `AIKV_PORT` (test-ui 环境条自动注入)."""
    host = os.environ.get("AIKV_HOST", "").strip()
    port_s = os.environ.get("AIKV_PORT", "").strip()
    if not host or not port_s:
        pytest.fail(
            "黑盒 e2e 需要被测服务地址: 设置 AIKV_HOST 与 AIKV_PORT "
            "(在 test-ui 中使用顶部环境条即可)"
        )
    node = connect_external(host, int(port_s))
    yield node
