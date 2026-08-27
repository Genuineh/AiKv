use serde_json::{json, Value as JsonValue};

use crate::command::jsonpath_util::{json_compare, json_equal, split_top_level};
use crate::error::Result;

use super::eval::traverse_path;
use super::JsonPathEngine;

/// `[?(@ > v)]` 左侧为裸 `@` 时表示数组元素本身; `@.field` 表示字段访问.
pub(super) fn filter_subject_field(left_raw: &str) -> &str {
    let trimmed = left_raw.trim();
    if trimmed == "@" {
        return "";
    }
    trimmed.trim_start_matches("@.")
}

impl JsonPathEngine {
    /// Parse and apply a JSONPath filter expression `[?(@.field == value)]`.
    ///
    /// Supports:
    /// - Field access: `[?(@.field == value)]`
    /// - Direct value: `[?(@ > value)]` for arrays of primitives
    /// - Operators: `==`, `!=`, `>`, `<`, `>=`, `<=`
    /// - Values: numbers, single-quoted strings `'value'`, bare identifiers
    pub(super) fn filter_jsonpath(&self, arr: &[JsonValue], expr: &str) -> Result<Vec<JsonValue>> {
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
    pub(super) fn eval_filter_expr(&self, item: &JsonValue, expr: &str) -> Result<bool> {
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
    pub(super) fn eval_single_condition(&self, item: &JsonValue, expr: &str) -> Result<bool> {
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
    pub(super) fn filter_arr_except(&self, arr: &[JsonValue], filter_str: &str) -> Vec<JsonValue> {
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
}
