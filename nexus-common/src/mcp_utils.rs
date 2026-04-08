//! MCP schema normalization for OpenAI function calling compatibility.

use serde_json::{Map, Value};

/// Extract the non-null branch from a oneOf/anyOf array that includes a null type.
pub fn extract_nullable_branch(options: &[Value]) -> Option<(Value, bool)> {
    let non_null: Vec<&Value> = options
        .iter()
        .filter(|v| v.get("type").and_then(Value::as_str) != Some("null"))
        .collect();
    let has_null = options
        .iter()
        .any(|v| v.get("type").and_then(Value::as_str) == Some("null"));
    if non_null.len() == 1 && has_null {
        Some((non_null[0].clone(), true))
    } else {
        None
    }
}

/// Normalize an MCP tool schema to be compatible with OpenAI function calling.
pub fn normalize_schema_for_openai(schema: &Value) -> Value {
    let Some(obj) = schema.as_object() else {
        return schema.clone();
    };
    let mut result = Map::new();

    // Handle type: ["string", "null"] -> type: "string", nullable: true
    if let Some(type_val) = obj.get("type") {
        if let Some(arr) = type_val.as_array() {
            let non_null: Vec<&Value> =
                arr.iter().filter(|v| v.as_str() != Some("null")).collect();
            let has_null = arr.iter().any(|v| v.as_str() == Some("null"));
            if non_null.len() == 1 {
                result.insert("type".into(), non_null[0].clone());
                if has_null {
                    result.insert("nullable".into(), Value::Bool(true));
                }
            } else {
                result.insert("type".into(), type_val.clone());
            }
        } else {
            result.insert("type".into(), type_val.clone());
        }
    }

    // Handle oneOf/anyOf with single non-null branch
    for key in &["oneOf", "anyOf"] {
        if let Some(Value::Array(options)) = obj.get(*key)
            && let Some((branch, is_nullable)) = extract_nullable_branch(options)
        {
            let normalized = normalize_schema_for_openai(&branch);
            if let Some(branch_obj) = normalized.as_object() {
                for (k, v) in branch_obj {
                    result.insert(k.clone(), v.clone());
                }
            }
            if is_nullable {
                result.insert("nullable".into(), Value::Bool(true));
            }
            for (k, v) in obj {
                if k != *key && !result.contains_key(k) {
                    result.insert(k.clone(), v.clone());
                }
            }
            return Value::Object(result);
        }
    }

    // Recursively normalize properties
    if let Some(Value::Object(props)) = obj.get("properties") {
        let mut np = Map::new();
        for (k, v) in props {
            np.insert(k.clone(), normalize_schema_for_openai(v));
        }
        result.insert("properties".into(), Value::Object(np));
    }

    // Recursively normalize items
    if let Some(items) = obj.get("items") {
        result.insert("items".into(), normalize_schema_for_openai(items));
    }

    // Ensure object types have properties and required
    if result.get("type").and_then(Value::as_str) == Some("object") {
        if !result.contains_key("properties") {
            result.insert("properties".into(), Value::Object(Map::new()));
        }
        if !result.contains_key("required") {
            result.insert("required".into(), Value::Array(vec![]));
        }
    }

    // Copy remaining keys
    for (k, v) in obj {
        if !result.contains_key(k) {
            result.insert(k.clone(), v.clone());
        }
    }

    Value::Object(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_normalize_nullable_type() {
        let s = json!({"type": ["string", "null"]});
        let r = normalize_schema_for_openai(&s);
        assert_eq!(r["type"], "string");
        assert_eq!(r["nullable"], true);
    }

    #[test]
    fn test_normalize_oneof_nullable() {
        let s = json!({"oneOf": [{"type": "string"}, {"type": "null"}]});
        let r = normalize_schema_for_openai(&s);
        assert_eq!(r["type"], "string");
        assert_eq!(r["nullable"], true);
    }

    #[test]
    fn test_normalize_anyof_nullable() {
        let s = json!({"anyOf": [{"type": "integer"}, {"type": "null"}]});
        let r = normalize_schema_for_openai(&s);
        assert_eq!(r["type"], "integer");
        assert_eq!(r["nullable"], true);
    }

    #[test]
    fn test_normalize_nested_properties() {
        let s =
            json!({"type": "object", "properties": {"name": {"type": ["string", "null"]}}});
        let r = normalize_schema_for_openai(&s);
        assert_eq!(r["properties"]["name"]["type"], "string");
        assert_eq!(r["properties"]["name"]["nullable"], true);
    }

    #[test]
    fn test_normalize_object_has_required() {
        let s = json!({"type": "object"});
        let r = normalize_schema_for_openai(&s);
        assert!(r.get("properties").is_some());
        assert!(r.get("required").is_some());
    }

    #[test]
    fn test_normalize_passthrough_simple() {
        let s = json!({"type": "string", "description": "A name"});
        let r = normalize_schema_for_openai(&s);
        assert_eq!(r["type"], "string");
        assert!(r.get("nullable").is_none());
    }
}
