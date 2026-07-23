"""CLUSTER NODES 解析与 L1 拓扑不变量 (不硬编码实验室拓扑)."""

from __future__ import annotations

from dataclasses import dataclass, field


@dataclass(frozen=True)
class ClusterNode:
    node_id: str
    addr: str  # host:port@busport 或 host:port
    flags: frozenset[str]
    primary_id: str  # "-" for master
    ping_sent: str
    pong_recv: str
    config_epoch: str
    link_state: str
    slot_ranges: tuple[tuple[int, int], ...] = ()

    @property
    def is_myself(self) -> bool:
        return "myself" in self.flags

    @property
    def is_master(self) -> bool:
        return "master" in self.flags

    @property
    def is_slave(self) -> bool:
        return "slave" in self.flags or "replica" in self.flags


def _parse_slot_token(tok: str) -> list[tuple[int, int]]:
    """Parse `0-8191` / `42` / skip migration markers like `[...]`."""
    if tok.startswith("["):
        return []
    if "-" in tok:
        a, b = tok.split("-", 1)
        return [(int(a), int(b))]
    return [(int(tok), int(tok))]


def parse_cluster_nodes(text: str) -> list[ClusterNode]:
    """Parse Redis CLUSTER NODES bulk string into nodes."""
    nodes: list[ClusterNode] = []
    for raw in text.replace("\r\n", "\n").splitlines():
        line = raw.strip()
        if not line:
            continue
        parts = line.split()
        if len(parts) < 8:
            raise ValueError(f"CLUSTER NODES 行字段不足: {line!r}")
        node_id, addr, flags_s, primary_id = parts[0], parts[1], parts[2], parts[3]
        ping_sent, pong_recv, config_epoch, link_state = (
            parts[4],
            parts[5],
            parts[6],
            parts[7],
        )
        flags = frozenset(f for f in flags_s.split(",") if f)
        ranges: list[tuple[int, int]] = []
        for tok in parts[8:]:
            ranges.extend(_parse_slot_token(tok))
        nodes.append(
            ClusterNode(
                node_id=node_id,
                addr=addr,
                flags=flags,
                primary_id=primary_id,
                ping_sent=ping_sent,
                pong_recv=pong_recv,
                config_epoch=config_epoch,
                link_state=link_state,
                slot_ranges=tuple(ranges),
            )
        )
    return nodes


def parse_cluster_info_kv(text: str) -> dict[str, str]:
    kv: dict[str, str] = {}
    for line in text.replace("\r\n", "\n").splitlines():
        if ":" not in line or line.startswith("#"):
            continue
        k, _, v = line.partition(":")
        kv[k] = v
    return kv


def client_port(addr: str) -> int | None:
    """Extract client port from `host:port@bus` / `host:port`."""
    hostport = addr.split("@", 1)[0]
    if ":" not in hostport:
        return None
    try:
        return int(hostport.rsplit(":", 1)[-1])
    except ValueError:
        return None


@dataclass
class SlotCoverage:
    covered: set[int] = field(default_factory=set)
    overlaps: list[tuple[int, str, str]] = field(default_factory=list)

    def add_ranges(self, node_id: str, ranges: tuple[tuple[int, int], ...]) -> None:
        for start, end in ranges:
            for s in range(start, end + 1):
                if s in self.covered:
                    self.overlaps.append((s, node_id, "duplicate"))
                self.covered.add(s)


def assert_cluster_nodes_invariants(
    nodes: list[ClusterNode],
    *,
    info: dict[str, str],
    expect_host: str,
    expect_port: int,
) -> None:
    """L1: structural invariants for a healthy CLUSTER NODES view."""
    assert nodes, "CLUSTER NODES 为空"

    by_id = {n.node_id: n for n in nodes}
    assert len(by_id) == len(nodes), "CLUSTER NODES 存在重复 node id"

    myself = [n for n in nodes if n.is_myself]
    assert len(myself) == 1, f"期望恰好 1 个 myself, 实际 {len(myself)}"
    me = myself[0]
    port = client_port(me.addr)
    assert port == expect_port, f"myself 端口 {port} != 连接端口 {expect_port} ({me.addr})"

    masters = [n for n in nodes if n.is_master]
    slaves = [n for n in nodes if n.is_slave]
    assert masters, "CLUSTER NODES 无 master"
    for n in nodes:
        assert n.is_master or n.is_slave, f"节点缺少 master/slave 标志: {n.node_id} {n.flags}"
        assert n.link_state == "connected", (
            f"节点 {n.node_id} link_state={n.link_state!r}, 期望 connected"
        )

    for sl in slaves:
        assert sl.primary_id != "-", f"slave {sl.node_id} primary_id 为 '-'"
        prim = by_id.get(sl.primary_id)
        assert prim is not None, f"slave {sl.node_id} 指向未知 primary {sl.primary_id}"
        assert prim.is_master, f"slave {sl.node_id} 的 primary {sl.primary_id} 不是 master"
        assert not sl.slot_ranges, f"slave {sl.node_id} 不应携带 slot 范围"

    for m in masters:
        assert m.primary_id == "-", f"master {m.node_id} primary_id 应为 '-'"

    cov = SlotCoverage()
    for m in masters:
        cov.add_ranges(m.node_id, m.slot_ranges)
    assert not cov.overlaps, f"slot 重叠: {cov.overlaps[:5]}"
    missing = [s for s in range(16384) if s not in cov.covered]
    assert not missing, (
        f"slot 未覆盖 {len(missing)} 个 (例: {missing[:8]}); "
        f"已覆盖 {len(cov.covered)}"
    )
    assert len(cov.covered) == 16384

    # Cross-check CLUSTER INFO when present
    if "cluster_known_nodes" in info:
        assert int(info["cluster_known_nodes"]) == len(nodes), (
            f"known_nodes={info['cluster_known_nodes']} != NODES 行数 {len(nodes)}"
        )
    if "cluster_size" in info:
        assert int(info["cluster_size"]) == len(masters), (
            f"cluster_size={info['cluster_size']} != master 数 {len(masters)}"
        )
    if info.get("cluster_slots_assigned"):
        assert int(info["cluster_slots_assigned"]) == 16384
    if info.get("cluster_state"):
        assert info["cluster_state"] == "ok", f"cluster_state={info['cluster_state']!r}"

    _ = expect_host  # reserved for future announce-mode checks
