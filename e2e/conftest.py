"""Pytest fixtures for AiKv E2E (release binary + redis-cli)."""

from __future__ import annotations

import sys
from collections.abc import Iterator
from collections.abc import Iterator
from pathlib import Path

import pytest

E2E_ROOT = Path(__file__).resolve().parent
if str(E2E_ROOT) not in sys.path:
    sys.path.insert(0, str(E2E_ROOT))

from lib.redis_cli import require_redis_cli  # noqa: E402
from lib.server import (  # noqa: E402
    DEFAULT_BIN,
    ServerHandle,
    build_release,
    start_memory_server,
)


def pytest_configure(config: pytest.Config) -> None:
    config.addinivalue_line("markers", "slow: real-time waits (seconds to minutes)")
    config.addinivalue_line("markers", "stress: large payloads or high throughput")


@pytest.fixture(scope="session", autouse=True)
def _require_redis_cli() -> None:
    try:
        require_redis_cli()
    except RuntimeError as exc:
        pytest.skip(str(exc))


@pytest.fixture(scope="session")
def aikv_binary() -> Path:
    if DEFAULT_BIN.is_file():
        return DEFAULT_BIN
    return build_release()


@pytest.fixture
def memory_server(aikv_binary: Path) -> Iterator[ServerHandle]:
    server = start_memory_server(aikv_binary)
    try:
        yield ServerHandle(host=server.host, port=server.port)
    finally:
        server.stop()
