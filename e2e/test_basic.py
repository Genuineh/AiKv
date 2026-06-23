# @component aikv-server
"""Basic SET/GET/SELECT smoke (ported from test_basic.sh)."""

from __future__ import annotations

from lib.server import ServerHandle


def test_set_get_select_dbsize(memory_server: ServerHandle) -> None:
    srv = memory_server
    assert srv.rc("SET", "foo", "bar") == "OK"
    assert srv.rc("GET", "foo") == "bar"
    assert srv.rc("SELECT", "1") == "OK"
    assert srv.rc("DBSIZE", db=1) == "0"
    assert srv.rc("DBSIZE", db=0) == "1"
