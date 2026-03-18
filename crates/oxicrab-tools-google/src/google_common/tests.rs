use super::GoogleApiClient;
use oxicrab_core::config::schema::GoogleConfig;
use serde_json::Value;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn test_google_config_default() {
    let config = GoogleConfig::default();
    assert!(config.gmail);
    assert!(config.calendar);
    assert!(config.tasks);
}

#[test]
fn test_google_api_client_with_base_url() {
    let client = GoogleApiClient::with_base_url("http://localhost:9999");
    assert_eq!(client.base_url, "http://localhost:9999");
}

// --- Secret filtering unit tests ---

/// Replicates the secret filtering logic from `parse_response` for isolated testing.
fn filter_error_text(text: &str) -> String {
    text.lines()
        .filter(|line| {
            let lower = line.to_lowercase();
            !lower.contains("access_token")
                && !lower.contains("refresh_token")
                && !lower.contains("bearer")
                && !lower.contains("client_secret")
        })
        .collect::<Vec<_>>()
        .join("\n")
        .chars()
        .take(500)
        .collect()
}

#[test]
fn test_error_filter_removes_access_token_line() {
    let error_text = "Error: invalid_grant\naccess_token: sk-secret-123\nSome safe detail";
    let safe = filter_error_text(error_text);
    assert!(!safe.contains("sk-secret"));
    assert!(safe.contains("invalid_grant"));
    assert!(safe.contains("safe detail"));
}

#[test]
fn test_error_filter_removes_refresh_token_line() {
    let error_text = "Something went wrong\nrefresh_token: rt-456\nMore details";
    let safe = filter_error_text(error_text);
    assert!(!safe.contains("rt-456"));
    assert!(safe.contains("went wrong"));
    assert!(safe.contains("More details"));
}

#[test]
fn test_error_filter_removes_bearer_line() {
    let error_text = "Auth failed\nAuthorization: Bearer eyJtoken123\nRetry later";
    let safe = filter_error_text(error_text);
    assert!(!safe.contains("eyJtoken123"));
    assert!(safe.contains("Auth failed"));
    assert!(safe.contains("Retry later"));
}

#[test]
fn test_error_filter_removes_client_secret_line() {
    let error_text = "Error detail\nclient_secret: cs-789\nAnother line";
    let safe = filter_error_text(error_text);
    assert!(!safe.contains("cs-789"));
    assert!(safe.contains("Error detail"));
    assert!(safe.contains("Another line"));
}

#[test]
fn test_error_filter_removes_multiple_secret_lines() {
    let error_text = "Error: bad request\naccess_token: tok1\nrefresh_token: tok2\nclient_secret: sec3\nSafe line";
    let safe = filter_error_text(error_text);
    assert!(!safe.contains("tok1"));
    assert!(!safe.contains("tok2"));
    assert!(!safe.contains("sec3"));
    assert!(safe.contains("bad request"));
    assert!(safe.contains("Safe line"));
}

#[test]
fn test_error_filter_case_insensitive() {
    let error_text = "Issue\nACCESS_TOKEN: abc\nRefresh_Token: def\nBEARER xyz\nCLIENT_SECRET: ghi";
    let safe = filter_error_text(error_text);
    assert!(!safe.contains("abc"));
    assert!(!safe.contains("def"));
    assert!(!safe.contains("xyz"));
    assert!(!safe.contains("ghi"));
    assert!(safe.contains("Issue"));
}

#[test]
fn test_error_filter_caps_at_500_chars() {
    let lines: Vec<String> = (0..20)
        .map(|i| format!("safe error line {i:03} padding text here!!"))
        .collect();
    let error_text = lines.join("\n");
    let safe = filter_error_text(&error_text);
    assert_eq!(safe.chars().count(), 500);
}

#[test]
fn test_error_filter_preserves_safe_content() {
    let error_text = "Permission denied for user@example.com\nResource not found: calendar/123";
    let safe = filter_error_text(error_text);
    assert_eq!(safe, error_text);
}

