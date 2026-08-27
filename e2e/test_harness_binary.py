from __future__ import annotations

import pytest

from harness.binary import ensure_release_binary


def test_prefers_executable_env_binary(monkeypatch, tmp_path):
    binary = tmp_path / "aikv"
    binary.write_bytes(b"candidate")
    binary.chmod(0o755)
    monkeypatch.setenv("AIKV_BIN", str(binary))
    assert ensure_release_binary() == binary.resolve()


def test_rejects_missing_env_binary(monkeypatch, tmp_path):
    monkeypatch.setenv("AIKV_BIN", str(tmp_path / "missing"))
    with pytest.raises(FileNotFoundError, match="AIKV_BIN"):
        ensure_release_binary()


def test_rejects_non_executable_env_binary(monkeypatch, tmp_path):
    binary = tmp_path / "aikv"
    binary.write_bytes(b"candidate")
    binary.chmod(0o644)
    monkeypatch.setenv("AIKV_BIN", str(binary))
    with pytest.raises(PermissionError, match="AIKV_BIN"):
        ensure_release_binary()
