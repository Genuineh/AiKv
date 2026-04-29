use crate::error::{AikvError, Result};
use crate::protocol::RespValue;
use crate::storage::{BatchOp, StorageEngine};
use bytes::Bytes;
use serde_json::Value as JsonValue;

/// Atom batch command handler.
///
/// Implements `ATOM.EXEC <json>` — a custom multi-command transaction that
/// receives a JSON array of command arrays and executes them atomically.
///
/// All commands must target keys in the same slot (verified by the C# client).
pub struct AtomCommands {
    storage: StorageEngine,
}

impl AtomCommands {
    pub fn new(storage: StorageEngine) -> Self {
        Self { storage }
    }

    /// ATOM.EXEC <json_string>
    ///
    /// `<json_string>` is a JSON array of command arrays:
    /// ```
    /// [["JSON.SET", "key", "$", "value", "0", "XE"], ["JSON.DEL", "key", "$[*]"]]
    /// ```
    pub fn atom_exec(&self, args: &[Bytes], current_db: usize) -> Result<RespValue> {
        if args.len() != 1 {
            return Err(AikvError::WrongArgCount("ATOM.EXEC".to_string()));
        }

        let json_str = String::from_utf8_lossy(&args[0]).to_string();
        let commands: Vec<Vec<JsonValue>> = serde_json::from_str(&json_str).map_err(|e| {
            AikvError::InvalidArgument(format!("Invalid ATOM.EXEC JSON: {}", e))
        })?;

        if commands.is_empty() {
            return Ok(RespValue::ok());
        }

        // Build write buffer: key -> serialized JSON value (or delete sentinel)
        let mut write_buf: Vec<(String, BatchOp)> = Vec::new();
        // Track expirations: key -> expire_seconds
        let mut expirations: Vec<(String, u64)> = Vec::new();

        for cmd in &commands {
            if cmd.is_empty() {
                continue;
            }

            let cmd_name = cmd[0].as_str().ok_or_else(|| {
                AikvError::InvalidArgument("Command name must be a string".to_string())
            })?;

            match cmd_name {
                "JSON.SET" => {
                    // JSON.SET key path value expire [flag]
                    if cmd.len() < 4 {
                        return Err(AikvError::WrongArgCount("JSON.SET".to_string()));
                    }
                    let key = cmd[1].as_str().ok_or_else(|| {
                        AikvError::InvalidArgument("JSON.SET key must be a string".to_string())
                    })?;
                    let set_path = cmd[2].as_str().unwrap_or("$");
                    let value = &cmd[3];

                    // TL.KvDoc sends the value as a JSON-encoded string inside the
                    // ATOM.EXEC JSON array (ToKvJson returns a JSON string). We must
                    // parse it as inner JSON to get the actual value bytes.
                    let parsed_value: JsonValue = match value {
                        JsonValue::String(s) => {
                            serde_json::from_str::<JsonValue>(s)
                                .unwrap_or_else(|_| value.clone())
                        }
                        other => other.clone(),
                    };

                    // For sub-paths (not $ or .), read the existing object,
                    // apply the value, and write back.
                    let final_value = if set_path == "$" || set_path == "." {
                        parsed_value
                    } else {
                        let existing = self
                            .storage
                            .get_from_db(current_db, key)?
                            .unwrap_or_default();
                        let mut json: JsonValue = if existing.is_empty() {
                            serde_json::Value::Null
                        } else {
                            serde_json::from_slice(&existing)?
                        };
                        if json == serde_json::Value::Null {
                            json = serde_json::json!({});
                        }
                        // Check filter match when path has [?(@ condition)
                        if set_path.contains("[?(@") {
                            let filter_expr = extract_filter_expr(set_path);
                            if let Some(expr) = filter_expr {
                                if let JsonValue::Array(arr) = &json {
                                    let mut has_match = false;
                                    for item in arr.iter() {
                                        if let JsonValue::Object(obj) = item {
                                            has_match = eval_simple_filter(obj, &expr);
                                            if has_match { break; }
                                        }
                                    }
                                    if !has_match {
                                        return Err(AikvError::InvalidArgument(
                                            "No elements match the where condition".to_string(),
                                        ));
                                    }
                                }
                            }
                        }
                        // Extract the field name from the end of the path.
                        let path_str = set_path.trim_start_matches('$').trim_start_matches('.');
                        let field_name = path_str.rsplit('.').next().unwrap_or(path_str);
                        let clean_field = field_name
                            .trim_start_matches('[')
                            .trim_end_matches(']');
                        // Apply on each element of the root array
                        if let JsonValue::Array(arr) = &mut json {
                            for item in arr.iter_mut() {
                                if let JsonValue::Object(obj) = item {
                                    obj.insert(clean_field.to_string(), parsed_value.clone());
                                }
                            }
                        }
                        json
                    };

                    let serialized = serde_json::to_vec(&final_value).map_err(|e| {
                        AikvError::Storage(format!("JSON.SET serialize error: {}", e))
                    })?;

                    write_buf.push((key.to_string(), BatchOp::Set(Bytes::from(serialized))));

                    // Capture expiration if provided (cmd[4] = expire seconds)
                    if cmd.len() > 4 {
                        if let Some(expire_str) = cmd[4].as_str() {
                            if let Ok(expire_secs) = expire_str.parse::<u64>() {
                                if expire_secs > 0 {
                                    expirations.push((key.to_string(), expire_secs));
                                }
                            }
                        }
                    }
                }
                "JSON.DEL" => {
                    // JSON.DEL key [path]
                    if cmd.len() < 2 {
                        return Err(AikvError::WrongArgCount("JSON.DEL".to_string()));
                    }
                    let key = cmd[1].as_str().ok_or_else(|| {
                        AikvError::InvalidArgument("JSON.DEL key must be a string".to_string())
                    })?;
                    let del_path = if cmd.len() > 2 {
                        cmd[2].as_str().unwrap_or("$")
                    } else {
                        "$"
                    };

                    if del_path == "$" || del_path == "$[*]" {
                        // Delete entire key
                        write_buf.push((key.to_string(), BatchOp::Delete));
                    } else {
                        // Path-level delete: read, delete field, write back
                        let key_str = key.to_string();
                        if let Some(existing) = self.storage.get_from_db(current_db, &key_str)? {
                            let mut json: JsonValue = serde_json::from_slice(&existing)?;
                            eprintln!("[ATOM_DEBUG] JSON.DEL path='{}' json_is_array={}", del_path, json.is_array());
                            let deleted = delete_field_at_path(&mut json, del_path);
                            if deleted {
                                let serialized = serde_json::to_vec(&json)?;
                                write_buf.retain(|(k, _)| k != &key_str);
                                write_buf.push((key_str, BatchOp::Set(Bytes::from(serialized))));
                            }
                        }
                    }
                }
                "JSON.UPDATE" => {
                    // JSON.UPDATE key wherePath path1 value1 [path2 value2 ...] [flag]
                    if cmd.len() < 4 {
                        return Err(AikvError::WrongArgCount("JSON.UPDATE".to_string()));
                    }
                    let key = cmd[1].as_str().ok_or_else(|| {
                        AikvError::InvalidArgument(
                            "JSON.UPDATE key must be a string".to_string(),
                        )
                    })?;
                    let _where_path = cmd[2].as_str().unwrap_or("$");

                    // The last arg may be "" or "NN" flag
                    let mut end = cmd.len();
                    if let Some(last) = cmd.last() {
                        if let Some(s) = last.as_str() {
                            if s.is_empty() || s == "NN" {
                                end = cmd.len() - 1;
                            }
                        }
                    }

                    // Merge all path-value pairs into one final JSON value.
                    // We start from the existing value and apply each path update.
                    let key_str = key.to_string();

                    // Check key exists
                    let exists = self.storage.exists_in_db(current_db, &key_str)?;
                    if !exists {
                        let flag = if cmd.len() > end {
                            cmd.last().and_then(|v| v.as_str()).unwrap_or("")
                        } else {
                            ""
                        };
                        if flag == "NN" {
                            continue;
                        }
                        return Err(AikvError::InvalidArgument(
                            "Key does not exist in ATOM transaction".to_string(),
                        ));
                    }

                    // For UPDATE in batch, read current value, apply modifications
                    let existing = self
                        .storage
                        .get_from_db(current_db, &key_str)?
                        .unwrap_or_default();
                    let mut json: JsonValue = if existing.is_empty() {
                        serde_json::Value::Null
                    } else {
                        serde_json::from_slice(&existing)?
                    };

                    if json == serde_json::Value::Null {
                        json = serde_json::json!({});
                    }

                    // Apply path-value pairs (starting from index 3)
                    let mut i = 3;
                    while i + 1 < end {
                        let _path_str = cmd[i].as_str().unwrap_or("$");
                        let val = cmd[i + 1].clone();
                        // Apply at root level: set the value directly
                        // (simplified: we merge at root since TL.KvDoc
                        //  typically updates the whole object)
                        if let serde_json::Value::Object(ref mut obj) = json {
                            if let serde_json::Value::Object(ref new_obj) = val {
                                for (k, v) in new_obj {
                                    obj.insert(k.clone(), v.clone());
                                }
                            }
                        }
                        i += 2;
                    }

                    let serialized = serde_json::to_vec(&json)?;
                    // De-duplicate: replace previous entry for same key
                    write_buf.retain(|(k, _)| k != &key_str);
                    write_buf.push((key_str, BatchOp::Set(Bytes::from(serialized))));
                }
                _ => {
                    return Err(AikvError::InvalidArgument(format!(
                        "Unknown command in ATOM transaction: {}",
                        cmd_name
                    )));
                }
            }
        }

        if write_buf.is_empty() {
            return Ok(RespValue::ok());
        }

        // Atomic commit via write_batch
        self.storage.write_batch(current_db, write_buf)?;

        // Apply expirations after commit
        for (key, expire_secs) in &expirations {
            let _ = self.storage.set_expire_in_db(current_db, key, expire_secs * 1000);
        }

        Ok(RespValue::ok())
    }
}

