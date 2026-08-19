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

#[test]
fn test_jsonpath_root_path_returns_whole_document() {
    let doc = serde_json::json!({"a": 1, "b": [2]});
    assert_eq!(JsonPathEngine.extract(&doc, "$").unwrap(), doc);
    assert_eq!(JsonPathEngine.extract(&doc, ".").unwrap(), doc);
}

#[test]
fn test_jsonpath_root_array_wildcard() {
    let doc = serde_json::json!([1, 2, 3]);
    assert_eq!(JsonPathEngine.extract(&doc, "$[*]").unwrap(), doc);
}

#[test]
fn test_jsonpath_incr_numeric_field() {
    let mut doc = serde_json::json!({"n": 1});
    let parts = JsonPathEngine::split_path_parts("n");
    JsonPathEngine.incr(&mut doc, &parts, 2.0).unwrap();
    assert_eq!(doc["n"], serde_json::json!(3));
}

#[test]
fn test_jsonpath_append_to_object_array() {
    let mut doc = serde_json::json!({"items": [1]});
    let parts = JsonPathEngine::split_path_parts("items");
    JsonPathEngine
        .append(&mut doc, &parts, &[serde_json::json!(2)])
        .unwrap();
    assert_eq!(doc["items"], serde_json::json!([1, 2]));
}
