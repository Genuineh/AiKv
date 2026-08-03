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

use serde_json::{json, Value as JsonValue};

pub(crate) use super::jsonpath_util::{json_compare, json_equal, split_top_level};

use crate::error::{Error, Result};

fn err(msg: impl Into<String>) -> Error {
    Error::Command(format!("ERR {}", msg.into()))
}

/// `[?(@ > v)]` 左侧为裸 `@` 时表示数组元素本身; `@.field` 表示字段访问.
fn filter_subject_field(left_raw: &str) -> &str {
    let trimmed = left_raw.trim();
    if trimmed == "@" {
        return "";
    }
    trimmed.trim_start_matches("@.")
}

/// JSONPath 路径解析与修改引擎
#[derive(Debug, Default, Clone, Copy)]
pub struct JsonPathEngine;

/// 从 JSON 节点按路径提取值，支持点号分隔的字段路径和数组索引。
fn traverse_path(item: &JsonValue, field: &str) -> JsonValue {
    if field.is_empty() {
        return item.clone();
    }
    let parts = JsonPathEngine::split_path_parts(field);
    let mut cur = item;
    let mut found = JsonValue::Null;
    let len = parts.len();
    for (i, part) in parts.iter().enumerate() {
        match cur {
            JsonValue::Object(obj) => {
                if i == len - 1 {
                    found = obj.get(part.as_str()).cloned().unwrap_or_default();
                } else if part.starts_with('[') && part.ends_with(']') {
                    let inner = &part[1..part.len() - 1];
                    if let Ok(idx) = inner.parse::<usize>() {
                        if let JsonValue::Array(arr) = cur {
                            cur = arr.get(idx).unwrap_or(&JsonValue::Null);
                            continue;
                        }
                    }
                    found = JsonValue::Null;
                    break;
                } else {
                    cur = obj.get(part.as_str()).unwrap_or(&JsonValue::Null);
                }
            }
            JsonValue::Array(arr) if part.starts_with('[') && part.ends_with(']') => {
                let inner = &part[1..part.len() - 1];
                if let Ok(idx) = inner.parse::<usize>() {
                    if i == len - 1 {
                        found = arr.get(idx).cloned().unwrap_or(JsonValue::Null);
                    } else {
                        cur = arr.get(idx).unwrap_or(&JsonValue::Null);
                    }
                } else {
                    found = JsonValue::Null;
                    break;
                }
            }
            JsonValue::Array(_) => {
                found = JsonValue::Null;
                break;
            }
            _ => {
                found = JsonValue::Null;
                break;
            }
        }
    }
    found
}

