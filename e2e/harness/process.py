"""进程辅助: 端口、就绪等待、停止、清理目录."""

from __future__ import annotations

import os
import random
import shutil
import subprocess
import time
from pathlib import Path

from harness.log import get_logger


def random_port() -> int:
    """分配临时客户端端口 (20000–59999)."""
    return 20000 + random.randint(0, 39999)


def wait_redis_ping(
    host: str,
    port: int,
    *,
    attempts: int = 50,
    interval_s: float = 0.1,
) -> None:
    """轮询 `PING` 直至就绪, 超时抛 `TimeoutError`."""
    import redis

    log = get_logger()
    last_err: Exception | None = None
    for i in range(attempts):
        try:
            client = redis.Redis(host=host, port=port, socket_connect_timeout=0.5)
            if client.ping():
                client.close()
                return
            client.close()
        except Exception as exc:  # noqa: BLE001 — 就绪前允许连接失败并重试
            last_err = exc
        time.sleep(interval_s)
        if i and i % 10 == 0:
            log.debug("等待 redis %s:%s (%s/%s)", host, port, i, attempts)
    raise TimeoutError(
        f"{host}:{port} 在 {attempts} 次尝试后仍未就绪"
        + (f": {last_err}" if last_err else "")
    )


def stop_process(proc: subprocess.Popen[bytes], *, timeout_s: float = 5.0) -> None:
    """先 `terminate`, 超时再 `kill`."""
    if proc.poll() is not None:
        return
    proc.terminate()
    try:
        proc.wait(timeout=timeout_s)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=timeout_s)


def cleanup_dir(path: Path | None) -> None:
    """幂等删除目录树."""
    if path is None:
        return
    if path.exists():
        shutil.rmtree(path, ignore_errors=True)


def data_dir_for(engine: str, port: int) -> Path:
    """为本次启动生成 `target/e2e-{engine}-{port}-{pid}/` 路径."""
    from harness.binary import REPO_ROOT

    return REPO_ROOT / "target" / f"e2e-{engine}-{port}-{os.getpid()}"
