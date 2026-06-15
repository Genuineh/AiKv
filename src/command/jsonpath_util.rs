//! JSONPath 公用工具函数: 表达式拆分、比较。

use serde_json::Value as JsonValue;

/// Split a logical expression by `delim` at the top level (outside quotes and brackets).
pub fn split_top_level<'a>(expr: &'a str, delim: &str) -> Vec<&'a str> {
  let mut parts = Vec::new();
  let mut start = 0;
  let mut in_single_quote = false;
  let mut in_double_quote = false;
  let mut bracket_depth = 0i32;
  let mut i = 0;
  let bytes = expr.as_bytes();
  while i < bytes.len() {
    match bytes[i] {
      b'\'' if !in_double_quote => {
        in_single_quote = !in_single_quote;
      }
      b'"' if !in_single_quote => {
        in_double_quote = !in_double_quote;
      }
      b'[' | b'{' | b'(' if !in_single_quote && !in_double_quote => {
        bracket_depth += 1;
      }
      b']' | b'}' | b')' if !in_single_quote && !in_double_quote => {
        bracket_depth -= 1;
      }
      _ => {}
    }

    if !in_single_quote
      && !in_double_quote
      && bracket_depth == 0
      && bytes[i..].starts_with(delim.as_bytes())
    {
      parts.push(&expr[start..i]);
      i += delim.len();
      start = i;
      continue;
    }
    i += 1;
  }
  if start <= expr.len() {
    parts.push(&expr[start..]);
  }
  parts
}

/// Compare two JSON values for equality, handling type coercion.
pub fn json_equal(a: &JsonValue, b: &JsonValue) -> bool {
  match (a, b) {
    (JsonValue::Number(na), JsonValue::Number(nb)) => na
      .as_f64()
      .is_some_and(|a| nb.as_f64().is_some_and(|b| (a - b).abs() < f64::EPSILON)),
    (JsonValue::String(sa), JsonValue::String(sb)) => sa == sb,
    (JsonValue::Bool(ba), JsonValue::Bool(bb)) => ba == bb,
    (JsonValue::Number(n), JsonValue::String(s)) | (JsonValue::String(s), JsonValue::Number(n)) => {
      if let Ok(nv) = s.parse::<f64>() {
        n.as_f64().is_some_and(|a| (a - nv).abs() < f64::EPSILON)
      } else {
        false
      }
    }
    _ => a == b,
  }
}

/// Compare two JSON values, returning Ordering when comparable.
pub fn json_compare(a: &JsonValue, b: &JsonValue) -> Option<std::cmp::Ordering> {
  match (a, b) {
    (JsonValue::Number(na), JsonValue::Number(nb)) => na
      .as_f64()
      .zip(nb.as_f64())
      .and_then(|(a, b)| a.partial_cmp(&b)),
    (JsonValue::String(sa), JsonValue::String(sb)) => {
      if let (Ok(a), Ok(b)) = (sa.parse::<f64>(), sb.parse::<f64>()) {
        a.partial_cmp(&b)
      } else {
        Some(sa.cmp(sb))
      }
    }
    (JsonValue::Number(n), JsonValue::String(s)) | (JsonValue::String(s), JsonValue::Number(n)) => {
      let nv = n.as_f64()?;
      let sv = s.parse::<f64>().ok()?;
      nv.partial_cmp(&sv)
    }
    _ => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_json_equal_number_coercion() {
    assert!(json_equal(&serde_json::json!(1), &serde_json::json!("1")));
  }

  #[test]
  fn test_split_top_level_or() {
    let parts = split_top_level("a == 1 || b == 2", "||");
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].trim(), "a == 1");
  }

  #[test]
  fn test_split_top_level_and() {
    let parts = split_top_level("a == 1 && b == 2", "&&");
    assert_eq!(parts.len(), 2);
  }

  #[test]
  fn test_json_compare_string_numbers() {
    use std::cmp::Ordering;
    assert_eq!(
      json_compare(&serde_json::json!("2"), &serde_json::json!("1")),
      Some(Ordering::Greater)
    );
    assert_eq!(
      json_compare(&serde_json::json!("10"), &serde_json::json!("2")),
      Some(Ordering::Greater)
    );
  }

  #[test]
  fn test_json_compare_numbers() {
    use std::cmp::Ordering;
    assert_eq!(
      json_compare(&serde_json::json!(5), &serde_json::json!(3)),
      Some(Ordering::Greater)
    );
    assert_eq!(
      json_compare(&serde_json::json!(3), &serde_json::json!(5)),
      Some(Ordering::Less)
    );
    assert_eq!(
      json_compare(&serde_json::json!(5), &serde_json::json!(5.0)),
      Some(Ordering::Equal)
    );
  }
}