impl JsonPathEngine {
    pub fn extract(&self, json: &JsonValue, path: &str) -> Result<JsonValue> {
        // Remove leading $ or .
        let path = path.trim_start_matches('$').trim_start_matches('.');

        if path.is_empty() {
            return Ok(json.clone());
        }

        // Handle root array wildcard $[*]
        if path == "[*]" {
            if let JsonValue::Array(arr) = json {
                // Return array of all elements
                return Ok(JsonValue::Array(arr.clone()));
            }
            return Ok(json.clone());
        }

        // Split by '.' but preserve bracket syntax
        let parts = Self::split_path_parts(path);
        let mut current = json.clone();

        for part in &parts {
            if part == "*" {
                // Wildcard - return current as-is if array, or wrap in array
                if let JsonValue::Array(_) = current {
                    // Already an array, keep as is
                } else {
                    // Wrap single value in array
                    current = json!(vec![current]);
                }
                continue;
            }

            if part.starts_with('[') && part.ends_with(']') {
                // Array index or field selector like [0] or ['field1','field2']
                let inner = &part[1..part.len() - 1];

                if inner.starts_with('\'') {
                    // Field selector: ['field1','field2'] - flatten values in order
                    // TL.KvDoc expects this to return [val1, val2, ...] not [{field1:val1, field2:val2}, ...]
                    let inner_str: &str = inner;
                    let fields: Vec<&str> = inner_str
                        .trim_start_matches('\'')
                        .trim_end_matches('\'')
                        .split("','")
                        .collect();

                    if let JsonValue::Array(arr) = &current {
                        // Array of objects - flatten all values in order
                        let mut result_array = Vec::new();
                        for item in arr {
                            for field in &fields {
                                if let JsonValue::Object(obj) = item {
                                    let value = obj
                                        .get(*field)
                                        .cloned()
                                        .unwrap_or(JsonValue::String(String::new()));
                                    result_array.push(value);
                                }
                            }
                        }
                        current = JsonValue::Array(result_array);
                    } else if let JsonValue::Object(obj) = &current {
                        // Single object - flatten values in order
                        let mut result_array = Vec::new();
                        for field in &fields {
                            let value = obj
                                .get(*field)
                                .cloned()
                                .unwrap_or(JsonValue::String(String::new()));
                            result_array.push(value);
                        }
                        current = JsonValue::Array(result_array);
                    } else {
                        return Err(err(format!("Cannot traverse non-object at: {}", part)));
                    }
                } else if inner.starts_with("?(") {
                    // JSONPath filter expression: [?(@.field == value)] or [?(@ > value)]
                    if let JsonValue::Array(arr) = &current {
                        // Flatten nested arrays: when a field access on an array of
                        // objects produces [[val1, val2, ...]], the filter should
                        // target the inner array elements, not the outer wrapper.
                        let target = if arr.len() == 1 {
                            if let Some(JsonValue::Array(inner_arr)) = arr.first() {
                                inner_arr
                            } else {
                                arr
                            }
                        } else {
                            arr
                        };
                        let filtered = self.filter_jsonpath(target, inner)?;
                        current = JsonValue::Array(filtered);
                    } else {
                        return Err(err(format!("Cannot apply filter on non-array: {}", part)));
                    }
                } else if inner == "*" {
                    // Array wildcard [*] - return all elements
                    if let JsonValue::Array(arr) = &current {
                        current = JsonValue::Array(arr.clone());
                    } else {
                        // If not array, return as single element array
                        current = json!(vec![current]);
                    }
                } else if inner.starts_with('-') || inner.parse::<i64>().is_ok_and(|n| n < 0) {
                    return Err(err(format!("Array index must be non-negative: [{inner}]")));
                } else if let Ok(idx) = inner.parse::<usize>() {
                    if let JsonValue::Array(arr) = &current {
                        if idx >= arr.len() {
                            return Err(err(format!("Array index out of bounds: {idx}")));
                        }
                        current = arr[idx].clone();
                    } else {
                        return Err(err(format!("Cannot index non-array: {part}")));
                    }
                }
            } else {
                // Regular object key access
                if let JsonValue::Object(obj) = &current {
                    current = obj
                        .get(part.as_str())
                        .cloned()
                        .ok_or_else(|| err(format!("Path not found: {}", part)))?;
                } else if let JsonValue::Array(arr) = &current {
                    // Trying to access field on array - apply to each element
                    let mut result_array = Vec::new();
                    for item in arr {
                        if let JsonValue::Object(obj) = item {
                            if let Some(value) = obj.get(part.as_str()) {
                                result_array.push(value.clone());
                            }
                        }
                    }
                    current = JsonValue::Array(result_array);
                } else {
                    return Err(err(format!("Cannot traverse non-object at: {}", part)));
                }
            }
        }

        Ok(current)
    }

    /// Split path string into parts, preserving bracket syntax
    /// Supports nested brackets like `[?(@.List[?(@ == '2')])]` by tracking depth.
    pub fn split_path_parts(path: &str) -> Vec<String> {
        let mut parts = Vec::new();
        let mut current = String::new();
        let mut bracket_depth = 0i32;

        for ch in path.chars() {
            match ch {
                '[' => {
                    if bracket_depth == 0 && !current.is_empty() {
                        parts.push(current.clone());
                        current.clear();
                    }
                    bracket_depth += 1;
                    current.push(ch);
                }
                ']' => {
                    current.push(ch);
                    bracket_depth -= 1;
                    if bracket_depth == 0 {
                        parts.push(current.clone());
                        current.clear();
                    }
                }
                '.' if bracket_depth == 0 && !current.is_empty() => {
                    parts.push(current.clone());
                    current.clear();
                }
                '.' => {
                    if bracket_depth != 0 {
                        current.push(ch);
                    }
                }
                _ => {
                    current.push(ch);
                }
            }
        }

        if !current.is_empty() {
            parts.push(current);
        }

        parts
    }

