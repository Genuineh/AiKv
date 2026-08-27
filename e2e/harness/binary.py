"""定位 / 构建 release 版 aikv 二进制."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

from harness.log import get_logger

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_BIN = REPO_ROOT / "target" / "release" / "aikv"


def _validate_binary(path: Path, source: str) -> Path:
    if not path.is_file():
        raise FileNotFoundError(f"{source} 指向的 binary 不存在或不是文件: {path}")
    if not os.access(path, os.X_OK):
        raise PermissionError(f"{source} 指向的 binary 不可执行: {path}")
    return path.resolve()


def ensure_release_binary() -> Path:
    """优先使用 `AIKV_BIN`, 否则确认或构建默认 release binary."""
    log = get_logger()
    env_bin = os.environ.get("AIKV_BIN")
    if env_bin is not None:
        return _validate_binary(Path(env_bin), "AIKV_BIN")

    if DEFAULT_BIN.is_file():
        binary = _validate_binary(DEFAULT_BIN, "默认 release binary")
        log.debug("使用已有二进制 %s", DEFAULT_BIN)
        return binary
    log.info("正在构建 release aikv (含 cluster feature)…")
    subprocess.run(
        [
            "cargo",
            "build",
            "--release",
            "--features",
            "cluster",
            "--manifest-path",
            str(REPO_ROOT / "Cargo.toml"),
        ],
        cwd=REPO_ROOT,
        check=True,
    )
    return _validate_binary(DEFAULT_BIN, "构建后的 release binary")
