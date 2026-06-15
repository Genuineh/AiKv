# AiKv E2E Tests

Shell-based smoke tests against a running `aikv` release binary via `redis-cli`.

## Prerequisites

- `redis-cli` on PATH (Redis tools package)
- Rust toolchain for `cargo build --release`

## Run

```bash
cd AiKv
chmod +x e2e/*.sh
./e2e/test_basic.sh
./e2e/test_datatypes.sh
./e2e/test_ext.sh
./e2e/test_json.sh
```

Environment overrides:

- `WIKV_HOST` (default `127.0.0.1`)
- `WIKV_PORT` (default `6399`)

## Notes

- Tests build `target/release/aikv` and start an ephemeral **memory** engine instance (`--engine memory`).
- **AiDb 重启持久化** 由 L1 覆盖: `cargo test --test storage test_aidb` (roundtrip + restart + adapter list/flushdb). 可选手动:

  ```bash
  DATA=/tmp/aikv-e2e
  cargo run --release -- --bind 127.0.0.1:6380 --engine aidb --data-dir "$DATA"
  redis-cli -p 6380 SET k v
  # 重启同命令后 GET k
  ```

- 本目录为 **shell-only** (无 `runner.rs`); 直接执行上述脚本.
- CI images without `redis-cli` should install `redis-tools` or skip these scripts.
