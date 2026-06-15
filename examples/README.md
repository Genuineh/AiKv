# AiKv 示例

| 示例 | 文件 | 说明 | 运行 |
|------|------|------|------|
| 基本 CRUD | `basic.rs` | PING/SET/GET/HSET/INCR/DEL/INFO/QUIT 等常用命令 | `cargo run --example basic` |
| 集群路由 | `cluster.rs` | CRC16 槽位计算 / hash tag 提取 | `cargo run --features cluster --example cluster` |
