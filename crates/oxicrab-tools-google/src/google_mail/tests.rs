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
            {"mimeType": "text/html", "body": {"data": encode("<p>Hello, this is a test email with enough content to pass the threshold</p>")}}
        ]
    });
    let result = extract_body(&payload);
    assert!(
        result.contains("Hello"),
        "should contain extracted text: {result}"
    );
    assert!(
        !result.contains("<p>"),
        "should not contain HTML tags: {result}"
    );
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
    assert_eq!(extract_body(&payload), "(no readable body)");
}

// --- Suggested buttons tests ---

#[test]
fn test_build_search_buttons_basic() {
    let messages = vec![
        ("msg1".to_string(), "Hello World".to_string()),
        ("msg2".to_string(), "Meeting Tomorrow".to_string()),
    ];
    let buttons = build_search_buttons(&messages);
    assert_eq!(buttons.len(), 2);

    assert_eq!(buttons[0]["id"], "read-msg1");
    assert_eq!(buttons[0]["style"], "primary");
    let ctx0: serde_json::Value =
        serde_json::from_str(buttons[0]["context"].as_str().unwrap()).unwrap();
    assert_eq!(ctx0["tool"], "google_mail");
    assert_eq!(ctx0["params"]["message_id"], "msg1");
    assert_eq!(ctx0["params"]["action"], "read");

    assert_eq!(buttons[1]["id"], "read-msg2");
    let ctx1: serde_json::Value =
        serde_json::from_str(buttons[1]["context"].as_str().unwrap()).unwrap();
    assert_eq!(ctx1["params"]["message_id"], "msg2");
}

#[test]
fn test_build_search_buttons_max_five() {
    let messages: Vec<(String, String)> = (0..8)
        .map(|i| (format!("msg{i}"), format!("Subject {i}")))
        .collect();
    let buttons = build_search_buttons(&messages);
    assert_eq!(buttons.len(), 5);
}

#[test]
fn test_build_search_buttons_skips_empty_id() {
    let messages = vec![
        (String::new(), "No ID".to_string()),
        ("msg1".to_string(), "Has ID".to_string()),
    ];
    let buttons = build_search_buttons(&messages);
    assert_eq!(buttons.len(), 1);
    assert_eq!(buttons[0]["id"], "read-msg1");
}

#[test]
fn test_build_search_buttons_empty() {
    let buttons = build_search_buttons(&[]);
    assert!(buttons.is_empty());
}

#[test]
fn test_build_read_buttons_basic() {
    let buttons = build_read_buttons("abc123", "Project Update");
    assert_eq!(buttons.len(), 2);

    assert_eq!(buttons[0]["id"], "reply-abc123");
    assert_eq!(buttons[0]["style"], "primary");
    assert!(buttons[0]["label"].as_str().unwrap().starts_with("Reply:"));
    // Reply button context is a plain string (not JSON) so it falls through to
    // the LLM path — direct dispatch would fail without a `body` parameter.
    let ctx0 = buttons[0]["context"].as_str().unwrap();
    assert!(
        ctx0.contains("abc123"),
        "context should mention the message id"
    );
    assert!(
        serde_json::from_str::<serde_json::Value>(ctx0).is_err(),
        "reply context must NOT parse as JSON ActionDispatchPayload"
    );

    assert_eq!(buttons[1]["id"], "archive-abc123");
    assert_eq!(buttons[1]["label"], "Archive");
    assert_eq!(buttons[1]["style"], "danger");
    let ctx1: serde_json::Value =
        serde_json::from_str(buttons[1]["context"].as_str().unwrap()).unwrap();
    assert_eq!(ctx1["params"]["action"], "label");
}

#[test]
fn test_build_read_buttons_empty_id() {
    let buttons = build_read_buttons("", "Some Subject");
    assert!(buttons.is_empty());
}

#[test]
fn test_build_read_buttons_long_subject_truncated() {
    let long_subject = "A very long email subject line that exceeds the limit";
    let buttons = build_read_buttons("msg1", long_subject);
    let label = buttons[0]["label"].as_str().unwrap();
    assert!(label.starts_with("Reply:"));
    assert!(label.ends_with("..."));
}

#[test]
fn test_truncate_label_short_text() {
    let result = truncate_label("Read", "Hi", 25);
    assert_eq!(result, "Read: Hi");
}

