/// Shared MCP schema normalization utilities.
/// These functions convert MCP tool schemas into OpenAI-compatible format.

use serde_json::Value;

/// Extract the single non-null branch from a oneOf/anyOf option list.
/// Returns `(branch, true)` if exactly one non-null branch exists alongside a null type.
pub fn extract_nullable_branch(options: &[Value]) -> Option<(Value, bool)> {
    let mut non_null_items: Vec<&Value> = Vec::new();
    let mut saw_null = false;

    for option in options {
        if let Some(obj) = option.as_object() {
            if obj.get("type").and_then(|t| t.as_str()) == Some("null") {
                saw_null = true;
                continue;
            }
            non_null_items.push(option);
        } else {
            return None;
        }
    }

    if saw_null && non_null_items.len() == 1 {
        Some((non_null_items[0].clone(), true))
    } else {
        None
    }
}

/// Normalize MCP schemas for OpenAI compatibility:
/// - `type: [string, null]` -> `type: string, nullable: true`
/// - `oneOf`/`anyOf` with a single non-null branch -> merge + `nullable: true`
/// - Recursively normalize `properties` and `items`
/// - Ensure object types have `properties` and `required` fields
pub fn normalize_schema_for_openai(schema: &Value) -> Value {
    let mut result = schema.clone();

    // Handle type field as array: [type, null] -> { type, nullable: true }
    if let Some(arr) = result.get("type").and_then(|t| t.as_array()) {
        let non_null: Vec<&str> = arr
            .iter()
            .filter_map(|v| v.as_str())
            .filter(|s| *s != "null")
            .collect();
        if arr.iter().any(|v| v.as_str() == Some("null")) && non_null.len() == 1 {
            let single_type = non_null[0].to_string();
            if let Some(obj) = result.as_object_mut() {
                obj.insert("type".to_string(), Value::String(single_type));
                obj.insert("nullable".to_string(), Value::Bool(true));
            }
        }
    }

    // Handle oneOf / anyOf: extract single non-null branch, merge, set nullable
    for key in &["oneOf", "anyOf"] {
        if let Some(options) = result.get(*key).and_then(|v| v.as_array()) {
            if let Some((branch, _)) = extract_nullable_branch(options) {
                if let Some(branch_obj) = branch.as_object() {
                    let mut merged: serde_json::Map<String, Value> = serde_json::Map::new();
                    if let Some(result_obj) = result.as_object() {
                        for (k, v) in result_obj {
                            if *key != k {
                                merged.insert(k.clone(), v.clone());
                            }
                        }
                    }
                    for (k, v) in branch_obj {
                        if !merged.contains_key(k) {
                            merged.insert(k.clone(), v.clone());
                        }
                    }
                    merged.insert("nullable".to_string(), Value::Bool(true));
                    result = Value::Object(merged);
                }
                break;
            }
        }
    }

    // Recursively normalize properties
    if let Some(props) = result.get("properties").and_then(|p| p.as_object()) {
        let mut new_props = serde_json::Map::new();
        for (name, prop) in props {
            if prop.is_object() || prop.is_array() {
                new_props.insert(name.clone(), normalize_schema_for_openai(prop));
            } else {
                new_props.insert(name.clone(), prop.clone());
            }
        }
        if let Some(obj) = result.as_object_mut() {
            obj.insert("properties".to_string(), Value::Object(new_props));
        }
    }

    // Recursively normalize items
    if let Some(items) = result.get("items") {
        if items.is_object() || items.is_array() {
            let normalized_items = normalize_schema_for_openai(items);
            if let Some(obj) = result.as_object_mut() {
                obj.insert("items".to_string(), normalized_items);
            }
        }
    }

    // Ensure object types have properties and required fields
    if result.get("type").and_then(|t| t.as_str()) == Some("object") {
        if let Some(obj) = result.as_object_mut() {
            obj.entry("properties").or_insert(Value::Object(serde_json::Map::new()));
            obj.entry("required").or_insert(Value::Array(Vec::new()));
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_normalize_nullable_type() {
        let schema = json!({"type": ["string", "null"]});
        let normalized = normalize_schema_for_openai(&schema);
        assert_eq!(normalized.get("type").and_then(|v| v.as_str()), Some("string"));
        assert_eq!(normalized.get("nullable").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn test_normalize_oneof_nullable() {
        let schema = json!({
            "oneOf": [
                {"type": "null"},
                {"type": "object", "properties": {"path": {"type": "string"}}}
            ]
        });
        let normalized = normalize_schema_for_openai(&schema);
        assert_eq!(normalized.get("nullable").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(normalized.get("type").and_then(|v| v.as_str()), Some("object"));
        assert!(normalized.get("properties").is_some());
        assert!(normalized.get("oneOf").is_none());
    }

    #[test]
    fn test_normalize_anyof_nullable() {
        let schema = json!({
            "anyOf": [{"type": "null"}, {"type": "string"}]
        });
        let normalized = normalize_schema_for_openai(&schema);
        assert_eq!(normalized.get("nullable").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(normalized.get("type").and_then(|v| v.as_str()), Some("string"));
    }

    #[test]
    fn test_normalize_nested_properties() {
        let schema = json!({
            "type": "object",
            "properties": {
                "config": {
                    "type": ["object", "null"],
                    "properties": {"enabled": {"type": "boolean"}}
                }
            }
        });
        let normalized = normalize_schema_for_openai(&schema);
        let config = normalized.get("properties").and_then(|p| p.get("config")).unwrap();
        assert_eq!(config.get("type").and_then(|v| v.as_str()), Some("object"));
        assert_eq!(config.get("nullable").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn test_normalize_object_has_required() {
        let schema = json!({"type": "object", "properties": {"name": {"type": "string"}}});
        let normalized = normalize_schema_for_openai(&schema);
        assert!(normalized.get("required").is_some());
    }

    #[test]
    fn test_normalize_passthrough_simple() {
        let schema = json!({"type": "string", "description": "a simple string"});
        let normalized = normalize_schema_for_openai(&schema);
        assert_eq!(normalized.get("type").and_then(|v| v.as_str()), Some("string"));
    }
}
