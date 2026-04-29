use crate::error::{AikvError, Result};
use crate::protocol::RespValue;
use crate::storage::StorageEngine;
use bytes::Bytes;
use serde_json::{json, Value as JsonValue};

/// JSON command handler
pub struct JsonCommands {
    storage: StorageEngine,
}

impl JsonCommands {
    pub fn new(storage: StorageEngine) -> Self {
        Self {
            storage,
        }
    }

    /// JSON.GET key \[path\]
    pub fn json_get(&self, args: &[Bytes], current_db: usize) -> Result<RespValue> {
        if args.is_empty() {
            return Err(AikvError::WrongArgCount("JSON.GET".to_string()));
        }

        let key = String::from_utf8_lossy(&args[0]).to_string();
        let path = if args.len() > 1 {
            String::from_utf8_lossy(&args[1]).to_string()
        } else {
            "$".to_string()
        };

        match self.storage.get_from_db(current_db, &key)? {
            Some(value) => {
                let json: JsonValue = serde_json::from_slice(&value)?;

                let result = if path == "$" || path == "." {
                    json
                } else {
                    // Simple path extraction (full JSONPath would be more complex)
                    self.extract_json_path(&json, &path)?
                };

                let json_string = serde_json::to_string(&result)?;
                Ok(RespValue::bulk_string(json_string))
            }
            None => Ok(RespValue::null_bulk_string()),
        }
    }

    /// JSON.SET key path value \[NX|XX\]
    pub fn json_set(&self, args: &[Bytes], current_db: usize) -> Result<RespValue> {
        if args.len() < 3 {
            return Err(AikvError::WrongArgCount("JSON.SET".to_string()));
        }

        let key = String::from_utf8_lossy(&args[0]).to_string();
        let path = String::from_utf8_lossy(&args[1]).to_string();
        let value_str = String::from_utf8_lossy(&args[2]).to_string();

        // Parse options
        let mut nx = false;
        let mut xx = false;
        let mut xe = false; // TL.KvDoc: throw when exist (opposite of NX)
        for arg in args.iter().skip(3) {
            let option = String::from_utf8_lossy(arg).to_uppercase();
            match option.as_str() {
                "NX" => nx = true,
                "XX" | "NN" => xx = true, // NN = TL.KvDoc: no-op if not exist
                "XE" => xe = true,
                _ => {}
            }
        }

        // Check conditions
        let exists = self.storage.exists_in_db(current_db, &key)?;
        if xe && exists {
            return Err(AikvError::InvalidArgument(
                "Key already exists (XE)".to_string(),
            ));
        }
        if nx && exists {
            return Ok(RespValue::null_bulk_string());
        }
        if xx && !exists {
            return Ok(RespValue::null_bulk_string());
        }

        // For paths with a filter expression, check if any elements match
        // before applying (TL.KvDoc expects an error when no match).
        if path.contains("[?(@") && !matches!(path.as_str(), "$" | ".") {
            if let Some(ref existing) = self.storage.get_from_db(current_db, &key)? {
                let json_check: JsonValue = serde_json::from_slice(existing)?;
                let matched = self.extract_json_path(&json_check, &path)?;
                match &matched {
                    JsonValue::Array(arr) if arr.is_empty() => {
                        return Err(AikvError::InvalidArgument(
                            "No elements match the where condition".to_string(),
                        ));
                    }
                    JsonValue::Null => {
                        return Err(AikvError::InvalidArgument(
                            "No elements match the where condition".to_string(),
                        ));
                    }
                    _ => {}
                }
            }
        }

        // Parse the new value
        let new_value: JsonValue = serde_json::from_str(&value_str)?;

        let result_json = if path == "$" || path == "." {
            // Root path - replace entire value
            new_value
        } else {
            // Get existing value or create empty object
            let mut json = match self.storage.get_from_db(current_db, &key)? {
                Some(existing) => serde_json::from_slice(&existing)?,
                None => json!({}),
            };

            // Set value at path (simplified)
            self.set_json_path(&mut json, &path, new_value)?;
            json
        };

        let json_bytes = Bytes::from(serde_json::to_vec(&result_json)?);
        self.storage.set_in_db(current_db, key, json_bytes)?;

        Ok(RespValue::ok())
    }

    /// JSON.DEL key \[path\]
    pub fn json_del(&self, args: &[Bytes], current_db: usize) -> Result<RespValue> {
        if args.is_empty() {
            return Err(AikvError::WrongArgCount("JSON.DEL".to_string()));
        }

        let key = String::from_utf8_lossy(&args[0]).to_string();
        let path = if args.len() > 1 {
            String::from_utf8_lossy(&args[1]).to_string()
        } else {
            "$".to_string()
        };

        if path == "$" || path == "." {
            // Delete entire key
            if self.storage.delete_from_db(current_db, &key)? {
                Ok(RespValue::integer(1))
            } else {
                Ok(RespValue::integer(0))
            }
        } else {
            // Delete specific path
            match self.storage.get_from_db(current_db, &key)? {
                Some(value) => {
                    let mut json: JsonValue = serde_json::from_slice(&value)?;

                    if self.delete_json_path(&mut json, &path)? {
                        let json_bytes = Bytes::from(serde_json::to_vec(&json)?);
                        self.storage.set_in_db(current_db, key, json_bytes)?;
                        Ok(RespValue::integer(1))
                    } else {
                        Ok(RespValue::integer(0))
                    }
                }
                None => Ok(RespValue::integer(0)),
            }
        }
    }

    /// JSON.TYPE key \[path\]
    pub fn json_type(&self, args: &[Bytes], current_db: usize) -> Result<RespValue> {
        if args.is_empty() {
            return Err(AikvError::WrongArgCount("JSON.TYPE".to_string()));
        }

        let key = String::from_utf8_lossy(&args[0]).to_string();
        let path = if args.len() > 1 {
            String::from_utf8_lossy(&args[1]).to_string()
        } else {
            "$".to_string()
        };

        match self.storage.get_from_db(current_db, &key)? {
            Some(value) => {
                let json: JsonValue = serde_json::from_slice(&value)?;

                let target = if path == "$" || path == "." {
                    &json
                } else {
                    &self.extract_json_path(&json, &path)?
                };

                let type_name = match target {
                    JsonValue::Null => "null",
                    JsonValue::Bool(_) => "boolean",
                    JsonValue::Number(_) => "number",
                    JsonValue::String(_) => "string",
                    JsonValue::Array(_) => "array",
                    JsonValue::Object(_) => "object",
                };

                Ok(RespValue::simple_string(type_name))
            }
            None => Ok(RespValue::null_bulk_string()),
        }
    }