/// Extract a simple filter expression from a JSONPath like `$[*][?(@.field == value)].field`.
fn extract_filter_expr(path: &str) -> Option<String> {
    if let Some(start) = path.find("[?(@") {
        let after = &path[start + 4..]; // skip [?(@
        // Find the closing ] of this filter
        let end = after.find(']')?;
        let expr = after[..end].trim_end_matches(')'); // remove trailing )
        Some(expr.to_string())
    } else {
        None
    }
}

/// Evaluate a simple filter expression like `@.field == value` against an object.
fn eval_simple_filter(obj: &serde_json::Map<String, serde_json::Value>, expr: &str) -> bool {
    let expr = expr.trim();
    // Find operator position
    let op_pos = expr.find(|c| c == '=' || c == '!' || c == '>' || c == '<');
    let (field, op, right) = match op_pos {
        Some(pos) => {
            let left = expr[..pos].trim();
            let f = left.trim_start_matches('@').trim_start_matches('.');
            let rest = expr[pos..].trim();
            let op_len = if rest.starts_with("==") || rest.starts_with("!=") { 2 } else { 1 };
            (f, &rest[..op_len], rest[op_len..].trim())
        }
        None => return false,
    };
    let cmp_val = if right.starts_with('\'') {
        let s = right.trim_start_matches('\'').trim_end_matches('\'');
        serde_json::Value::String(s.to_string())
    } else if let Ok(n) = right.parse::<f64>() {
        serde_json::json!(n)
    } else {
        serde_json::Value::String(right.to_string())
    };
    let val = obj.get(field);
    match val {
        Some(v) => match op {
            "==" => json_simple_equal(v, &cmp_val),
            "!=" => !json_simple_equal(v, &cmp_val),
            ">" => json_cmp(v, &cmp_val) == Some(std::cmp::Ordering::Greater),
            "<" => json_cmp(v, &cmp_val) == Some(std::cmp::Ordering::Less),
            ">=" => matches!(json_cmp(v, &cmp_val), Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)),
            "<=" => matches!(json_cmp(v, &cmp_val), Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)),
            _ => false,
        },
        None => false,
    }
}

