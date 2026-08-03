"""节点句柄与外部被测服务 连接.

e2e 用例只应使用 `connect_external` (黑盒). `start_node` 留作本机调试工具,
不由 pytest fixture 暴露, 也不参与门禁验收.
"""

from __future__ import annotations

import subprocess
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

from harness.binary import REPO_ROOT
from harness.client import RedisClient
from harness.log import get_logger
from harness.process import (
    cleanup_dir,
    data_dir_for,
    random_port,
    stop_process,
    wait_redis_ping,
)

Engine = Literal["memory", "aidb", "external"]


@dataclass
class Node:
    """运行中或仅连接的节点句柄."""

    host: str
    port: int
    engine: Engine
    data_dir: Path | None
    proc: subprocess.Popen[bytes] | None

    def client(self, db: int = 0) -> RedisClient:
        return RedisClient(self.host, self.port, db=db)

    @property
    def is_cluster(self) -> bool:
        """判断目标节点是否运行在集群模式下."""
        try:
            res = self.client().execute("CLUSTER", "INFO")
            return "cluster_state:" in str(res)
        except Exception:
            return False

    def tag_key(self, name: str, tag: str = "test") -> str:
        """生成带 Hash Tag 的 Key, 确保集群模式下计算至相同 Slot."""
        return f"{{{tag}}}{name}"

    def flush_all(self) -> None:
        """清空数据 (自动兼容单机与集群)."""
        try:
            self.client().execute("FLUSHALL")
        except Exception:
            pass

    def stop(self, *, keep_data: bool = False) -> None:
        """停止进程; 默认清理 data-dir (`keep_data=True` 时保留)."""
        log = get_logger()
        if self.proc is not None:
            log.info("停止节点 %s:%s engine=%s", self.host, self.port, self.engine)
            stop_process(self.proc)
            self.proc = None
        if not keep_data:
            cleanup_dir(self.data_dir)
            self.data_dir = None


def connect_external(host: str, port: int) -> Node:
    """连接已启动的外部被测服务, 不起本机进程."""
    log = get_logger()
    log.info("连接外部被测服务 %s:%s", host, port)
    wait_redis_ping(host, port)
    return Node(host=host, port=port, engine="external", data_dir=None, proc=None)


def start_node(
    *,
    binary: Path,
    engine: Literal["memory", "aidb"],
    host: str = "127.0.0.1",
    port: int | None = None,
) -> Node:
    """启动本机单节点并等待 PING 就绪."""
    log = get_logger()
    port = port if port is not None else random_port()
    data_dir = data_dir_for(engine, port)
    data_dir.mkdir(parents=True, exist_ok=True)

    argv = [
        str(binary),
        "--bind",
        f"{host}:{port}",
        "--engine",
        engine,
    ]
    if engine == "aidb":
        argv.extend(["--data-dir", str(data_dir)])

    log.info("启动节点 %s:%s engine=%s bin=%s", host, port, engine, binary)
    t0 = time.monotonic()
    proc = subprocess.Popen(
        argv,
        cwd=REPO_ROOT,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        wait_redis_ping(host, port)
    except Exception:
        stop_process(proc)
        cleanup_dir(data_dir)
        raise
    elapsed_ms = int((time.monotonic() - t0) * 1000)
    log.info("节点就绪 %s:%s engine=%s 耗时 %sms", host, port, engine, elapsed_ms)
    return Node(host=host, port=port, engine=engine, data_dir=data_dir, proc=proc)
