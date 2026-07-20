"""Subprocess wrapper for redis-cli (mirrors e2e/utils.sh rc/rc_node)."""

from __future__ import annotations

import shutil
import subprocess
from typing import Sequence


def require_redis_cli() -> None:
    if shutil.which("redis-cli") is None:
        raise RuntimeError("redis-cli is required for e2e tests")


def run(
    host: str,
    port: int,
    *args: str,
    db: int | None = None,
    check: bool = True,
) -> str:
    """Run redis-cli and return stdout (stripped). Raises on non-zero exit if check=True."""
    cmd: list[str] = ["redis-cli", "-h", host, "-p", str(port)]
    if db is not None:
        cmd.extend(["-n", str(db)])
    cmd.extend(args)
    result = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if check and result.returncode != 0:
        stderr = result.stderr.strip()
        raise RuntimeError(
            f"redis-cli {' '.join(args)!r} failed (exit {result.returncode}): {stderr}"
        )
    return result.stdout.strip()


def ping(host: str, port: int) -> bool:
    try:
        return run(host, port, "PING", check=False).upper() == "PONG"
    except (OSError, RuntimeError):
        return False


def wait_ready(
    host: str,
    port: int,
    *,
    attempts: int = 50,
    interval_s: float = 0.1,
) -> None:
    """Wait until redis-cli PING succeeds (mirrors utils.sh start_server loop)."""
    import time

    for _ in range(attempts):
        if ping(host, port):
            return
        time.sleep(interval_s)
    raise TimeoutError(f"server on {host}:{port} failed to become ready")


def run_node(host: str, port: int, args: Sequence[str]) -> str:
    """Mirrors utils.sh rc_node."""
    return run(host, port, *args)