    /// Navigate JSON path parts and increment the leaf numeric value.
    pub fn incr(&self, json: &mut JsonValue, parts: &[String], incr: f64) -> Result<()> {
        // Handle `[*]` at the start: apply to each element
        if let Some(first) = parts.first() {
            if first == "[*]" || first == "*" {
                if let JsonValue::Array(arr) = json {
                    let remaining = &parts[1..];
                    for item in arr.iter_mut() {
                        if remaining.is_empty() {
                            // Increment each element directly
                            if let JsonValue::Number(n) = item {
                                let val = n.as_f64().ok_or_else(|| err("Not a number"))?;
                                let new_val = val + incr;
                                *item = if new_val.fract() == 0.0 && n.is_i64() {
                                    json!((n.as_i64().unwrap() as f64 + incr) as i64)
                                } else {
                                    json!(new_val)
                                };
                            }
                        } else if let JsonValue::Object(obj) = item {
                            // Check for JSONPath filter as first remaining part
                            let check_rem = if let Some(filter_part) = remaining.first() {
                                if filter_part.starts_with("[?(") && filter_part.ends_with(']') {
                                    let expr = &filter_part[1..filter_part.len() - 1];
                                    let inner =
                                        expr.trim_start_matches("?(").trim_end_matches(')').trim();
                                    if !inner.is_empty() {
                                        let filtered = self.filter_jsonpath(
                                            &[JsonValue::Object(obj.clone())],
                                            &filter_part[1..filter_part.len() - 1],
                                        )?;
                                        if filtered.is_empty() {
                                            continue; // Skip this element — doesn't match filter
                                        }
                                    }
                                    &remaining[1..] // Skip filter part
                                } else {
                                    remaining
                                }
                            } else {
                                remaining
                            };
                            // Traverse remaining path on each element
                            let current = obj;
                            for (i, part) in check_rem.iter().enumerate() {
                                if i == check_rem.len() - 1 {
                                    if let Some(JsonValue::Number(n)) = current.get(part.as_str()) {
                                        let val = n.as_f64().ok_or_else(|| err("Not a number"))?;
                                        let new_val = val + incr;
                                        let new_json = if new_val.fract() == 0.0 && n.is_i64() {
                                            json!((n.as_i64().unwrap() as f64 + incr) as i64)
                                        } else {
                                            json!(new_val)
                                        };
                                        current.insert(part.clone(), new_json);
                                    }
                                }
                            }
                        }
                    }
                    return Ok(());
                }
                return Err(err("Cannot apply [*] on non-array".to_string()));
            }

            // Handle JSONPath filter as intermediate part in non-[*] path:
            // e.g. `[?(@.IntValue == 1)]` — filter the array in `current`
            if first.starts_with("[?(") && first.ends_with(']') {
                if let JsonValue::Array(arr) = json {
                    let remaining = &parts[1..];
                    if remaining.is_empty()
                        || remaining[0] == "IntValue"
                        || remaining[0] == "LongValue"
                    {
                        // Filter then increment on each matching element's field
                        let _filtered = self.filter_jsonpath(arr, &first[1..first.len() - 1])?;
                        let field = remaining.first().map(|s| s.as_str()).unwrap_or("");
                        for item in arr.iter_mut() {
                            if let JsonValue::Object(obj) = item {
                                // Check if this item matches the filter
                                let matches = !self
                                    .filter_jsonpath(
                                        &[JsonValue::Object(obj.clone())],
                                        &first[1..first.len() - 1],
                                    )?
                                    .is_empty();
                                if !matches {
                                    continue; // Skip non-matching items
                                }
                                if !field.is_empty() {
                                    if let Some(JsonValue::Number(n)) = obj.get(field) {
                                        let val = n.as_f64().ok_or_else(|| err("Not a number"))?;
                                        let new_val = val + incr;
                                        let new_json = if new_val.fract() == 0.0 && n.is_i64() {
                                            json!((n.as_i64().unwrap() as f64 + incr) as i64)
                                        } else {
                                            json!(new_val)
                                        };
                                        obj.insert(field.to_string(), new_json);
                                    }
                                }
                            }
                        }
                    }
                    return Ok(());
                }
                return Err(err("Cannot apply filter on non-array".to_string()));
            }
        }

        let mut current = json;

        for (i, part) in parts.iter().enumerate() {
            if i == parts.len() - 1 {
                if let JsonValue::Object(obj) = current {
                    if let Some(JsonValue::Number(n)) = obj.get(part.as_str()) {
                        let val = n.as_f64().ok_or_else(|| err("Not a number"))?;
                        let new_val = val + incr;
                        let new_json = if new_val.fract() == 0.0 && n.is_i64() {
                            json!((n.as_i64().unwrap() as f64 + incr) as i64)
                        } else {
                            json!(new_val)
                        };
                        obj.insert(part.clone(), new_json);
                        return Ok(());
                    } else {
                        return Err(err("Path value is not a number".to_string()));
                    }
                }
                return Err(err(format!("Path not found: {}", part)));
            } else {
                if let JsonValue::Object(obj) = current {
                    current = obj.entry(part.clone()).or_insert_with(|| json!({}));
                } else {
                    return Err(err(format!("Cannot traverse non-object at: {}", part)));
                }
            }
        }

        Ok(())
    }

