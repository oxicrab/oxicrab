use super::*;
use base64::Engine;
use serde_json::json;

fn encode(text: &str) -> String {
    URL_SAFE_NO_PAD.encode(text.as_bytes())
}

#[test]
fn test_extract_body_plain_text() {
    let payload = json!({
        "mimeType": "text/plain",
        "body": {"data": encode("Hello world")}
    });
    assert_eq!(extract_body(&payload), "Hello world");
}

#[test]
fn test_extract_body_multipart_prefers_plain() {
    let payload = json!({
        "mimeType": "multipart/alternative",
        "parts": [
            {"mimeType": "text/plain", "body": {"data": encode("plain version")}},
            {"mimeType": "text/html", "body": {"data": encode("<b>html version</b>")}}
        ]
    });
    assert_eq!(extract_body(&payload), "plain version");
}

#[test]
fn test_extract_body_multipart_falls_back_to_html() {
    let payload = json!({
        "mimeType": "multipart/alternative",
        "parts": [
            {"mimeType": "text/html", "body": {"data": encode("<p>Hello</p>")}}
        ]
    });
    let result = extract_body(&payload);
    // HTML tags should be stripped
    assert!(result.contains("Hello"));
    assert!(!result.contains("<p>"));
}

#[test]
fn test_extract_body_nested_multipart() {
    let payload = json!({
        "mimeType": "multipart/mixed",
        "parts": [
            {
                "mimeType": "multipart/alternative",
                "parts": [
                    {"mimeType": "text/plain", "body": {"data": encode("nested plain")}}
                ]
            }
        ]
    });
    assert_eq!(extract_body(&payload), "nested plain");
}

#[test]
fn test_extract_body_no_readable_body() {
    let payload = json!({
        "mimeType": "multipart/mixed",
        "parts": [
            {"mimeType": "application/pdf", "body": {"data": encode("binary")}}
        ]
    });
    assert_eq!(extract_body(&payload), "(no readable body)");
}

#[test]
fn test_extract_body_depth_limit() {
    // Build deeply nested payload (depth > 10)
    let mut payload = json!({"mimeType": "text/plain", "body": {"data": encode("deep")}});
    for _ in 0..12 {
        payload = json!({
            "mimeType": "multipart/mixed",
            "parts": [payload]
        });
    }
    assert_eq!(extract_body(&payload), "(nested too deep)");
}

#[test]
fn test_extract_body_empty_payload() {
    let payload = json!({});
    assert_eq!(extract_body(&payload), "(no readable body)");
}

#[test]
fn test_extract_body_invalid_base64() {
    let payload = json!({
        "mimeType": "text/plain",
        "body": {"data": "!!!invalid-base64!!!"}
    });
    // Should not crash, falls through to no readable body
    assert_eq!(extract_body(&payload), "(no readable body)");
}

fn test_credentials() -> GoogleCredentials {
    GoogleCredentials {
        token: "fake".to_string(),
        refresh_token: None,
        token_uri: "https://oauth2.googleapis.com/token".to_string(),
        client_id: "fake".to_string(),
        client_secret: "fake".to_string(),
        scopes: vec![],
        expiry: None,
    }
}

#[test]
fn test_google_mail_capabilities() {
    use crate::agent::tools::base::SubagentAccess;
    let tool = GoogleMailTool::new(test_credentials());
    let caps = tool.capabilities();
    assert!(caps.built_in);
    assert!(caps.network_outbound);
    assert_eq!(caps.subagent_access, SubagentAccess::ReadOnly);
    let read_only: Vec<&str> = caps
        .actions
        .iter()
        .filter(|a| a.read_only)
        .map(|a| a.name)
        .collect();
    let mutating: Vec<&str> = caps
        .actions
        .iter()
        .filter(|a| !a.read_only)
        .map(|a| a.name)
        .collect();
    assert!(read_only.contains(&"search"));
    assert!(read_only.contains(&"read"));
    assert!(read_only.contains(&"list_labels"));
    assert!(mutating.contains(&"send"));
    assert!(mutating.contains(&"reply"));
    assert!(mutating.contains(&"label"));
}

#[test]
fn test_google_mail_actions_match_schema() {
    let tool = GoogleMailTool::new(test_credentials());
    let caps = tool.capabilities();
    let params = tool.parameters();
    let schema_actions: Vec<String> = params["properties"]["action"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let cap_actions: Vec<String> = caps.actions.iter().map(|a| a.name.to_string()).collect();
    for action in &schema_actions {
        assert!(
            cap_actions.contains(action),
            "action '{}' in schema but not in capabilities()",
            action
        );
    }
    for action in &cap_actions {
        assert!(
            schema_actions.contains(action),
            "action '{}' in capabilities() but not in schema",
            action
        );
    }
}
