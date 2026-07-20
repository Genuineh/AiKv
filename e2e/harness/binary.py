"""定位 / 构建 release 版 aikv 二进制."""

from __future__ import annotations

import subprocess
from pathlib import Path

from harness.log import get_logger

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_BIN = REPO_ROOT / "target" / "release" / "aikv"


def ensure_release_binary() -> Path:
    """若 `target/release/aikv` 已存在则直接用, 否则 `cargo build --release --features cluster`."""
    log = get_logger()
    if DEFAULT_BIN.is_file():
        log.debug("使用已有二进制 %s", DEFAULT_BIN)
        return DEFAULT_BIN
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
    if not DEFAULT_BIN.is_file():
        raise FileNotFoundError(
            f"期望二进制位于 {DEFAULT_BIN}; 请检查 CARGO_TARGET_DIR / 构建输出"
        )
    return DEFAULT_BIN
