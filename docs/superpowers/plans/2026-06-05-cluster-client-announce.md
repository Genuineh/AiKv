# Cluster Client Announce Implementation Plan

> **Status:** Completed 2026-06-05

**Goal:** `unknown` endpoint 模式使 WSL2/Windows GUI 集群客户端无需猜 IP 即可连上全部分片.

**Architecture:** `AnnounceResolver` 输出层插在 Router `client_addr` 与 RESP 协议之间; `CLUSTER NODES` 不经 resolver.

**Delivered:** `announce.rs`, `state.rs`, `commands.rs`, `router.rs`, `main.rs`, tests, `e2e/test_cluster_announce.sh`.
