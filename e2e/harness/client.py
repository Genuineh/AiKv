"""Redis 客户端 (`redis` 包) 与 `redis-cli` 逃生口."""

from __future__ import annotations

import shutil
import subprocess
from typing import Any

from harness.log import get_logger


def require_redis_cli() -> None:
    """确认 PATH 上有 `redis-cli`, 否则抛错."""
    if shutil.which("redis-cli") is None:
        raise RuntimeError("端到端测试需要 PATH 上存在 redis-cli")


class RedisClient:
    """基于 redis-py 的薄封装 (`decode_responses=True`)."""

    def __init__(self, host: str, port: int, db: int = 0) -> None:
        import redis

        self.host = host
        self.port = port
        self._r = redis.Redis(host=host, port=port, db=db, decode_responses=True)

    def ping(self) -> bool:
        return bool(self._r.ping())

    def execute(self, *args: str) -> Any:
        return self._r.execute_command(*args)

    def set(self, key: str, value: str) -> bool:
        return bool(self._r.set(key, value))

    def get(self, key: str) -> str | None:
        return self._r.get(key)

    def select(self, db: int) -> None:
        self._r.execute_command("SELECT", db)
        self._r.connection_pool.connection_kwargs["db"] = db

    def close(self) -> None:
        self._r.close()


def cli(host: str, port: int, *args: str, db: int | None = None) -> str:
    """调用 `redis-cli` 并返回 stdout (去空白); 非零退出码时抛错."""
    require_redis_cli()
    cmd: list[str] = ["redis-cli", "-h", host, "-p", str(port)]
    if db is not None:
        cmd.extend(["-n", str(db)])
    cmd.extend(args)
    result = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if result.returncode != 0:
        stderr = result.stderr.strip()
        get_logger().error("redis-cli 失败: %s (%s)", args, stderr)
        raise RuntimeError(
            f"redis-cli {args!r} 失败 (exit {result.returncode}): {stderr}"
        )
    return result.stdout.strip()
