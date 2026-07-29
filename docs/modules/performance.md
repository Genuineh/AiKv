---
name: performance
description: AiKv performance tuning and allocation optimization documentation
---

# 性能优化

## 全局分配器

生产环境使用 mimalloc 替代 glibc malloc. 在 aikv main.rs 通过 `#[global_allocator]` 设置.

### 基线数据

基于 eBPF 火焰图 (Grafana Profiles / Pyroscope, 15min 累计):

| 排名 | Symbol | Self Time | CPU 占比 |
|:----:|--------|:---------:|:--------:|
| 1 | `__libc_malloc` | 8.42 s | 8.1% |
| 2 | `cfree` | 7.37 s | 7.1% |
| 3 | `seccomp_export_bpf` | 3.95 s | 3.8% |
| 4 | `__libc_realloc` | 3.53 s | 3.4% |
| **合计** | malloc + free + realloc | **~19.3 s** | **18.6%** |

### 预期收益

- 分配器自身开销 (锁争用、碎片整理) 降低 **30-50%**
- 总 CPU 收益 **5-9%**

### 验证方法

```bash
# 对比换分配器前后的吞吐与延迟
redis-benchmark -h 127.0.0.1 -p 6379 -t SET,GET -n 50000 -c 50 -d 64 --cluster

# 火焰图验证 malloc 占比是否下降
# Grafana Profiles: https://192.168.1.115:3000/d/aikv-profiles/profiles
```

### 后续优化 (Phase 2)

在验证 Phase 1 收益后, 通过新火焰图定位逻辑层分配热点, 进行 buffer 复用优化. 见 `superpower/specs/2026-07-29-memory-optimization-design.md`.