    /// JSON.STRLEN key \[path\]
    pub fn json_strlen(&self, args: &[Bytes], current_db: usize) -> Result<RespValue> {
        if args.is_empty() {
            return Err(AikvError::WrongArgCount("JSON.STRLEN".to_string()));
        }

        let key = String::from_utf8_lossy(&args[0]).to_string();
        let path = if args.len() > 1 {
            String::from_utf8_lossy(&args[1]).to_string()
        } else {
            "$".to_string()
        };

        match self.storage.get_from_db(current_db, &key)? {
            Some(value) => {
                let json: JsonValue = serde_json::from_slice(&value)?;

                let target = if path == "$" || path == "." {
                    &json
                } else {
                    &self.extract_json_path(&json, &path)?
                };

                if let JsonValue::String(s) = target {
                    Ok(RespValue::integer(s.len() as i64))
                } else {
                    Ok(RespValue::null_bulk_string())
                }
            }
            None => Ok(RespValue::null_bulk_string()),
        }
    }

    /// JSON.ARRLEN key \[path\]
    pub fn json_arrlen(&self, args: &[Bytes], current_db: usize) -> Result<RespValue> {
        if args.is_empty() {
            return Err(AikvError::WrongArgCount("JSON.ARRLEN".to_string()));
        }

        let key = String::from_utf8_lossy(&args[0]).to_string();
        let path = if args.len() > 1 {
            String::from_utf8_lossy(&args[1]).to_string()
        } else {
            "$".to_string()
        };

        match self.storage.get_from_db(current_db, &key)? {
            Some(value) => {
                let json: JsonValue = serde_json::from_slice(&value)?;

                let target = if path == "$" || path == "." {
                    &json
                } else {
                    &self.extract_json_path(&json, &path)?
                };

                if let JsonValue::Array(arr) = target {
                    Ok(RespValue::integer(arr.len() as i64))
                } else {
                    Ok(RespValue::null_bulk_string())
                }
            }
            None => Ok(RespValue::null_bulk_string()),
        }
    }

    /// JSON.NUMINCRBY key path increment
    ///
    /// Increments the numeric value at the specified path by the given amount.
    pub fn json_numincrby(&self, args: &[Bytes], current_db: usize) -> Result<RespValue> {
        if args.len() < 3 {
            return Err(AikvError::WrongArgCount("JSON.NUMINCRBY".to_string()));
        }

        let key = String::from_utf8_lossy(&args[0]).to_string();
        let path = String::from_utf8_lossy(&args[1]).to_string();
        let incr_str = String::from_utf8_lossy(&args[2]).to_string();

        let incr: f64 = incr_str.parse().map_err(|_| {
            AikvError::InvalidArgument(format!("Invalid increment: {}", incr_str))
        })?;

        match self.storage.get_from_db(current_db, &key)? {
            Some(value) => {
                let mut json: JsonValue = serde_json::from_slice(&value)?;
                let path_normalized = if path == "$" || path == "." {
                    return Err(AikvError::InvalidArgument(
                        "Cannot increment root".to_string(),
                    ));
                } else {
                    path.trim_start_matches('$').trim_start_matches('.').to_string()
                };

                // Navigate to the target and increment
                let parts = Self::split_path_parts(&path_normalized);
                self.json_incr_path(&mut json, &parts, incr)?;

                let json_bytes = Bytes::from(serde_json::to_vec(&json)?);
                self.storage.set_in_db(current_db, key, json_bytes)?;

                // Return the new value at path (RedisJSON returns array of results)
                let result = self.extract_json_path(&json, &path)?;
                Ok(RespValue::bulk_string(serde_json::to_string(&result)?))
            }
            None => Ok(RespValue::null_bulk_string()),
        }
    }

    /// JSON.ARRAPPEND key path value [value ...]
    ///
    /// Appends values to the array at the specified path.
    pub fn json_arrappend(&self, args: &[Bytes], current_db: usize) -> Result<RespValue> {
        if args.len() < 3 {
            return Err(AikvError::WrongArgCount("JSON.ARRAPPEND".to_string()));
        }

        let key = String::from_utf8_lossy(&args[0]).to_string();
        let path = String::from_utf8_lossy(&args[1]).to_string();

        // Parse values to append
        let mut values = Vec::new();
        for arg in args.iter().skip(2) {
            let val_str = String::from_utf8_lossy(arg);
            let val: JsonValue = serde_json::from_str(&val_str).map_err(|e| {
                AikvError::InvalidArgument(format!("Invalid JSON value: {}", e))
            })?;
            values.push(val);
        }

        match self.storage.get_from_db(current_db, &key)? {
            Some(value) => {
                let mut json: JsonValue = serde_json::from_slice(&value)?;

                // Navigate to the array and append
                let path_normalized = if path == "$" || path == "." {
                    return Err(AikvError::InvalidArgument(
                        "Cannot append to root".to_string(),
                    ));
                } else {
                    path.trim_start_matches('$').trim_start_matches('.').to_string()
                };

                let parts = Self::split_path_parts(&path_normalized);
                self.json_append_path(&mut json, &parts, &values)?;

                let json_bytes = Bytes::from(serde_json::to_vec(&json)?);
                self.storage.set_in_db(current_db, key, json_bytes)?;

                // RedisJSON returns the new array length
                let result = self.extract_json_path(&json, &path)?;
                if let JsonValue::Array(arr) = &result {
                    Ok(RespValue::integer(arr.len() as i64))
                } else {
                    Ok(RespValue::null_bulk_string())
                }
            }
            None => Ok(RespValue::null_bulk_string()),
        }
    }

