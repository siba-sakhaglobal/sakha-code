//! Minimal manual JSON-schema validator for tool input schemas.
//!
//! We deliberately do not pull in a JSON-schema crate (not in the
//! pre-declared workspace dependency set). Tool input schemas used in this
//! crate are a small, well-known subset of JSON Schema: `type: object` with
//! `properties` (each a `{"type": ...}`, optionally `"enum"`), and a
//! `required` array. This module validates a `serde_json::Value` against
//! that subset, which is all the built-in tools need.

use sakha_core::{SakhaError, SakhaResult};

/// Validates `value` against `schema`, returning a descriptive
/// `SakhaError::invalid_input` on the first mismatch found.
///
/// Supported schema shape:
/// ```json
/// {
///   "type": "object",
///   "properties": {
///     "path": {"type": "string"},
///     "count": {"type": "integer"}
///   },
///   "required": ["path"]
/// }
/// ```
pub fn validate_against_schema(
    module: &str,
    value: &serde_json::Value,
    schema: &serde_json::Value,
) -> SakhaResult<()> {
    let schema_type = schema.get("type").and_then(|t| t.as_str());

    if schema_type == Some("object") || schema.get("properties").is_some() {
        let obj = value.as_object().ok_or_else(|| {
            SakhaError::invalid_input(module, "expected a JSON object for tool input")
        })?;

        if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
            for req in required {
                if let Some(name) = req.as_str() {
                    if !obj.contains_key(name) {
                        return Err(SakhaError::invalid_input(
                            module,
                            format!("missing required field: {name}"),
                        ));
                    }
                }
            }
        }

        if let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) {
            for (key, field_schema) in properties {
                if let Some(field_value) = obj.get(key) {
                    validate_field(module, key, field_value, field_schema)?;
                }
            }
        }
    }

    Ok(())
}

fn validate_field(
    module: &str,
    field_name: &str,
    value: &serde_json::Value,
    schema: &serde_json::Value,
) -> SakhaResult<()> {
    if let Some(expected_type) = schema.get("type").and_then(|t| t.as_str()) {
        let matches = match expected_type {
            "string" => value.is_string(),
            "integer" => value.is_i64() || value.is_u64(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "array" => value.is_array(),
            "object" => value.is_object(),
            "null" => value.is_null(),
            _ => true,
        };
        if !matches {
            return Err(SakhaError::invalid_input(
                module,
                format!(
                    "field '{field_name}' expected type '{expected_type}', got {}",
                    type_name(value)
                ),
            ));
        }
    }

    if let Some(enum_values) = schema.get("enum").and_then(|e| e.as_array()) {
        if !enum_values.contains(value) {
            return Err(SakhaError::invalid_input(
                module,
                format!("field '{field_name}' is not one of the allowed enum values"),
            ));
        }
    }

    Ok(())
}

fn type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "count": {"type": "integer"}
            },
            "required": ["path"]
        })
    }

    #[test]
    fn accepts_valid_input() {
        let result = validate_against_schema("test", &json!({"path": "a.txt"}), &schema());
        assert!(result.is_ok());
    }

    #[test]
    fn rejects_missing_required_field() {
        let result = validate_against_schema("test", &json!({}), &schema());
        assert!(result.is_err());
    }

    #[test]
    fn rejects_wrong_type() {
        let result = validate_against_schema("test", &json!({"path": 123}), &schema());
        assert!(result.is_err());
    }

    #[test]
    fn rejects_non_object_input() {
        let result = validate_against_schema("test", &json!("not an object"), &schema());
        assert!(result.is_err());
    }
}
