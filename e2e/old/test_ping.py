# @component aikv-server
"""Minimal PING smoke — validates memory_server fixture lifecycle."""

from __future__ import annotations

from lib.server import ServerHandle


def test_ping(memory_server: ServerHandle) -> None:
    assert memory_server.rc("PING") == "PONG"
