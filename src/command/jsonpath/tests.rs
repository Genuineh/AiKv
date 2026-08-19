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