#[test]
fn test_truncate_label_long_text() {
    let result = truncate_label("Read", "This is a very long subject line", 25);
    assert!(result.starts_with("Read: "));
    assert!(result.ends_with("..."));
    assert!(result.chars().count() <= 25);
}

#[test]
fn test_truncate_label_unicode() {
    let result = truncate_label("Read", "日本語のメールの件名です", 15);
    assert!(result.starts_with("Read: "));
    assert!(result.ends_with("..."));
}

#[test]
fn test_with_buttons_empty() {
    let result = ToolResult::new("test");
    let result = result.with_buttons(vec![]);
    assert!(result.metadata.is_none());
}

#[test]
fn test_with_buttons_attaches_metadata() {
    let result = ToolResult::new("test");
    let buttons = vec![json!({"id": "b1", "label": "Button"})];
    let result = result.with_buttons(buttons);
    let meta = result.metadata.expect("should have metadata");
    let btns = meta["suggested_buttons"].as_array().unwrap();
    assert_eq!(btns.len(), 1);
    assert_eq!(btns[0]["id"], "b1");
}

#[test]
fn test_extract_body_html_marketing_email() {
    // Marketing email with tables, divs, and some text content
    let html = r#"<html><head><style>body{margin:0}</style></head><body>
        <table><tr><td><img src="logo.png"/></td></tr></table>
        <div class="header"><h1>Sports Illustrated Tickets</h1></div>
        <table width="600"><tr><td>
        <p>Your tickets for the upcoming game are confirmed.</p>
        <p>Section 204, Row F, Seats 12-14</p>
        </td></tr></table>
        <table><tr><td><a href="http://example.com">View Details</a></td></tr></table>
        <div class="footer"><p>Unsubscribe | Privacy Policy</p></div>
        </body></html>"#;
    let payload = json!({
        "mimeType": "multipart/alternative",
        "parts": [
            {"mimeType": "text/html", "body": {"data": encode(html)}}
        ]
    });
    let result = extract_body(&payload);
    assert!(
        result.contains("Sports Illustrated Tickets"),
        "should extract heading text, got: {result}"
    );
    assert!(
        result.contains("Section 204"),
        "should extract ticket details, got: {result}"
    );
    assert!(
        !result.contains('<'),
        "should not contain HTML tags, got: {result}"
    );
    assert_ne!(
        result, "(no readable body)",
        "should not fall through to sentinel"
    );
}

#[test]
fn test_extract_body_html_image_only_email() {
    // Email that's all images/layout with no meaningful text
    let html = r#"<html><body>
        <table><tr><td><img src="banner.png"/></td></tr></table>
        <table><tr><td><img src="promo.png"/></td></tr></table>
        <table><tr><td>&nbsp;</td></tr></table>
        </body></html>"#;
    let payload = json!({
        "mimeType": "multipart/alternative",
        "parts": [
            {"mimeType": "text/html", "body": {"data": encode(html)}}
        ]
    });
    let result = extract_body(&payload);
    assert_eq!(
        result,
        "(HTML email with minimal text content. Subject and headers above may contain the key details.)"
    );
}

#[test]
fn test_html_entity_decoding() {
    let input = "Hello&nbsp;World &amp; &lt;Friends&gt; said &quot;hi&quot; &#39;today&#39; &apos;now&apos; &#160;end";
    let decoded = decode_html_entities(input);
    assert_eq!(
        decoded,
        "Hello World & <Friends> said \"hi\" 'today' 'now'  end"
    );
}

#[test]
fn test_collapse_whitespace() {
    assert_eq!(collapse_whitespace("  hello   world  "), "hello world");
    assert_eq!(collapse_whitespace("no extra"), "no extra");
    assert_eq!(collapse_whitespace("  \n\t  spaced  \n  "), "spaced");
    assert_eq!(collapse_whitespace(""), "");
    assert_eq!(collapse_whitespace("   "), "");
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
    use oxicrab_core::tools::base::SubagentAccess;
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
            "action '{action}' in schema but not in capabilities()"
        );
    }
    for action in &cap_actions {
        assert!(
            schema_actions.contains(action),
            "action '{action}' in capabilities() but not in schema"
        );
    }
}