    /// JSON.UPDATE key wherePath path value [path value ...] [flag]
    ///
    /// Updates JSON values at specified paths. Unlike JSON.SET, UPDATE requires
    /// the key to exist. Flag "" means error on missing key, "NN" means ignore.
    pub fn json_update(&self, args: &[Bytes], current_db: usize) -> Result<RespValue> {
        if args.len() < 4 {
            return Err(AikvError::WrongArgCount("JSON.UPDATE".to_string()));
        }

        let key = String::from_utf8_lossy(&args[0]).to_string();
        let where_path = String::from_utf8_lossy(&args[1]).to_string();

        // Check key existence
        let exists = self.storage.exists_in_db(current_db, &key)?;
        if !exists {
            // Check flag - last arg may be "" (throw) or "NN" (ignore)
            let last_arg = String::from_utf8_lossy(&args[args.len() - 1]).to_uppercase();
            if last_arg == "NN" {
                return Ok(RespValue::ok());
            }
            return Err(AikvError::InvalidArgument("Key does not exist".to_string()));
        }

        // Check if wherePath matches any elements
        if !where_path.is_empty() && !matches!(where_path.as_str(), "$" | "." | "$[*]") {
            let json_check: JsonValue = serde_json::from_slice(
                &self.storage.get_from_db(current_db, &key)?.unwrap()
            )?;
            let matched = self.extract_json_path(&json_check, &where_path)?;
            match &matched {
                JsonValue::Array(arr) if arr.is_empty() => {
                    let last_arg = String::from_utf8_lossy(&args[args.len() - 1]);
                    if last_arg.to_uppercase() == "NN" { return Ok(RespValue::ok()); }
                    return Err(AikvError::InvalidArgument(
                        "No elements match the where condition".to_string(),
                    ));
                }
                JsonValue::Null => {
                    return Err(AikvError::InvalidArgument(
                        "No elements match the where condition".to_string(),
                    ));
                }
                _ => {}
            }
        }

        let value = self.storage.get_from_db(current_db, &key)?.unwrap();
        let mut json: JsonValue = serde_json::from_slice(&value)?;

        // Parse path-value pairs from args[2..]
        // The last arg may be "" or "NN" flag, skip it
        let mut i = 2;
        let has_flag = {
            let last = String::from_utf8_lossy(&args[args.len() - 1]);
            last.is_empty() || last.to_uppercase() == "NN"
        };
        let end = if has_flag { args.len() - 1 } else { args.len() };

        while i + 1 < end {
            let path_str = String::from_utf8_lossy(&args[i]).to_string();
            let val_str = String::from_utf8_lossy(&args[i + 1]).to_string();
            let val: JsonValue = serde_json::from_str(&val_str).map_err(|e| {
                AikvError::InvalidArgument(format!("Invalid JSON value: {}", e))
            })?;
            self.set_json_path(&mut json, &path_str, val)?;
            i += 2;
        }

        let json_bytes = Bytes::from(serde_json::to_vec(&json)?);
        self.storage.set_in_db(current_db, key, json_bytes)?;

        Ok(RespValue::ok())
    }

    /// JSON.MSET key path value [key path value ...]
    ///
    /// Sets JSON values for multiple keys atomically.
    /// All keys must be in the same slot (checked by the caller in dispatch).
    pub fn json_mset(&self, args: &[Bytes], current_db: usize) -> Result<RespValue> {
        if args.len() < 3 || args.len() % 3 != 0 {
            return Err(AikvError::WrongArgCount("JSON.MSET".to_string()));
        }

        for chunk in args.chunks(3) {
            let key = String::from_utf8_lossy(&chunk[0]).to_string();
            let path = String::from_utf8_lossy(&chunk[1]).to_string();
            let val_str = String::from_utf8_lossy(&chunk[2]).to_string();

            let new_value: JsonValue = serde_json::from_str(&val_str).map_err(|e| {
                AikvError::InvalidArgument(format!("Invalid JSON value: {}", e))
            })?;

            let result_json = if path == "$" || path == "." {
                new_value
            } else {
                let mut json = match self.storage.get_from_db(current_db, &key)? {
                    Some(existing) => serde_json::from_slice(&existing)?,
                    None => json!({}),
                };
                self.set_json_path(&mut json, &path, new_value)?;
                json
            };

            let json_bytes = Bytes::from(serde_json::to_vec(&result_json)?);
            self.storage.set_in_db(current_db, key, json_bytes)?;
        }

        Ok(RespValue::ok())
    }

    /// JSON.OBJLEN key \[path\]
    pub fn json_objlen(&self, args: &[Bytes], current_db: usize) -> Result<RespValue> {
        if args.is_empty() {
            return Err(AikvError::WrongArgCount("JSON.OBJLEN".to_string()));
        }

        let key = String::from_utf8_lossy(&args[0]).to_string();
        let path = if args.len() > 1 {
            String::from_utf8_lossy(&args[1]).to_string()
        } else {
            "$".to_string()
        };

        match self.storage.get_from_db(current_db, &key)? {
            Some(value) => {
                let json: JsonValue = serde_json::from_slice(&value)?;

                let target = if path == "$" || path == "." {
                    &json
                } else {
                    &self.extract_json_path(&json, &path)?
                };

                if let JsonValue::Object(obj) = target {
                    Ok(RespValue::integer(obj.len() as i64))
                } else {
                    Ok(RespValue::null_bulk_string())
                }
            }
            None => Ok(RespValue::null_bulk_string()),
        }
    }

    // Helper methods for path operations (simplified JSONPath)