fn json_simple_equal(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    match (a, b) {
        (serde_json::Value::Number(na), serde_json::Value::Number(nb)) => {
            na.as_f64().zip(nb.as_f64()).map_or(false, |(a,b)| a == b)
        }
        (serde_json::Value::String(sa), serde_json::Value::String(sb)) => sa == sb,
        _ => a == b,
    }
}

fn json_cmp(a: &serde_json::Value, b: &serde_json::Value) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (serde_json::Value::Number(na), serde_json::Value::Number(nb)) => {
            na.as_f64().zip(nb.as_f64()).and_then(|(a,b)| a.partial_cmp(&b))
        }
        (serde_json::Value::String(sa), serde_json::Value::String(sb)) => Some(sa.cmp(sb)),
        _ => None,
    }
}

/// Delete a field at the given JSONPath from the JSON value.
/// Supports simple patterns like `$[*].field` used by TL.KvDoc.
/// Split path into parts, preserving bracket groups.
/// `[?(@.Str == 'test2')].Str` → `["[?(@.Str == 'test2')]", "Str"]`
fn split_path_parts_simple(path: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut bracket_depth = 0i32;

    for ch in path.chars() {
        match ch {
            '[' | '{' | '(' => {
                bracket_depth += 1;
                current.push(ch);
            }
            ']' | '}' | ')' => {
                bracket_depth -= 1;
                current.push(ch);
            }
            '.' => {
                if bracket_depth == 0 {
                    if !current.is_empty() {
                        parts.push(current.clone());
                        current.clear();
                    }
                } else {
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

fn delete_field_at_path(json: &mut JsonValue, path: &str) -> bool {
    let path = path.trim_start_matches('$').trim_start_matches('.');
    eprintln!("[ATOM_DEBUG] delete_field_at_path trimmed='{}' json_is_array={}", path, json.is_array());
    // Use bracket-aware splitting: `[?(@.Str == 'test2')].Str` -> [..., "Str"]
    let parts = split_path_parts_simple(path);
    // Find the field name (last part, strip brackets, skip filter parts)
    let field_part = parts.last().map(|s| s.as_str()).unwrap_or(path);
    let clean = field_part.trim_start_matches('[').trim_end_matches(']');
    // Skip if the last part is a filter expression (no field to delete)
    if clean.starts_with('?') {
        return false;
    }
    // Handle [*] or [?()] prefix: iterate array elements, skip filter parts
    if let Some(first) = parts.first().map(|s| s.as_str()) {
        if first == "[*]" {
            if let JsonValue::Array(arr) = json {
                let mut deleted = false;
                for item in arr.iter_mut() {
                    if let JsonValue::Object(obj) = item {
                        if obj.remove(clean).is_some() {
                            deleted = true;
                        }
                    }
                }
                return deleted;
            }
        }
        // [?()] filter: evaluate filter before deleting from each element
        if first.starts_with("[?(") && first.ends_with(']') {
            let inner = &first[1..first.len()-1]; // strip [ and ]
            let filter_expr = inner.trim_start_matches("?(").trim_end_matches(')').trim();
            if let JsonValue::Array(arr) = json {
                let mut deleted = false;
                for item in arr.iter_mut() {
                    if let JsonValue::Object(obj) = item {
                        if eval_simple_filter(obj, filter_expr) {
                            if obj.remove(clean).is_some() {
                                deleted = true;
                            }
                        }
                    }
                }
                return deleted;
            }
            // Root is a single object — evaluate filter match before deleting
            if let JsonValue::Object(obj) = json {
                if eval_simple_filter(obj, filter_expr) {
                    return obj.remove(clean).is_some();
                }
            }
            return false;
        }
    }
    // Simple field name on object
    if let JsonValue::Object(obj) = json {
        obj.remove(clean).is_some()
    } else {
        false
    }
}
