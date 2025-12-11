<!-- Copilot / AI Agent instructions for contributors working on AiKv -->

Purpose
-------
- Short actionable guide for AI agents to be effective in this repo.

Quick orientation (big picture)
-------------------------------
- AiKv is a Redis-protocol-compatible, single-binary Rust server (see `README.md`).
- Core pieces:
  - Protocol & server: `src/main.rs`, `src/server/mod.rs`, `src/server/connection.rs`
  - Command layer: `src/command/*` (one file per command family; dispatch in `src/command/mod.rs`)
  - Storage adapters: `src/storage/{memory_adapter.rs,aidb_adapter.rs}` and unified API in `src/storage/mod.rs`
  - Cluster glue (feature-gated): `src/cluster/*` (enabled via Cargo feature `cluster`)
  - Observability: `src/observability/*` (logging, metrics, tracing)

Where to start for common tasks
-------------------------------
- Add a new Redis command: create/extend a file under `src/command/` (follow family naming, e.g. `string.rs`), implement handlers, add a match arm in `src/command/mod.rs` and write unit tests under `tests/` or module tests alongside the file.
- Change storage behavior: edit or add methods in `src/storage/*` and implement behavior in both `memory_adapter.rs` and `aidb_adapter.rs` if it affects persistence.
- Cluster work: enable feature `cluster` locally (`cargo build --features cluster`) and look at `src/cluster/mod.rs` for integration points (ClusterCommands, ClusterBus, SlotRouter).

Important developer workflows & commands
--------------------------------------
- Build (debug): `cargo build`
- Build (release): `cargo build --release`
- Build with cluster: `cargo build --release --features cluster`
- Run locally: `cargo run` or `./target/release/aikv --config config/aikv.toml`
- Docker: single/cluster images via provided Dockerfile and `docker-compose.cluster.yml` (see `README.md`)
- Tests: `cargo test` (CI uses `cargo test --all-features`; use `--all-features` locally if testing cluster integration)
- Format & lint: `cargo fmt`, `cargo clippy --all-targets --all-features`

Project-specific conventions & patterns
--------------------------------------
- Feature-gated behavior: many cluster APIs are behind `#[cfg(feature = "cluster")]`. Tests and code should use `--features cluster` when exercising those paths.
- Command dispatch: a single match-based dispatcher in `src/command/mod.rs` routes command strings to family handlers. Follow existing naming and error handling (return `Result<RespValue>` and use `AikvError` variants).
- Storage adapter pattern: `StorageEngine` enum in `src/storage/mod.rs` delegates to either memory or AiDb adapter. Prefer adding methods to `StorageEngine` to keep higher layers adapter-agnostic.
- Shared cluster state: connections use `Arc<RwLock<ClusterState>>`; prefer explicit `with_shared_cluster_state` constructors for executors in cluster mode.

Testing notes & CI
------------------
- Integration tests in `tests/` spawn or expect a running server (some are `#[ignore]` until server startup is wired into tests). For full coverage run `cargo test --all-features` (CI does this).
- To run ignored integration tests after starting a local server manually: `cargo test -- --ignored` or run them with features: `cargo test --all-features -- --ignored`.
- End-to-end and cluster tests may require running `./scripts/cluster_init.sh` or `docker-compose -f docker-compose.cluster.yml up -d` prior to exercising cluster flows.

Debugging & observability
-------------------------
- Logging uses `tracing`/`tracing_subscriber`. Configure via `--config`'s `[logging]` section or environment `RUST_LOG`.
- Metrics+monitoring: `src/observability/metrics.rs` exposes `Metrics` used by `Server` and `Connection` objects.

Files to read first (minimal set)
-------------------------------
- `README.md` (high-level and workflows)
- `src/main.rs` (startup, config merging)
- `src/server/mod.rs` and `src/server/connection.rs` (networking + per-connection lifecycle)
- `src/command/mod.rs` and `src/command/*` (how commands are parsed/dispatched)
- `src/storage/mod.rs` and `src/storage/*` (adapter API and persistence contract)
- `src/cluster/mod.rs` (feature notes and responsibilities)

Example micro-tasks for an AI agent
-----------------------------------
- Implement a new `SETNX`-like command: add handler in `src/command/string.rs`, export from `src/command/mod.rs`, add unit tests and an integration test in `tests/`.
- Add a new StorageEngine helper: add method to `StorageEngine` and mirror behavior in both adapters.
- Add a cluster diagnostic CLI: add command logic under `src/command/server.rs` and ensure the `cluster` feature conditional compiles.

Pointers & docs
----------------
- Cluster scripts: `scripts/cluster_init.sh` and `scripts/README.md`
- Config templates: `config/aikv.toml`, `config/aikv-cluster.toml`
- Architecture & deep dive: `docs/ARCHITECTURE_REFACTORING.md`, `docs/AIDB_INTEGRATION.md`

If something is unclear
-----------------------
- Ask for: typical dev environment (OS/toolchain), desired feature flag mode (`cluster` vs non-cluster), and whether you should run integration tests requiring external services.

— End of file —