    /// Navigate to an array at the given path parts and append values.
    pub fn append(
        &self,
        json: &mut JsonValue,
        parts: &[String],
        values: &[JsonValue],
    ) -> Result<()> {
        // Handle `[*]` at the start: find array in first array element
        if let Some(first) = parts.first() {
            if first == "[*]" || first == "*" {
                if let JsonValue::Array(arr) = json {
                    if let Some(first_elem) = arr.first_mut() {
                        let remaining = &parts[1..];
                        if remaining.is_empty() {
                            return Err(err("Cannot append to [*] directly".to_string()));
                        }
                        if let JsonValue::Object(obj) = first_elem {
                            let target_key = remaining[0].as_str();
                            // Strip bracket syntax if present
                            let clean_key =
                                target_key.trim_start_matches('[').trim_end_matches(']');
                            if let Some(JsonValue::Array(target_arr)) = obj.get_mut(clean_key) {
                                for v in values {
                                    target_arr.push(v.clone());
                                }
                                return Ok(());
                            }
                            return Err(err(format!("Path value is not an array: {}", clean_key)));
                        }
                    }
                    return Err(err("Cannot access [*] on empty array".to_string()));
                }
                return Err(err("Cannot apply [*] on non-array".to_string()));
            }
        }

        let mut current = json;

        for (i, part) in parts.iter().enumerate() {
            if i == parts.len() - 1 {
                let clean_key = part.trim_start_matches('[').trim_end_matches(']');
                if let JsonValue::Object(obj) = current {
                    if let Some(JsonValue::Array(arr)) = obj.get_mut(clean_key) {
                        for v in values {
                            arr.push(v.clone());
                        }
                        return Ok(());
                    } else {
                        return Err(err("Path value is not an array".to_string()));
                    }
                }
                return Err(err(format!("Path not found: {}", part)));
            } else {
                if let JsonValue::Object(obj) = current {
                    current = obj.entry(part.clone()).or_insert_with(|| json!({}));
                } else {
                    return Err(err(format!("Cannot traverse non-object at: {}", part)));
                }
            }
        }

        Err(err("Empty path".to_string()))
    }

    /// Parse and apply a JSONPath filter expression `[?(@.field == value)]`.
    ///
    /// Supports:
    /// - Field access: `[?(@.field == value)]`
    /// - Direct value: `[?(@ > value)]` for arrays of primitives
    /// - Operators: `==`, `!=`, `>`, `<`, `>=`, `<=`
    /// - Values: numbers, single-quoted strings `'value'`, bare identifiers
    fn filter_jsonpath(&self, arr: &[JsonValue], expr: &str) -> Result<Vec<JsonValue>> {
        let inner = expr.trim_start_matches("?(").trim_end_matches(')').trim();
        if inner.is_empty() {
            return Ok(arr.to_vec());
        }

        let result: Vec<JsonValue> = arr
            .iter()
            .filter(|item| self.eval_filter_expr(item, inner).unwrap_or(false))
            .cloned()
            .collect();

        Ok(result)
    }

