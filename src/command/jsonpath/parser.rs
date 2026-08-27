use super::JsonPathEngine;

impl JsonPathEngine {
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
}