    fn extract_json_path(&self, json: &JsonValue, path: &str) -> Result<JsonValue> {
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
                let inner = &part[1..part.len()-1];

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
                                    let value = obj.get(*field).cloned().unwrap_or(JsonValue::String(String::new()));
                                    result_array.push(value);
                                }
                            }
                        }
                        current = JsonValue::Array(result_array);
                    } else if let JsonValue::Object(obj) = &current {
                        // Single object - flatten values in order
                        let mut result_array = Vec::new();
                        for field in &fields {
                            let value = obj.get(*field).cloned().unwrap_or(JsonValue::String(String::new()));
                            result_array.push(value);
                        }
                        current = JsonValue::Array(result_array);
                    } else {
                        return Err(AikvError::InvalidArgument(format!(
                            "Cannot traverse non-object at: {}",
                            part
                        )));
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
                        return Err(AikvError::InvalidArgument(format!(
                            "Cannot apply filter on non-array: {}",
                            part
                        )));
                    }
                } else if inner == "*" {
                    // Array wildcard [*] - return all elements
                    if let JsonValue::Array(arr) = &current {
                        current = JsonValue::Array(arr.clone());
                    } else {
                        // If not array, return as single element array
                        current = json!(vec![current]);
                    }
                } else {
                    // Numeric index like [0]
                    if let Ok(idx) = inner.parse::<usize>() {
                        if let JsonValue::Array(arr) = &current {
                            if idx >= arr.len() {
                                return Err(AikvError::InvalidArgument(format!(
                                    "Array index out of bounds: {}",
                                    idx
                                )));
                            }
                            current = arr[idx].clone();
                        } else {
                            return Err(AikvError::InvalidArgument(format!(
                                "Cannot index non-array: {}",
                                part
                            )));
                        }
                    }
                }
            } else {
                // Regular object key access
                if let JsonValue::Object(obj) = &current {
                    current = obj.get(part.as_str()).cloned().ok_or_else(|| {
                        AikvError::InvalidArgument(format!("Path not found: {}", part))
                    })?;
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
                    return Err(AikvError::InvalidArgument(format!(
                        "Cannot traverse non-object at: {}",
                        part
                    )));
                }
            }
        }

        Ok(current)
    }

    /// Split path string into parts, preserving bracket syntax
    /// Supports nested brackets like `[?(@.List[?(@ == '2')])]` by tracking depth.
    fn split_path_parts(path: &str) -> Vec<String> {
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

    /// Navigate JSON path parts and increment the leaf numeric value.
    fn json_incr_path(&self, json: &mut JsonValue, parts: &[String], incr: f64) -> Result<()> {
        // Handle `[*]` at the start: apply to each element
        if let Some(first) = parts.first() {
            if first == "[*]" || first == "*" {
                if let JsonValue::Array(arr) = json {
                    let remaining = &parts[1..];
                    for item in arr.iter_mut() {
                        if remaining.is_empty() {
                            // Increment each element directly
                            if let JsonValue::Number(n) = item {
                                let val = n.as_f64().ok_or_else(|| {
                                    AikvError::InvalidArgument("Not a number".to_string())
                                })?;
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
                                    let expr = &filter_part[1..filter_part.len()-1];
                                    let inner = expr.trim_start_matches("?(").trim_end_matches(')').trim();
                                    if !inner.is_empty() {
                                        let filtered = self.filter_jsonpath(
                                            &[JsonValue::Object(obj.clone())],
                                            &filter_part[1..filter_part.len()-1]
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
                                        let val = n.as_f64().ok_or_else(|| {
                                            AikvError::InvalidArgument("Not a number".to_string())
                                        })?;
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
                return Err(AikvError::InvalidArgument(
                    "Cannot apply [*] on non-array".to_string(),
                ));
            }

            // Handle JSONPath filter as intermediate part in non-[*] path:
            // e.g. `[?(@.IntValue == 1)]` — filter the array in `current`
            if first.starts_with("[?(") && first.ends_with(']') {
                if let JsonValue::Array(arr) = json {
                    let remaining = &parts[1..];
                    if remaining.is_empty() || remaining[0] == "IntValue" || remaining[0] == "LongValue" {
                        // Filter then increment on each matching element's field
                        let _filtered = self.filter_jsonpath(arr, &first[1..first.len()-1])?;
                        let field = remaining.first().map(|s| s.as_str()).unwrap_or("");
                        for item in arr.iter_mut() {
                            if let JsonValue::Object(obj) = item {
                                // Check if this item matches the filter
                                let matches = self.filter_jsonpath(
                                    &[JsonValue::Object(obj.clone())],
                                    &first[1..first.len()-1]
                                )?.len() > 0;
                                if !matches {
                                    continue; // Skip non-matching items
                                }
                                if !field.is_empty() {
                                    if let Some(JsonValue::Number(n)) = obj.get(field) {
                                        let val = n.as_f64().ok_or_else(|| {
                                            AikvError::InvalidArgument("Not a number".to_string())
                                        })?;
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
                return Err(AikvError::InvalidArgument(
                    "Cannot apply filter on non-array".to_string(),
                ));
            }
        }

        let mut current = json;

        for (i, part) in parts.iter().enumerate() {
            if i == parts.len() - 1 {
                if let JsonValue::Object(obj) = current {
                    if let Some(JsonValue::Number(n)) = obj.get(part.as_str()) {
                        let val = n.as_f64().ok_or_else(|| {
                            AikvError::InvalidArgument("Not a number".to_string())
                        })?;
                        let new_val = val + incr;
                        let new_json = if new_val.fract() == 0.0 && n.is_i64() {
                            json!((n.as_i64().unwrap() as f64 + incr) as i64)
                        } else {
                            json!(new_val)
                        };
                        obj.insert(part.clone(), new_json);
                        return Ok(());
                    } else {
                        return Err(AikvError::InvalidArgument(
                            "Path value is not a number".to_string(),
                        ));
                    }
                }
                return Err(AikvError::InvalidArgument(format!(
                    "Path not found: {}",
                    part
                )));
            } else {
                if let JsonValue::Object(obj) = current {
                    current = obj.entry(part.clone()).or_insert_with(|| json!({}));
                } else {
                    return Err(AikvError::InvalidArgument(format!(
                        "Cannot traverse non-object at: {}",
                        part
                    )));
                }
            }
        }

        Ok(())
    }

    /// Navigate to an array at the given path parts and append values.
    fn json_append_path(
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
                            return Err(AikvError::InvalidArgument(
                                "Cannot append to [*] directly".to_string(),
                            ));
                        }
                        if let JsonValue::Object(obj) = first_elem {
                            let target_key = remaining[0].as_str();
                            // Strip bracket syntax if present
                            let clean_key = target_key
                                .trim_start_matches('[')
                                .trim_end_matches(']');
                            if let Some(JsonValue::Array(target_arr)) = obj.get_mut(clean_key) {
                                for v in values {
                                    target_arr.push(v.clone());
                                }
                                return Ok(());
                            }
                            return Err(AikvError::InvalidArgument(format!(
                                "Path value is not an array: {}",
                                clean_key
                            )));
                        }
                    }
                    return Err(AikvError::InvalidArgument(
                        "Cannot access [*] on empty array".to_string(),
                    ));
                }
                return Err(AikvError::InvalidArgument(
                    "Cannot apply [*] on non-array".to_string(),
                ));
            }
        }

        let mut current = json;

        for (i, part) in parts.iter().enumerate() {
            if i == parts.len() - 1 {
                let clean_key = part
                    .trim_start_matches('[')
                    .trim_end_matches(']');
                if let JsonValue::Object(obj) = current {
                    if let Some(JsonValue::Array(arr)) = obj.get_mut(clean_key) {
                        for v in values {
                            arr.push(v.clone());
                        }
                        return Ok(());
                    } else {
                        return Err(AikvError::InvalidArgument(
                            "Path value is not an array".to_string(),
                        ));
                    }
                }
                return Err(AikvError::InvalidArgument(format!(
                    "Path not found: {}",
                    part
                )));
            } else {
                if let JsonValue::Object(obj) = current {
                    current = obj.entry(part.clone()).or_insert_with(|| json!({}));
                } else {
                    return Err(AikvError::InvalidArgument(format!(
                        "Cannot traverse non-object at: {}",
                        part
                    )));
                }
            }
        }

        Err(AikvError::InvalidArgument("Empty path".to_string()))
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

        let result: Vec<JsonValue> = arr.iter().filter(|item| {
            let matched = self.eval_filter_expr(item, inner).unwrap_or(false);
            if !matched {
            }
            matched
        }).cloned().collect();

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
            let inner = expr[1..].trim();
            // If parenthesized: !(expr)
            if inner.starts_with('(') && inner.ends_with(')') {
                let result = self.eval_filter_expr(item, &inner[1..inner.len()-1])?;
                return Ok(!result);
            }
            let result = self.eval_single_condition(item, inner)?;
            return Ok(!result);
        }

        // Split on || at top level
        let or_parts = split_top_level(expr, "||");
        for or_part in or_parts {
            let or_part = or_part.trim();
            if or_part.is_empty() { continue; }

            // Split on && at top level
            let and_parts = split_top_level(or_part, "&&");
            let all_true = and_parts.iter().all(|part| {
                let part = part.trim();
                if part.is_empty() { return true; }
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
            let inner = expr[1..].trim();
            // If parenthesized: !(expr)
            if inner.starts_with('(') && inner.ends_with(')') {
                let result = self.eval_single_condition(item, &inner[1..inner.len()-1])?;
                return Ok(!result);
            }
            let result = self.eval_single_condition(item, inner)?;
            return Ok(!result);
        }

        // Handle nested filter pattern: @.field[?(@ ...)] — Any() check
        if let Some(bracket_pos) = expr.find("[?(") {
            let field_part = expr[..bracket_pos].trim();
            let field = field_part.trim_start_matches('@').trim_start_matches('.');
            let inner_expr = &expr[bracket_pos..]; // e.g. [?(@ == 'test11')]


            // Get the array field value from the item
            // Handle dotted paths (e.g., "InnerDocument.StrList")
            let arr_val = if field.is_empty() {
                item.clone()
            } else if field.contains('.') || field.contains('[') {
                let parts = Self::split_path_parts(field);
                let mut current_val = item;
                let mut found = JsonValue::Null;
                for (i, part) in parts.iter().enumerate() {
                    if let JsonValue::Object(obj) = current_val {
                        if i == parts.len() - 1 {
                            found = obj.get(part.as_str()).cloned().unwrap_or(JsonValue::Null);
                        } else {
                            // Handle array indexing like [0]
                            if part.starts_with('[') && part.ends_with(']') {
                                let inner = &part[1..part.len()-1];
                                if let Ok(idx) = inner.parse::<usize>() {
                                    if let JsonValue::Array(arr) = current_val {
                                        current_val = arr.get(idx).unwrap_or(&JsonValue::Null);
                                        continue;
                                    }
                                }
                            }
                            current_val = obj.get(part.as_str()).unwrap_or(&JsonValue::Null);
                        }
                    } else if let JsonValue::Array(arr) = current_val {
                        // Array indexing [N]
                        if part.starts_with('[') && part.ends_with(']') {
                            let inner = &part[1..part.len()-1];
                            if let Ok(idx) = inner.parse::<usize>() {
                                if i == parts.len() - 1 {
                                    found = arr.get(idx).cloned().unwrap_or(JsonValue::Null);
                                } else {
                                    current_val = arr.get(idx).unwrap_or(&JsonValue::Null);
                                }
                            } else {
                                found = JsonValue::Null;
                                break;
                            }
                        } else {
                            found = JsonValue::Null;
                            break;
                        }
                    } else {
                        found = JsonValue::Null;
                        break;
                    }
                }
                found
            } else if let JsonValue::Object(obj) = item {
                obj.get(field).cloned().unwrap_or(JsonValue::Null)
            } else {
                return Ok(false);
            };


            return if let JsonValue::Array(arr) = &arr_val {
                // Evaluate sub-filter recursively: any match means true
                // Strip outer []: [?(@ == 'test11')] -> ?(@ == 'test11')
                let stripped = inner_expr.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(inner_expr);
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
            let field = left.trim_start_matches('@').trim_start_matches('.');


            // Parse /pattern/flags
            if right.starts_with('/') {
                let closing = right[1..].rfind('/');
                if let Some(end) = closing {
                    let pattern_end = 1 + end;
                    let pattern_str = &right[1..pattern_end];
                    let flags = &right[pattern_end + 1..];
                    let case_insensitive = flags.contains('i');

                    // Extract search term from .*value.* pattern
                    let search_term = if pattern_str.starts_with(".*") && pattern_str.ends_with(".*") {
                        &pattern_str[2..pattern_str.len()-2]
                    } else if pattern_str.starts_with(".*") {
                        &pattern_str[2..]
                    } else if pattern_str.ends_with(".*") {
                        &pattern_str[..pattern_str.len()-2]
                    } else {
                        pattern_str
                    };

                    let val = if field.is_empty() {
                        Some(item.to_string())
                    } else if field.contains('.') || field.contains('[') {
                        let parts = Self::split_path_parts(field);
                        let mut current_val = item;
                        let mut found_val = None;
                        for (i, part) in parts.iter().enumerate() {
                            if let JsonValue::Object(obj) = current_val {
                                if i == parts.len() - 1 {
                                    found_val = obj.get(part.as_str()).and_then(|v| v.as_str()).map(|s| s.to_string());
                                } else {
                                    if part.starts_with('[') && part.ends_with(']') {
                                        let inner = &part[1..part.len()-1];
                                        if let Ok(idx) = inner.parse::<usize>() {
                                            if let JsonValue::Array(arr) = current_val {
                                                current_val = arr.get(idx).unwrap_or(&JsonValue::Null);
                                                continue;
                                            }
                                        }
                                    }
                                    current_val = obj.get(part.as_str()).unwrap_or(&JsonValue::Null);
                                }
                            } else {
                                break;
                            }
                        }
                        found_val
                    } else if let JsonValue::Object(obj) = item {
                        obj.get(field).and_then(|v| v.as_str()).map(|s| s.to_string())
                    } else {
                        None
                    };

                    if let Some(s) = val {
                        let match_str = if case_insensitive { s.to_lowercase() } else { s };
                        let search_str = if case_insensitive { search_term.to_lowercase().to_string() } else { search_term.to_string() };
                        let matched = match_str.contains(&search_str);
                        return Ok(matched);
                    }
                }
            }
            return Ok(false);
        }

        // Standard comparison: @.field op value
        let op_pos = expr.find(|c| c == '=' || c == '!' || c == '>' || c == '<');
        if let Some(pos) = op_pos {
            let left_raw = expr[..pos].trim();
            let rest = expr[pos..].trim();
            let op_len = if rest.starts_with("==") || rest.starts_with("!=")
                || rest.starts_with(">=") || rest.starts_with("<=")
            { 2 } else { 1 };
            let op_str = &rest[..op_len];
            let right_raw = rest[op_len..].trim();

            let field = left_raw.trim_start_matches('@').trim_start_matches('.');

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
            } else if field.contains('.') || field.contains('[') {
                // Traverse dotted path through object hierarchy
                let parts = Self::split_path_parts(field);
                let mut current_val = item;
                let mut found = JsonValue::Null;
                for (i, part) in parts.iter().enumerate() {
                    if let JsonValue::Object(obj) = current_val {
                        if i == parts.len() - 1 {
                            found = obj.get(part.as_str()).cloned().unwrap_or(JsonValue::Null);
                        } else {
                            if part.starts_with('[') && part.ends_with(']') {
                                let inner = &part[1..part.len()-1];
                                if let Ok(idx) = inner.parse::<usize>() {
                                    if let JsonValue::Array(arr) = current_val {
                                        current_val = arr.get(idx).unwrap_or(&JsonValue::Null);
                                        continue;
                                    }
                                }
                            }
                            current_val = obj.get(part.as_str()).unwrap_or(&JsonValue::Null);
                        }
                    } else if let JsonValue::Array(arr) = current_val {
                        if part.starts_with('[') && part.ends_with(']') {
                            let inner = &part[1..part.len()-1];
                            if let Ok(idx) = inner.parse::<usize>() {
                                if i == parts.len() - 1 {
                                    found = arr.get(idx).cloned().unwrap_or(JsonValue::Null);
                                } else {
                                    current_val = arr.get(idx).unwrap_or(&JsonValue::Null);
                                }
                            } else {
                                found = JsonValue::Null;
                                break;
                            }
                        } else {
                            found = JsonValue::Null;
                            break;
                        }
                    } else {
                        found = JsonValue::Null;
                        break;
                    }
                }
                found
            } else if let JsonValue::Object(obj) = item {
                obj.get(field).cloned().unwrap_or(JsonValue::Null)
            } else {
                return Ok(false);
            };

            return Ok(match op_str {
                "==" => json_equal(&val, &cmp_value),
                "!=" => !json_equal(&val, &cmp_value),
                ">" => json_compare(&val, &cmp_value) == Some(std::cmp::Ordering::Greater),
                "<" => json_compare(&val, &cmp_value) == Some(std::cmp::Ordering::Less),
                ">=" => matches!(json_compare(&val, &cmp_value), Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)),
                "<=" => matches!(json_compare(&val, &cmp_value), Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)),
                _ => false,
            });
        }

        // Bare field truthiness: @.BoolValue
        let field = expr.trim_start_matches('@').trim_start_matches('.');
        // Handle dotted field paths (e.g., "InnerDocument.BoolValue")
        let val = if field.is_empty() {
            item.clone()
        } else if field.contains('.') || field.contains('[') {
            let parts = Self::split_path_parts(field);
            let mut current_val = item;
            let mut found = JsonValue::Null;
            for (i, part) in parts.iter().enumerate() {
                if let JsonValue::Object(obj) = current_val {
                    if i == parts.len() - 1 {
                        found = obj.get(part.as_str()).cloned().unwrap_or(JsonValue::Null);
                    } else {
                        if part.starts_with('[') && part.ends_with(']') {
                            let inner = &part[1..part.len()-1];
                            if let Ok(idx) = inner.parse::<usize>() {
                                if let JsonValue::Array(arr) = current_val {
                                    current_val = arr.get(idx).unwrap_or(&JsonValue::Null);
                                    continue;
                                }
                            }
                        }
                        current_val = obj.get(part.as_str()).unwrap_or(&JsonValue::Null);
                    }
                } else if let JsonValue::Array(arr) = current_val {
                    if part.starts_with('[') && part.ends_with(']') {
                        let inner = &part[1..part.len()-1];
                        if let Ok(idx) = inner.parse::<usize>() {
                            if i == parts.len() - 1 {
                                found = arr.get(idx).cloned().unwrap_or(JsonValue::Null);
                            } else {
                                current_val = arr.get(idx).unwrap_or(&JsonValue::Null);
                            }
                        } else {
                            found = JsonValue::Null;
                            break;
                        }
                    } else {
                        found = JsonValue::Null;
                        break;
                    }
                } else {
                    found = JsonValue::Null;
                    break;
                }
            }
            found
        } else if let JsonValue::Object(obj) = item {
            obj.get(field).cloned().unwrap_or(JsonValue::Null)
        } else {
            return Ok(false);
        };
        let result = val != JsonValue::Null && val != JsonValue::Bool(false);
        Ok(result)
    }

    /// Filter out elements matching a filter expression from an array.
    /// Returns only elements that do NOT match (for deletion).
    /// Unified with `eval_single_condition` — reuses the same matching logic.
    fn filter_arr_except(&self, arr: &[JsonValue], filter_str: &str) -> Vec<JsonValue> {
        let expr = &filter_str[1..filter_str.len()-1]; // strip []
        let inner = expr.trim_start_matches("?(").trim_end_matches(')').trim();
        arr.iter().filter(|elem| {
            match self.eval_single_condition(elem, inner) {
                Ok(true) => false,  // matches filter → remove it
                _ => true,          // doesn't match or error → keep it
            }
        }).cloned().collect()
    }

    fn set_json_path(&self, json: &mut JsonValue, path: &str, value: JsonValue) -> Result<()> {
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
                        self.set_json_path(item, &remaining_path, value.clone())?;
                    }
                    return Ok(());
                }
                // [?()] filter: apply remaining path to matching elements
                if first.starts_with("[?(") && first.ends_with(']') {
                    let remaining = &parts[1..];
                    let remaining_path = remaining.join(".");
                    // Evaluate filter for each element
                    for item in arr.iter_mut() {
                        let matches = self.filter_jsonpath(
                            &[item.clone()],
                            &first[1..first.len()-1]
                        )?.len() > 0;
                        if matches {
                            if remaining_path.is_empty() {
                                *item = value.clone();
                            } else {
                                self.set_json_path(item, &remaining_path, value.clone())?;
                            }
                        }
                    }
                    return Ok(());
                }
                // Numeric index: [N]
                if first.starts_with('[') && first.ends_with(']') {
                    let inner = &first[1..first.len()-1];
                    if let Ok(idx) = inner.parse::<usize>() {
                        if idx < arr.len() {
                            let remaining = &parts[1..];
                            let remaining_path = remaining.join(".");
                            if remaining_path.is_empty() {
                                arr[idx] = value;
                            } else {
                                self.set_json_path(&mut arr[idx], &remaining_path, value)?;
                            }
                            return Ok(());
                        }
                        return Err(AikvError::InvalidArgument(format!(
                            "Array index out of bounds: {}", idx
                        )));
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
                        self.set_json_path(item, &remaining_path, value.clone())?;
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
                    let remaining = &parts_str[i+1..];
                    let remaining_path = remaining.join(".");
                    for item in arr.iter_mut() {
                        let matches = self.filter_jsonpath(
                            &[item.clone()],
                            &part[1..part.len()-1]
                        )?.len() > 0;
                        if matches {
                            if remaining_path.is_empty() {
                                *item = value.clone();
                            } else {
                                self.set_json_path(item, &remaining_path, value.clone())?;
                            }
                        }
                    }
                    return Ok(());
                }
                break;
            }

            // Handle [N] index on array
            if part.starts_with('[') && part.ends_with(']') {
                let inner = &part[1..part.len()-1];
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

    fn delete_json_path(&self, json: &mut JsonValue, path: &str) -> Result<bool> {
        // Remove leading $ or .
        let path = path.trim_start_matches('$').trim_start_matches('.');

        if path.is_empty() {
            return Ok(false);
        }

        // Use split_path_parts to handle [*] syntax
        let parts = Self::split_path_parts(path);

        if let Some(first) = parts.first() {
            // Handle [$.*] wildcard at the start on array root
            if first == "[*]" || first == "*" {
                if let JsonValue::Array(arr) = json {
                    let remaining: Vec<&String> = parts[1..].iter()
                        .filter(|p| !p.starts_with("[?("))
                        .collect();
                    let filter_part: Option<&String> = parts[1..].iter()
                        .find(|p| p.starts_with("[?(") && !p[2..].contains("[?("));
                    if remaining.is_empty() && filter_part.is_none() {
                        return Ok(false);
                    }
                    let mut deleted = false;
                    for item in arr.iter_mut() {
                        if let JsonValue::Object(obj) = item {
                            let mut current = obj;
                            let mut traversal_failed = false;
                            for (i, part) in remaining.iter().enumerate() {
                                let field = part.trim_start_matches('[').trim_end_matches(']');
                                if i == remaining.len() - 1 && filter_part.is_none() {
                                    if current.remove(field).is_some() {
                                        deleted = true;
                                    }
                                } else if i < remaining.len() - 1 {
                                    if let Some(next) = current.get_mut(field) {
                                        if let JsonValue::Object(obj_ref) = next {
                                            current = obj_ref;
                                        } else if let JsonValue::Array(arr_ref) = next {
                                            if let Some(filter_str) = filter_part {
                                                let kept = self.filter_arr_except(arr_ref, filter_str);
                                                *arr_ref = kept;
                                                deleted = true;
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
                                    if let Some(JsonValue::Array(arr_ref)) = current.get_mut(field) {
                                        if let Some(filter_str) = filter_part {
                                            let kept = self.filter_arr_except(arr_ref, filter_str);
                                            *arr_ref = kept;
                                            deleted = true;
                                        }
                                    }
                                }
                            }
                            if traversal_failed { continue; }
                        }
                    }
                    return Ok(deleted);
                }
                return Ok(false);
            }

            // Handle [?()] filter at the start on array root
            if first.starts_with("[?(") && first.ends_with(']') {
                if let JsonValue::Array(arr) = json {
                    let remaining: Vec<String> = parts[1..].iter().cloned().collect();
                    let remaining_path = remaining.join(".");
                    let mut deleted = false;
                    for item in arr.iter_mut() {
                        let matches = self.filter_jsonpath(
                            &[item.clone()],
                            &first[1..first.len()-1]
                        )?.len() > 0;
                        if matches {
                            if remaining_path.is_empty() {
                                deleted = true;
                            } else {
                                if self.delete_json_path(item, &remaining_path)? {
                                    deleted = true;
                                }
                            }
                        }
                    }
                    return Ok(deleted);
                }
                return Ok(false);
            }
        }

        let parts_str: Vec<&str> = parts.iter().map(|s| s.as_str()).collect();
        let mut current = json;

        for (i, part) in parts_str.iter().enumerate() {
            let is_last = i == parts_str.len() - 1;

            // Handle [?()] filter on array: apply to matching elements
            if part.starts_with("[?(") && part.ends_with(']') {
                if let JsonValue::Array(arr) = current {
                    let remaining = &parts_str[i+1..];
                    if remaining.is_empty() {
                        // Filter with no remaining path: remove matching elements
                        let orig_len = arr.len();
                        arr.retain(|item| {
                            self.filter_jsonpath(
                                &[item.clone()],
                                &part[1..part.len()-1]
                            ).map(|r| r.is_empty()).unwrap_or(true)
                        });
                        return Ok(arr.len() < orig_len);
                    }
                    let remaining_path = remaining.join(".");
                    let mut deleted = false;
                    for item in arr.iter_mut() {
                        let matches = self.filter_jsonpath(
                            &[item.clone()],
                            &part[1..part.len()-1]
                        )?.len() > 0;
                        if matches {
                            if self.delete_json_path(item, &remaining_path)? {
                                deleted = true;
                            }
                        }
                    }
                    return Ok(deleted);
                }
                return Ok(false);
            }

            // Handle [N] index on array
            if part.starts_with('[') && part.ends_with(']') {
                let inner = &part[1..part.len()-1];
                if let Ok(idx) = inner.parse::<usize>() {
                    if let JsonValue::Array(arr) = current {
                        if is_last {
                            if idx < arr.len() {
                                arr.remove(idx);
                                return Ok(true);
                            }
                            return Ok(false);
                        } else {
                            if idx < arr.len() {
                                current = &mut arr[idx];
                                continue;
                            }
                            return Ok(false);
                        }
                    }
                }
                return Ok(false);
            }

            // Standard object field access
            if is_last {
                if let JsonValue::Object(obj) = current {
                    return Ok(obj.remove(*part).is_some());
                }
                return Ok(false);
            } else {
                if let JsonValue::Object(obj) = current {
                    if let Some(next) = obj.get_mut(*part) {
                        current = next;
                    } else {
                        return Ok(false);
                    }
                } else {
                    return Ok(false);
                }
            }
        }

        Ok(false)
    }
}

/// Split a logical expression by `delim` at the top level (outside quotes and brackets).
fn split_top_level<'a>(expr: &'a str, delim: &str) -> Vec<&'a str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut bracket_depth = 0i32;
    let mut i = 0;
    let bytes = expr.as_bytes();
    while i < bytes.len() {
        match bytes[i] {
            b'\'' => if !in_double_quote { in_single_quote = !in_single_quote; }
            b'"' => if !in_single_quote { in_double_quote = !in_double_quote; }
            b'[' | b'{' | b'(' => if !in_single_quote && !in_double_quote { bracket_depth += 1; }
            b']' | b'}' | b')' => if !in_single_quote && !in_double_quote { bracket_depth -= 1; }
            _ => {}
        }

        if !in_single_quote && !in_double_quote && bracket_depth == 0 {
            if bytes[i..].starts_with(delim.as_bytes()) {
                parts.push(&expr[start..i]);
                i += delim.len();
                start = i;
                continue;
            }
        }
        i += 1;
    }
    if start <= expr.len() {
        parts.push(&expr[start..]);
    }
    parts
}

/// Compare two JSON values for equality, handling type coercion.
fn json_equal(a: &JsonValue, b: &JsonValue) -> bool {
    match (a, b) {
        (JsonValue::Number(na), JsonValue::Number(nb)) => {
            na.as_f64().map_or(false, |a| nb.as_f64().map_or(false, |b| (a - b).abs() < f64::EPSILON))
        }
        (JsonValue::String(sa), JsonValue::String(sb)) => sa == sb,
        (JsonValue::Bool(ba), JsonValue::Bool(bb)) => ba == bb,
        // Number vs string comparison (coercion)
        (JsonValue::Number(n), JsonValue::String(s)) | (JsonValue::String(s), JsonValue::Number(n)) => {
            if let Ok(nv) = s.parse::<f64>() {
                n.as_f64().map_or(false, |a| (a - nv).abs() < f64::EPSILON)
            } else {
                false
            }
        }
        _ => a == b,
    }
}

/// Compare two JSON values, returning Ordering when comparable.
fn json_compare(a: &JsonValue, b: &JsonValue) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (JsonValue::Number(na), JsonValue::Number(nb)) => {
            na.as_f64().zip(nb.as_f64()).and_then(|(a, b)| a.partial_cmp(&b))
        }
        (JsonValue::String(sa), JsonValue::String(sb)) => Some(sa.cmp(sb)),
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
    use crate::storage::StorageEngine;

    fn setup() -> JsonCommands {
        JsonCommands::new(StorageEngine::new_memory(16))
    }

    #[test]
    fn test_json_set_get() {
        let cmd = setup();

        let json_str = r#"{"name":"John","age":30}"#;
        cmd.json_set(
            &[Bytes::from("user"), Bytes::from("$"), Bytes::from(json_str)],
            0,
        )
        .unwrap();

        let result = cmd.json_get(&[Bytes::from("user")], 0).unwrap();
        if let RespValue::BulkString(Some(data)) = result {
            let json: JsonValue = serde_json::from_slice(&data).unwrap();
            assert_eq!(json["name"], "John");
            assert_eq!(json["age"], 30);
        } else {
            panic!("Expected bulk string");
        }
    }

    #[test]
    fn test_json_type() {
        let cmd = setup();

        cmd.json_set(
            &[
                Bytes::from("user"),
                Bytes::from("$"),
                Bytes::from(r#"{"name":"John","age":30,"active":true}"#),
            ],
            0,
        )
        .unwrap();

        let result = cmd
            .json_type(&[Bytes::from("user"), Bytes::from("$.name")], 0)
            .unwrap();
        assert_eq!(result, RespValue::simple_string("string"));

        let result = cmd
            .json_type(&[Bytes::from("user"), Bytes::from("$.age")], 0)
            .unwrap();
        assert_eq!(result, RespValue::simple_string("number"));
    }

    #[test]
    fn test_json_arrlen() {
        let cmd = setup();

        cmd.json_set(
            &[
                Bytes::from("arr"),
                Bytes::from("$"),
                Bytes::from("[1,2,3,4,5]"),
            ],
            0,
        )
        .unwrap();

        let result = cmd.json_arrlen(&[Bytes::from("arr")], 0).unwrap();
        assert_eq!(result, RespValue::integer(5));
    }

    #[test]
    fn test_json_objlen() {
        let cmd = setup();

        cmd.json_set(
            &[
                Bytes::from("user"),
                Bytes::from("$"),
                Bytes::from(r#"{"name":"John","age":30}"#),
            ],
            0,
        )
        .unwrap();

        let result = cmd.json_objlen(&[Bytes::from("user")], 0).unwrap();
        assert_eq!(result, RespValue::integer(2));
    }
}
