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


def detect_cluster_mode(host: str, port: int) -> bool:
    """探测目标是否为集群模式 (`CLUSTER INFO` 成功则视为集群)."""
    import redis

    r = redis.Redis(
        host=host, port=port, decode_responses=True, socket_connect_timeout=1.0
    )
    try:
        r.execute_command("CLUSTER", "INFO")
        return True
    except redis.ResponseError as exc:
        msg = str(exc).lower()
        if "disabled" in msg or "not support" in msg or "unknown command" in msg:
            return False
        return False
    except redis.RedisError:
        return False
    finally:
        r.close()


class RedisClient:
    """基于 redis-py 的薄封装 (`decode_responses=True`).

    `cluster=None` 时自动探测; 集群下使用 `RedisCluster` 以跟随 MOVED.
    """

    def __init__(
        self, host: str, port: int, db: int = 0, *, cluster: bool | None = None
    ) -> None:
        import redis

        self.host = host
        self.port = port
        self.cluster = detect_cluster_mode(host, port) if cluster is None else cluster
        if self.cluster:
            from redis.cluster import RedisCluster

            self._r = RedisCluster(
                host=host,
                port=port,
                decode_responses=True,
                socket_connect_timeout=2.0,
            )
        else:
            self._r = redis.Redis(
                host=host, port=port, db=db, decode_responses=True
            )

    def ping(self) -> bool:
        return bool(self._r.ping())

    def execute(self, *args: str) -> Any:
        return self._r.execute_command(*args)

    def set(self, key: str, value: str) -> bool:
        return bool(self._r.set(key, value))

    def get(self, key: str) -> str | None:
        return self._r.get(key)

    def delete(self, *keys: str) -> int:
        return int(self._r.delete(*keys))

    def hset(self, key: str, field: str, value: str) -> int:
        return int(self._r.hset(key, field, value))

    def hget(self, key: str, field: str) -> str | None:
        return self._r.hget(key, field)

    def hdel(self, key: str, *fields: str) -> int:
        return int(self._r.hdel(key, *fields))

    def lpush(self, key: str, *values: str) -> int:
        return int(self._r.lpush(key, *values))

    def lrange(self, key: str, start: int, end: int) -> list[str]:
        return list(self._r.lrange(key, start, end))

    def llen(self, key: str) -> int:
        return int(self._r.llen(key))

    def sadd(self, key: str, *members: str) -> int:
        return int(self._r.sadd(key, *members))

    def smembers(self, key: str) -> set[str]:
        return set(self._r.smembers(key))

    def srem(self, key: str, *members: str) -> int:
        return int(self._r.srem(key, *members))

    def zadd(self, key: str, mapping: dict[str, float]) -> int:
        return int(self._r.zadd(key, mapping))

    def zrange(self, key: str, start: int, end: int) -> list[str]:
        return list(self._r.zrange(key, start, end))

    def zrem(self, key: str, *members: str) -> int:
        return int(self._r.zrem(key, *members))

    def info(self, section: str | None = None) -> dict[str, Any]:
        if section is None:
            return dict(self._r.info())
        return dict(self._r.info(section))

    def select(self, db: int) -> None:
        if self.cluster:
            raise RuntimeError("集群模式下不支持 SELECT")
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