#[test]
fn test_error_filter_all_lines_filtered() {
    let error_text = "access_token: a\nrefresh_token: b\nbearer c\nclient_secret: d";
    let safe = filter_error_text(error_text);
    assert!(safe.is_empty());
}

// --- Wiremock integration tests for parse_response via call() ---

#[tokio::test]
async fn test_parse_response_204_returns_null() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/calendars/abc"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let client = GoogleApiClient::with_base_url(&server.uri());
    let result = client.call("calendars/abc", "DELETE", None).await.unwrap();
    assert_eq!(result, Value::Null);
}

#[tokio::test]
async fn test_parse_response_success_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/me"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"email": "user@example.com"})),
        )
        .mount(&server)
        .await;

    let client = GoogleApiClient::with_base_url(&server.uri());
    let result = client.call("users/me", "GET", None).await.unwrap();
    assert_eq!(result["email"], "user@example.com");
}

#[tokio::test]
async fn test_parse_response_empty_body_returns_null() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/empty"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&server)
        .await;

    let client = GoogleApiClient::with_base_url(&server.uri());
    let result = client.call("empty", "GET", None).await.unwrap();
    assert_eq!(result, Value::Null);
}

#[tokio::test]
async fn test_parse_response_error_filters_secrets() {
    let server = MockServer::start().await;
    let error_body =
        "Error: invalid_grant\naccess_token: secret-tok\nrefresh_token: secret-rt\nSafe info";
    Mock::given(method("GET"))
        .and(path("/fail"))
        .respond_with(ResponseTemplate::new(400).set_body_string(error_body))
        .mount(&server)
        .await;

    let client = GoogleApiClient::with_base_url(&server.uri());
    let err = client.call("fail", "GET", None).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("invalid_grant"));
    assert!(msg.contains("Safe info"));
    assert!(!msg.contains("secret-tok"));
    assert!(!msg.contains("secret-rt"));
}

#[tokio::test]
async fn test_parse_response_error_caps_length() {
    let server = MockServer::start().await;
    let long_body = "x".repeat(1000);
    Mock::given(method("GET"))
        .and(path("/long-error"))
        .respond_with(ResponseTemplate::new(500).set_body_string(long_body))
        .mount(&server)
        .await;

    let client = GoogleApiClient::with_base_url(&server.uri());
    let err = client.call("long-error", "GET", None).await.unwrap_err();
    let msg = err.to_string();
    // The prefix "Google API error (500 Internal Server Error): " is ~48 chars,
    // plus at most 500 chars of filtered body
    assert!(msg.len() <= 600);
    assert!(msg.contains("Google API error"));
}

#[tokio::test]
async fn test_call_unsupported_method() {
    let server = MockServer::start().await;
    let client = GoogleApiClient::with_base_url(&server.uri());
    let err = client.call("test", "TRACE", None).await.unwrap_err();
    assert!(err.to_string().contains("Unsupported HTTP method"));
}

#[tokio::test]
async fn test_call_post_with_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/events"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"id": "evt-123", "status": "confirmed"})),
        )
        .mount(&server)
        .await;

    let client = GoogleApiClient::with_base_url(&server.uri());
    let body = serde_json::json!({"summary": "Test Event"});
    let result = client.call("events", "POST", Some(body)).await.unwrap();
    assert_eq!(result["id"], "evt-123");
}

#[tokio::test]
async fn test_call_put_method() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/items/1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"updated": true})),
        )
        .mount(&server)
        .await;

    let client = GoogleApiClient::with_base_url(&server.uri());
    let result = client
        .call("items/1", "PUT", Some(serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(result["updated"], true);
}

#[tokio::test]
async fn test_call_patch_method() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/items/2"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"patched": true})),
        )
        .mount(&server)
        .await;

    let client = GoogleApiClient::with_base_url(&server.uri());
    let result = client
        .call("items/2", "PATCH", Some(serde_json::json!({"name": "new"})))
        .await
        .unwrap();
    assert_eq!(result["patched"], true);
}
