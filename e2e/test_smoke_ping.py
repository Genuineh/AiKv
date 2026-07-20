# @component aikv-server
"""最小冒烟: memory / aidb 节点 PING."""

from __future__ import annotations


def test_ping_memory(memory_node):
    assert memory_node.client().ping()


def test_ping_aidb(aidb_node):
    assert aidb_node.client().ping()