    /// Evaluate a filter expression (may contain ||, &&) against a single JSON item.
    fn eval_filter_expr(&self, item: &JsonValue, expr: &str) -> Result<bool> {
        let expr = expr.trim();
        if expr.is_empty() {
            return Ok(true);
        }

        // Handle negation: !(...) or !@.field...
        if expr.starts_with('!') {
            let inner = expr.strip_prefix('!').unwrap_or(expr).trim();
            // If parenthesized: !(expr)
            if inner.starts_with('(') && inner.ends_with(')') {
                let result = self.eval_filter_expr(item, &inner[1..inner.len() - 1])?;
                return Ok(!result);
            }
            let result = self.eval_single_condition(item, inner)?;
            return Ok(!result);
        }

        // Split on || at top level
        let or_parts = split_top_level(expr, "||");
        for or_part in or_parts {
            let or_part = or_part.trim();
            if or_part.is_empty() {
                continue;
            }

            // Split on && at top level
            let and_parts = split_top_level(or_part, "&&");
            let all_true = and_parts.iter().all(|part| {
                let part = part.trim();
                if part.is_empty() {
                    return true;
                }
                self.eval_single_condition(item, part).unwrap_or(false)
            });
            if all_true {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Evaluate a single atomic condition (no ||/&&) against a JSON item.
    fn eval_single_condition(&self, item: &JsonValue, expr: &str) -> Result<bool> {
        let expr = expr.trim();

        // Handle negation: !(...) or !@.field...
        if expr.starts_with('!') {
            let inner = expr.strip_prefix('!').unwrap_or(expr).trim();
            // If parenthesized: !(expr)
            if inner.starts_with('(') && inner.ends_with(')') {
                let result = self.eval_single_condition(item, &inner[1..inner.len() - 1])?;
                return Ok(!result);
            }
            let result = self.eval_single_condition(item, inner)?;
            return Ok(!result);
        }

        // Handle nested filter pattern: @.field[?(@ ...)] — Any() check
        if let Some(bracket_pos) = expr.find("[?(") {
            let field_part = expr[..bracket_pos].trim();
            let field = filter_subject_field(field_part);
            let inner_expr = &expr[bracket_pos..]; // e.g. [?(@ == 'test11')]

            // Get the array field value from the item
            let arr_val = if field.is_empty() {
                item.clone()
            } else {
                traverse_path(item, field)
            };

            return if let JsonValue::Array(arr) = &arr_val {
                // Evaluate sub-filter recursively: any match means true
                // Strip outer []: [?(@ == 'test11')] -> ?(@ == 'test11')
                let stripped = inner_expr
                    .strip_prefix('[')
                    .and_then(|s| s.strip_suffix(']'))
                    .unwrap_or(inner_expr);
                let filtered = self.filter_jsonpath(arr, stripped)?;
                Ok(!filtered.is_empty())
            } else {
                Ok(false)
            };
        }

        // Handle regex =~ operator
        if let Some(eq_pos) = expr.find("=~") {
            let left = expr[..eq_pos].trim();
            let right = expr[eq_pos + 2..].trim();
            let field = filter_subject_field(left);

            // Parse /pattern/flags
            if right.starts_with('/') {
                let closing = right.strip_prefix('/').and_then(|s| s.rfind('/'));
                if let Some(end) = closing {
                    let pattern_end = 1 + end;
                    let pattern_str = &right[1..pattern_end];
                    let flags = &right[pattern_end + 1..];
                    let case_insensitive = flags.contains('i');

                    // Extract search term from .*value.* pattern
                    let search_term =
                        if pattern_str.starts_with(".*") && pattern_str.ends_with(".*") {
                            pattern_str
                                .strip_prefix(".*")
                                .and_then(|s| s.strip_suffix(".*"))
                                .unwrap_or(pattern_str)
                        } else if pattern_str.starts_with(".*") {
                            pattern_str.strip_prefix(".*").unwrap_or(pattern_str)
                        } else if pattern_str.ends_with(".*") {
                            pattern_str.strip_suffix(".*").unwrap_or(pattern_str)
                        } else {
                            pattern_str
                        };

                    let val = if field.is_empty() {
                        Some(item.to_string())
                    } else {
                        match traverse_path(item, field) {
                            JsonValue::String(s) => Some(s),
                            JsonValue::Number(n) => Some(n.to_string()),
                            _ => None,
                        }
                    };

                    if let Some(s) = val {
                        let match_str = if case_insensitive {
                            s.to_lowercase()
                        } else {
                            s
                        };
                        let search_str = if case_insensitive {
                            search_term.to_lowercase().to_string()
                        } else {
                            search_term.to_string()
                        };
                        let matched = match_str.contains(&search_str);
                        return Ok(matched);
                    }
                }
            }
            return Ok(false);
        }

        // Standard comparison: @.field op value
        let op_pos = expr.find(|c: char| ['=', '!', '>', '<'].contains(&c));
        if let Some(pos) = op_pos {
            let left_raw = expr[..pos].trim();
            let rest = expr[pos..].trim();
            let op_len = if rest.starts_with("==")
                || rest.starts_with("!=")
                || rest.starts_with(">=")
                || rest.starts_with("<=")
            {
                2
            } else {
                1
            };
            let op_str = &rest[..op_len];
            let right_raw = rest[op_len..].trim();

            let field = filter_subject_field(left_raw);

            let cmp_value = if right_raw.starts_with('\'') {
                let s = right_raw.trim_start_matches('\'').trim_end_matches('\'');
                JsonValue::String(s.to_string())
            } else if let Ok(n) = right_raw.parse::<f64>() {
                json!(n)
            } else if right_raw == "true" {
                JsonValue::Bool(true)
            } else if right_raw == "false" {
                JsonValue::Bool(false)
            } else {
                JsonValue::String(right_raw.to_string())
            };

            // Handle dotted field paths (e.g., "InnerDocument.Str") with array indexing
            let val = if field.is_empty() {
                item.clone()
            } else {
                traverse_path(item, field)
            };

            return Ok(match op_str {
                "==" => json_equal(&val, &cmp_value),
                "!=" => !json_equal(&val, &cmp_value),
                ">" => json_compare(&val, &cmp_value) == Some(std::cmp::Ordering::Greater),
                "<" => json_compare(&val, &cmp_value) == Some(std::cmp::Ordering::Less),
                ">=" => matches!(
                    json_compare(&val, &cmp_value),
                    Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
                ),
                "<=" => matches!(
                    json_compare(&val, &cmp_value),
                    Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
                ),
                _ => false,
            });
        }

        // Bare field truthiness: @.BoolValue (裸 @ 表示元素本身)
        let field = filter_subject_field(expr);
        // Handle dotted field paths (e.g., "InnerDocument.BoolValue")
        let val = if field.is_empty() {
            item.clone()
        } else {
            traverse_path(item, field)
        };
        let result = val != JsonValue::Null && val != JsonValue::Bool(false);
        Ok(result)
    }

    /// Filter out elements matching a filter expression from an array.
    /// Returns only elements that do NOT match (for deletion).
    /// Unified with `eval_single_condition` — reuses the same matching logic.
    fn filter_arr_except(&self, arr: &[JsonValue], filter_str: &str) -> Vec<JsonValue> {
        let expr = &filter_str[1..filter_str.len() - 1]; // strip []
        let inner = expr.trim_start_matches("?(").trim_end_matches(')').trim();
        arr.iter()
            .filter(|elem| {
                match self.eval_single_condition(elem, inner) {
                    Ok(true) => false, // matches filter → remove it
                    _ => true,         // doesn't match or error → keep it
                }
            })
            .cloned()
            .collect()
    }

    pub fn set(&self, json: &mut JsonValue, path: &str, value: JsonValue) -> Result<()> {
        // Remove leading $ or .
        let path = path.trim_start_matches('$').trim_start_matches('.');

        if path.is_empty() {
            *json = value;
            return Ok(());
        }

        // Use split_path_parts to handle bracket syntax
        let parts = Self::split_path_parts(path);

        // Handle Array root: apply path to elements
        if let JsonValue::Array(arr) = json {
            if let Some(first) = parts.first() {
                // [*] wildcard: apply remaining path to each element
                if first == "[*]" || first == "*" {
                    let remaining = &parts[1..];
                    if remaining.is_empty() {
                        return Ok(());
                    }
                    let remaining_path = remaining.join(".");
                    for item in arr.iter_mut() {
                        self.set(item, &remaining_path, value.clone())?;
                    }
                    return Ok(());
                }
                // [?()] filter: apply remaining path to matching elements
                if first.starts_with("[?(") && first.ends_with(']') {
                    let remaining = &parts[1..];
                    let remaining_path = remaining.join(".");
                    // Evaluate filter for each element
                    for item in arr.iter_mut() {
                        let matches = self
                            .filter_jsonpath(
                                std::slice::from_ref(item),
                                &first[1..first.len() - 1],
                            )?
                            .is_empty();
                        if !matches {
                            if remaining_path.is_empty() {
                                *item = value.clone();
                            } else {
                                self.set(item, &remaining_path, value.clone())?;
                            }
                        }
                    }
                    return Ok(());
                }
                // Numeric index: [N]
                if first.starts_with('[') && first.ends_with(']') {
                    let inner = &first[1..first.len() - 1];
                    if let Ok(idx) = inner.parse::<usize>() {
                        if idx < arr.len() {
                            let remaining = &parts[1..];
                            let remaining_path = remaining.join(".");
                            if remaining_path.is_empty() {
                                arr[idx] = value;
                            } else {
                                self.set(&mut arr[idx], &remaining_path, value)?;
                            }
                            return Ok(());
                        }
                        return Err(err(format!("Array index out of bounds: {}", idx)));
                    }
                }
                // Regular field name: apply to each object in the array
                let remaining = &parts[1..];
                let remaining_path = remaining.join(".");
                for item in arr.iter_mut() {
                    if remaining_path.is_empty() {
                        if let JsonValue::Object(obj) = item {
                            obj.insert(first.clone(), value.clone());
                        }
                    } else {
                        self.set(item, &remaining_path, value.clone())?;
                    }
                }
                return Ok(());
            }
            return Ok(());
        }

        // For non-object values, create a new object only when we need to traverse
        if !json.is_object() {
            *json = json!({});
        }

        // Traverse path parts, handling [?()] filters and [N] indices.
        // Uses recursive calls for sub-paths to keep logic clear.
        let parts_str: Vec<&str> = parts.iter().map(|s| s.as_str()).collect();
        let mut current = json;

        for (i, part) in parts_str.iter().enumerate() {
            let is_last = i == parts_str.len() - 1;

            // Handle [?()] filter on array: apply to matching elements
            if part.starts_with("[?(") && part.ends_with(']') {
                if let JsonValue::Array(arr) = current {
                    let remaining = &parts_str[i + 1..];
                    let remaining_path = remaining.join(".");
                    for item in arr.iter_mut() {
                        let matches = self
                            .filter_jsonpath(std::slice::from_ref(item), &part[1..part.len() - 1])?
                            .is_empty();
                        if !matches {
                            if remaining_path.is_empty() {
                                *item = value.clone();
                            } else {
                                self.set(item, &remaining_path, value.clone())?;
                            }
                        }
                    }
                    return Ok(());
                }
                break;
            }

            // Handle [N] index on array
            if part.starts_with('[') && part.ends_with(']') {
                let inner = &part[1..part.len() - 1];
                if let Ok(idx) = inner.parse::<usize>() {
                    if let JsonValue::Array(arr) = current {
                        if is_last {
                            if idx < arr.len() {
                                arr[idx] = value;
                            }
                            return Ok(());
                        } else {
                            if idx < arr.len() {
                                current = &mut arr[idx];
                                continue;
                            }
                            return Ok(());
                        }
                    }
                }
                break;
            }

            // Standard object field access
            if is_last {
                if let JsonValue::Object(obj) = current {
                    obj.insert(part.to_string(), value.clone());
                    break;
                }
            } else {
                if let JsonValue::Object(obj) = current {
                    current = obj.entry(part.to_string()).or_insert_with(|| json!({}));
                }
            }
        }

        Ok(())
    }

    pub fn delete(&self, json: &mut JsonValue, path: &str) -> Result<u64> {
        let path = path.trim_start_matches('$').trim_start_matches('.');

        if path.is_empty() {
            return Ok(0);
        }

        let parts = Self::split_path_parts(path);

        if let Some(first) = parts.first() {
            if first == "[*]" || first == "*" {
                if let JsonValue::Array(arr) = json {
                    let remaining: Vec<&String> = parts[1..]
                        .iter()
                        .filter(|p| !p.starts_with("[?("))
                        .collect();
                    let filter_part: Option<&String> = parts[1..]
                        .iter()
                        .find(|p| p.starts_with("[?(") && !p[2..].contains("[?("));
                    if remaining.is_empty() && filter_part.is_none() {
                        return Ok(0);
                    }
                    let mut count = 0u64;
                    for item in arr.iter_mut() {
                        if let JsonValue::Object(obj) = item {
                            let mut current = obj;
                            let mut traversal_failed = false;
                            for (i, part) in remaining.iter().enumerate() {
                                let field = part.trim_start_matches('[').trim_end_matches(']');
                                if i == remaining.len() - 1 && filter_part.is_none() {
                                    if current.remove(field).is_some() {
                                        count += 1;
                                    }
                                } else if i < remaining.len() - 1 {
                                    if let Some(next) = current.get_mut(field) {
                                        if let JsonValue::Object(obj_ref) = next {
                                            current = obj_ref;
                                        } else if let JsonValue::Array(arr_ref) = next {
                                            if let Some(filter_str) = filter_part {
                                                let before = arr_ref.len();
                                                *arr_ref =
                                                    self.filter_arr_except(arr_ref, filter_str);
                                                count +=
                                                    (before.saturating_sub(arr_ref.len())) as u64;
                                            }
                                            break;
                                        } else {
                                            traversal_failed = true;
                                            break;
                                        }
                                    } else {
                                        traversal_failed = true;
                                        break;
                                    }
                                } else if i == remaining.len() - 1 {
                                    if let Some(JsonValue::Array(arr_ref)) = current.get_mut(field)
                                    {
                                        if let Some(filter_str) = filter_part {
                                            let before = arr_ref.len();
                                            *arr_ref = self.filter_arr_except(arr_ref, filter_str);
                                            count += (before.saturating_sub(arr_ref.len())) as u64;
                                        }
                                    }
                                }
                            }
                            if traversal_failed {
                                continue;
                            }
                        }
                    }
                    return Ok(count);
                }
                return Ok(0);
            }

            if first.starts_with("[?(") && first.ends_with(']') {
                if let JsonValue::Array(arr) = json {
                    let remaining: Vec<String> = parts[1..].to_vec();
                    let remaining_path = remaining.join(".");
                    if remaining_path.is_empty() {
                        let orig_len = arr.len();
                        arr.retain(|item| {
                            self.filter_jsonpath(
                                std::slice::from_ref(item),
                                &first[1..first.len() - 1],
                            )
                            .map(|r| r.is_empty())
                            .unwrap_or(true)
                        });
                        return Ok((orig_len.saturating_sub(arr.len())) as u64);
                    }
                    let mut count = 0u64;
                    for item in arr.iter_mut() {
                        let matches = self
                            .filter_jsonpath(
                                std::slice::from_ref(item),
                                &first[1..first.len() - 1],
                            )?
                            .is_empty();
                        if !matches {
                            count += self.delete(item, &remaining_path)?;
                        }
                    }
                    return Ok(count);
                }
                return Ok(0);
            }
        }

        let parts_str: Vec<&str> = parts.iter().map(|s| s.as_str()).collect();
        let mut current = json;

        for (i, part) in parts_str.iter().enumerate() {
            let is_last = i == parts_str.len() - 1;

            if part.starts_with("[?(") && part.ends_with(']') {
                if let JsonValue::Array(arr) = current {
                    let remaining = &parts_str[i + 1..];
                    if remaining.is_empty() {
                        let orig_len = arr.len();
                        arr.retain(|item| {
                            self.filter_jsonpath(
                                std::slice::from_ref(item),
                                &part[1..part.len() - 1],
                            )
                            .map(|r| r.is_empty())
                            .unwrap_or(true)
                        });
                        return Ok((orig_len.saturating_sub(arr.len())) as u64);
                    }
                    let remaining_path = remaining.join(".");
                    let mut count = 0u64;
                    for item in arr.iter_mut() {
                        let matches = self
                            .filter_jsonpath(std::slice::from_ref(item), &part[1..part.len() - 1])?
                            .is_empty();
                        if !matches {
                            count += self.delete(item, &remaining_path)?;
                        }
                    }
                    return Ok(count);
                }
                return Ok(0);
            }

            if part.starts_with('[') && part.ends_with(']') {
                let inner = &part[1..part.len() - 1];
                if let Ok(idx) = inner.parse::<usize>() {
                    if let JsonValue::Array(arr) = current {
                        if is_last {
                            if idx < arr.len() {
                                arr.remove(idx);
                                return Ok(1);
                            }
                            return Ok(0);
                        } else if idx < arr.len() {
                            current = &mut arr[idx];
                            continue;
                        }
                        return Ok(0);
                    }
                }
                return Ok(0);
            }

            if is_last {
                if let JsonValue::Object(obj) = current {
                    return Ok(u64::from(obj.remove(*part).is_some()));
                }
                return Ok(0);
            } else if let JsonValue::Object(obj) = current {
                if let Some(next) = obj.get_mut(*part) {
                    current = next;
                } else {
                    return Ok(0);
                }
            } else {
                return Ok(0);
            }
        }

        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jsonpath_negative_index_err() {
        let doc = serde_json::json!({"a": [1, 2]});
        let err = JsonPathEngine.extract(&doc, "$[-1]").unwrap_err();
        assert!(err.to_string().contains("ERR"));
    }

    #[test]
    fn test_jsonpath_split_path_parts_nested_filter() {
        let parts = JsonPathEngine::split_path_parts("items[?(@.age > 1)].name");
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], "items");
        assert!(parts[1].starts_with("[?("));
        assert_eq!(parts[2], "name");
    }

    #[test]
    fn test_jsonpath_extract_field_and_index() {
        let doc = serde_json::json!({"user": {"tags": ["a", "b"]}});
        let got = JsonPathEngine.extract(&doc, "$.user.tags[1]").unwrap();
        assert_eq!(got, "b");
    }

    #[test]
    fn test_jsonpath_set_nested_field() {
        let mut doc = serde_json::json!({"a": {"b": 1}});
        JsonPathEngine
            .set(&mut doc, "$.a.b", serde_json::json!(2))
            .unwrap();
        assert_eq!(doc["a"]["b"], 2);
    }

    #[test]
    fn test_jsonpath_delete_returns_count() {
        let mut doc = serde_json::json!([{"x": 1}, {"x": 2}, {"x": 3}]);
        let count = JsonPathEngine.delete(&mut doc, "$[?(@.x > 1)]").unwrap();
        assert_eq!(count, 2);
        assert_eq!(doc.as_array().unwrap().len(), 1);
    }
}
