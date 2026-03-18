/// Serialize a provider-level `ResponseFormat` to a JSON value for metadata transport.
pub(super) fn response_format_to_json(
    rf: &oxicrab_core::providers::base::ResponseFormat,
) -> serde_json::Value {
    match rf {
        oxicrab_core::providers::base::ResponseFormat::JsonObject => {
            serde_json::Value::String("json".to_string())
        }
        oxicrab_core::providers::base::ResponseFormat::JsonSchema { name, schema } => {
            serde_json::json!({ "name": name, "schema": schema })
        }
    }
}

/// Deserialize a provider-level `ResponseFormat` from a metadata JSON value.
pub fn response_format_from_json(
    v: &serde_json::Value,
) -> Option<oxicrab_core::providers::base::ResponseFormat> {
    match v {
        serde_json::Value::String(s) if s == "json" => {
            Some(oxicrab_core::providers::base::ResponseFormat::JsonObject)
        }
        serde_json::Value::Object(map) => {
            let name = map.get("name")?.as_str()?.to_string();
            let schema = map.get("schema")?.clone();
            Some(oxicrab_core::providers::base::ResponseFormat::JsonSchema { name, schema })
        }
        _ => None,
    }
}
