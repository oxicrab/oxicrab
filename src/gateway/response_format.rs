/// Serialize a provider-level `ResponseFormat` to a JSON value for metadata transport.
pub(super) fn response_format_to_json(
    rf: &crate::providers::base::ResponseFormat,
) -> serde_json::Value {
    match rf {
        crate::providers::base::ResponseFormat::JsonObject => {
            serde_json::Value::String("json".to_string())
        }
        crate::providers::base::ResponseFormat::JsonSchema { name, schema } => {
            serde_json::json!({ "name": name, "schema": schema })
        }
    }
}

/// Deserialize a provider-level `ResponseFormat` from a metadata JSON value.
pub(crate) fn response_format_from_json(
    v: &serde_json::Value,
) -> Option<crate::providers::base::ResponseFormat> {
    match v {
        serde_json::Value::String(s) if s == "json" => {
            Some(crate::providers::base::ResponseFormat::JsonObject)
        }
        serde_json::Value::Object(map) => {
            let name = map.get("name")?.as_str()?.to_string();
            let schema = map.get("schema")?.clone();
            Some(crate::providers::base::ResponseFormat::JsonSchema { name, schema })
        }
        _ => None,
    }
}
