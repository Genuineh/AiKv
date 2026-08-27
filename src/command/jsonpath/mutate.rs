use serde_json::{json, Value as JsonValue};

use crate::error::Result;

use super::{err, JsonPathEngine};

impl JsonPathEngine {
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
