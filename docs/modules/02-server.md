---
name: aikv-server
depends_on:
  - aikv-protocol
description: AiKv Server 运行时与连接管理 — TCP Listener、Connection 读写循环、HELLO 协议协商、ATOM 事务 (MULTI/EXEC/WATCH)、MONITOR 广播与 max_clients 门控. 修改 src/server/{listener,connection,config} 时查阅.
---

# AiKv Server (服务运行时与连接管理)

## 何时读本文

- 修改 `src/server/{listener.rs, connection.rs, config.rs, mod.rs}` 源码;
- 排查 TCP 连接建立/关闭、Pipeline 批读、`HELLO` 协议协商、ATOM 事务原子性、MONITOR 模式、`max_clients` 拒绝与优雅停机;
- **不覆盖**: RESP 帧流式解析与 Limits 校验 → [protocol.md](01-protocol.md);
- **不覆盖**: 命令具体业务分发与 KeyLock 并发锁 → [commands-core.md](04-commands-core.md) / [commands-extended.md](05-commands-extended.md);
- **不覆盖**: 集群 MOVED/ASK 重定向 → [cluster.md](06-cluster.md);
- **不覆盖**: 可观测性组件 (INFO, Slowlog, Latency, OTel 指标) → [observability.md](07-observability.md).

---

## 代码地图

| 文件路径 | 模块核心职责 | 公共接口与核心入口 |
| :--- | :--- | :--- |
| [`src/server/mod.rs`](../../src/server/mod.rs) | Server 模块根; 子模块组织与类型导出 | `Server`, `Connection`, `ServerSharedState` |
| [`src/server/listener.rs`](../../src/server/listener.rs) | TCP Server accept 循环、`max_clients` 并发门控与优雅停机联动 | `Server::run`, `Server::run_with_listener` |
| [`src/server/connection.rs`](../../src/server/connection.rs) | 单连接状态机、Pipeline 读写循环、内联命令、`HELLO` 协商与 ATOM 事务 | `Connection::handle`, `Connection::process_command` |
| [`src/server/config.rs`](../../src/server/config.rs) | 服务器全局共享状态 (`ServerSharedState`) 与每连接配置定义 | `ServerSharedState`, `ConnectionConfig` |

---

## 关键 Invariants (勿破坏规则)

- **每连接独立 Task**: 每个客户端连接通过 `tokio::spawn(Connection::handle)` 派发至独立异步任务处理, 连接级状态 (`current_db`, `parser`, `tx_state`, `cluster_state`) 绝对隔离, 不跨连接共享.
- **Pipeline 紧凑循环**: 单次从 TCP 读取数据后调用 `parser.feed()`, 随后在内层循环中连续调用 `parse() -> process_command() -> write_response()`, 直至返回 `Ok(None)`, 充分压榨 Pipeline 性能.
- **Buffer 溢出断连**: 当接收数据与缓冲区现有数据之和超过 `max_buffer_size` (64 MiB) 时, 直接中断连接循环并关闭 Socket, 不向客户端写错误响应.
- **协议协商与 Null 适配 (`adapt_for_protocol`)**:
  - 内部业务层统一生成 RESP3 AST (如 `RespValue::Null`);
  - 默认状态下 `protocol_negotiated = false`, 此时 `adapt_for_protocol` 自动将 `Null` / `NullArray` 转换为 RESP2 兼容的 `$-1` / `*-1` 线格式;
  - 仅当客户端主动发送 `HELLO 3` 成功协商后, 才输出原生的 RESP3 `_\r\n` Null 格式.
- **内联命令短路**: `PING`, `ECHO`, `HELLO`, `QUIT`, `MONITOR`, `ATOM.*` 等轻量命令直接在 `Connection` 内部处理, 避免经过 `CommandRouter` 与 KeyLock, 降低热路径调度延迟.
- **并发连接门控 (`max_clients`)**:
  - `ServerSharedState` 维护活跃连接数计数器;
  - 当活跃连接数达到 `--max-clients` 上限时, `listener.accept()` 立即主动关闭新连接 (`drop(stream)`), 防止连接耗尽引发系统级崩溃.
- **CommandRouter 延迟初始化**: `ServerSharedState::router()` 通过 `OnceLock` 在首次需要时惰性构建 `CommandRouter`.

---

## 连接生命周期与 Pipeline 架构

```mermaid
sequenceDiagram
    participant Client as TCP Client
    participant Listener as TcpListener (Server)
    participant State as ServerSharedState
    participant Conn as Connection Task
    participant Parser as RespParser
    participant Router as CommandRouter

    Listener->>State: try_register_connection()
    alt 超过 max_clients
        State-->>Listener: false
        Listener->>Client: drop(stream) 立即拒绝
    else 允许连接
        State-->>Listener: true
        Listener->>Conn: tokio::spawn(Connection::handle)
    end

    loop 读-解析-写循环
        Client->>Conn: TCP 数据到达 (最大 16KB read_buf)
        Conn->>Parser: parser.feed(&buf)
        loop Pipeline 内层解析
            Conn->>Parser: parser.parse()
            alt Ok(Some(frame))
                Conn->>Conn: 校验顶层为 Array
                alt 内联命令 (PING/HELLO/ATOM等)
                    Conn->>Conn: 内部快速执行并生成响应
                else 普通命令
                    Conn->>Router: dispatch_command(args)
                    Router-->>Conn: RespValue 响应
                end
                Conn->>Conn: adapt_for_protocol()
                Conn->>Client: TCP 发送响应字节帧
            else Ok(None)
                Note over Conn: 数据不足, 退出内层循环等待更多 TCP 数据
            else Err(fatal)
                Note over Conn: 致命协议错误, 立即退出连接
            end
        end
    end
    Conn->>State: on_disconnect() (减少连接数)
```

---

## ATOM 事务处理机制 (MULTI / EXEC / WATCH)

AiKv 在 `Connection` 内部维护 `TransactionState`:

1. **`WATCH <key>`**: 查询被监听 Key 的当前版本号并存入 `watched_keys` 字典;
2. **`MULTI`**: 设置 `in_multi = true`, 开启事务收集模式;
3. **入队阶段**: 在 `in_multi = true` 期间, 除 `EXEC`, `DISCARD`, `WATCH`, `UNWATCH` 外的命令均被放入 `tx_queue`, 立即返回 `+QUEUED`;
4. **`EXEC`**:
   - 检查 `watched_keys` 中的 Key 是否被外部并发写入修改;
   - 若发生冲突, 清空事务队列并返回 `RespValue::NullArray` (`*-1\r\n`);
   - 若未发生冲突, 提取队列中所有 Key 统一按字典序获取 `KeyLock`, 通过 `WriteBatch` 原子落盘并返回各命令的执行结果数组;
5. **`DISCARD`**: 清空 `tx_queue` 与 `watched_keys`, 重置 `in_multi = false`, 返回 `+OK`.
