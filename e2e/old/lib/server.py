"""Build and lifecycle helpers for ephemeral aikv instances."""

from __future__ import annotations

import os
import random
import subprocess
from dataclasses import dataclass
from pathlib import Path

from lib.redis_cli import wait_ready

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_BIN = REPO_ROOT / "target" / "release" / "aikv"
DEFAULT_HOST = os.environ.get("AIKV_HOST", "127.0.0.1")


def random_port() -> int:
    """Ephemeral client port (mirrors utils.sh PORT assignment)."""
    return 20000 + random.randint(0, 39999)


def build_release(*, manifest: Path | None = None) -> Path:
    """Build release binary with cluster features (mirrors utils.sh build_release)."""
    manifest = manifest or REPO_ROOT / "Cargo.toml"
    subprocess.run(
        ["cargo", "build", "--release", "--features", "cluster", "--manifest-path", str(manifest)],
        cwd=REPO_ROOT,
        check=True,
    )
    return DEFAULT_BIN


@dataclass(frozen=True)
class ServerHandle:
    """Public handle for a running memory-engine instance (used by pytest fixtures)."""

    host: str
    port: int

    def rc(self, *args: str, db: int | None = None) -> str:
        from lib.redis_cli import run

        return run(self.host, self.port, *args, db=db)


@dataclass
class MemoryServer:
    host: str
    port: int
    data_dir: Path
    _proc: subprocess.Popen[bytes] | None = None

    def start(self, binary: Path) -> None:
        self.data_dir.mkdir(parents=True, exist_ok=True)
        self._proc = subprocess.Popen(
            [
                str(binary),
                "--bind",
                f"{self.host}:{self.port}",
                "--engine",
                "memory",
            ],
            cwd=REPO_ROOT,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        wait_ready(self.host, self.port)

    def stop(self) -> None:
        proc = self._proc
        if proc is not None and proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait(timeout=5)
        self._proc = None
        if self.data_dir.exists():
            import shutil

            shutil.rmtree(self.data_dir, ignore_errors=True)


def start_memory_server(
    binary: Path,
    *,
    host: str = DEFAULT_HOST,
    port: int | None = None,
) -> MemoryServer:
    port = port if port is not None else random_port()
    data_dir = REPO_ROOT / "target" / f"e2e-py-{port}-{os.getpid()}"
    server = MemoryServer(host=host, port=port, data_dir=data_dir)
    server.start(binary)
    return server
