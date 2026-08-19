//! JSONPath 路径解析与求值引擎 (Phase 11): 供 `json.rs` 的 JSON.* 命令与
//! `script/json_exec.rs` (Lua 内 JSON 子集) 复用. 纯函数式, 不接触存储层.
//!
//! # 求值流程
//!
//! ```text
//! 路径字符串 ── split_path_parts (保留中括号语法, 按深度跟踪嵌套 filter) ──> parts
//!   │
//!   ├─ extract(json, path): trim 前导 $/. → 逐 part 遍历当前节点
//!   │    ├─ .field        → 对象字段访问; 对数组节点访问字段 → 逐元素展开为数组
//!   │    ├─ ['a','b']     → 多字段选择, 按序展平取值
//!   │    ├─ [N]           → 数组索引 (负索引 / 越界 → ERR)
//!   │    ├─ [*] / *       → 通配: 数组全部元素 (非数组则包成单元素数组)
//!   │    └─ [?(@…)]       → filter_jsonpath → eval_filter_expr → eval_single_condition
//!   │
//!   ├─ set(json, path, value): 沿路径递归写入; [*] / [?()] / [N] 支持批量写
//!   ├─ delete(json, path): 删除节点, 返回删除个数
//!   ├─ incr(json, parts, incr): 数值节点自增 (支持 [*] 批量)
//!   └─ append(json, path, value): 数组追加
//! ```
//!
//! # Invariant
//!
//! - 根路径等价: `$` 与 `.` 均为文档根 (trim 前导 `$`/`.`), `$[*]` 特判为根数组整体.
//! - filter 语义: `[?(@ > v)]` 中裸 `@` 表示数组元素本身; `@.field` 表示字段访问
//!   (见 `filter_subject_field`); filter 结果作为数组返回.
//! - 能力边界: 支持 `$`, `.`, `$[N]`, `[*]`, 多字段、filter; 负数组索引拒绝.
//! - 命令层负责加锁与写回存储 (见 `json.rs` / `script/json_exec.rs`).

mod eval;
mod filter;
mod mutate;
mod parser;
#[cfg(test)]
mod tests;

use crate::error::Error;

pub(super) fn err(msg: impl Into<String>) -> Error {
    Error::Command(format!("ERR {}", msg.into()))
}

/// JSONPath 路径解析与修改引擎
#[derive(Debug, Default, Clone, Copy)]
pub struct JsonPathEngine;
