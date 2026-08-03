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


def _load_env_file() -> None:
    """若环境变量未指定, 尝试从 e2e/.env 加载."""
    env_file = _E2E / ".env"
    if not env_file.is_file():
        return
    for line in env_file.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, val = line.split("=", 1)
        key, val = key.strip(), val.strip().strip("\"'")
        if key and key not in os.environ:
            os.environ[key] = val


@pytest.fixture
def svc() -> Iterator[Node]:
    """预部署被测服务 (黑盒). 地址按优先级依次取: 环境变量 > e2e/.env > 默认值 127.0.0.1:6379."""
    _load_env_file()
    host = os.environ.get("AIKV_HOST", "127.0.0.1").strip() or "127.0.0.1"
    port_s = os.environ.get("AIKV_PORT", "6379").strip() or "6379"
    node = connect_external(host, int(port_s))
    node.flush_all()
    yield node
