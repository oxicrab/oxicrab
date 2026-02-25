use super::*;
use crate::agent::tools::base::ExecutionContext;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// --- Validation tests (no network) ---

#[tokio::test]
async fn test_missing_url() {
    let tool = HttpTool::new();
    let result = tool
        .execute(
            serde_json::json!({"method": "GET"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("url"));
}

#[tokio::test]
async fn test_ssrf_blocked_localhost() {
    let tool = HttpTool::new();
    let result = tool
        .execute(
            serde_json::json!({"url": "http://127.0.0.1/admin"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn test_ssrf_blocked_private_ip() {
    let tool = HttpTool::new();
    let result = tool
        .execute(
            serde_json::json!({"url": "http://192.168.1.1/secret"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn test_ssrf_blocked_metadata() {
    let tool = HttpTool::new();
    let result = tool
        .execute(
            serde_json::json!({"url": "http://169.254.169.254/latest/meta-data/"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn test_unsupported_method() {
    let tool = HttpTool::new();
    let result = tool
        .send_request(&serde_json::json!({"url": "https://example.com", "method": "TRACE"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("unsupported method"));
}

// --- Wiremock tests (exercise actual HTTP execution) ---

#[tokio::test]
async fn test_get_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/data"))
        .respond_with(ResponseTemplate::new(200).set_body_string("hello world"))
        .mount(&server)
        .await;

    let tool = HttpTool::new();
    let result = tool
        .send_request(&serde_json::json!({"url": format!("{}/data", server.uri())}))
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("HTTP 200 GET"));
    assert!(result.content.contains("hello world"));
}

#[tokio::test]
async fn test_post_with_json_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api"))
        .and(body_json(serde_json::json!({"key": "value"})))
        .respond_with(ResponseTemplate::new(201).set_body_string("created"))
        .mount(&server)
        .await;

    let tool = HttpTool::new();
    let result = tool
        .send_request(&serde_json::json!({
            "url": format!("{}/api", server.uri()),
            "method": "POST",
            "body": {"key": "value"}
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("HTTP 201 POST"));
    assert!(result.content.contains("created"));
}

#[tokio::test]
async fn test_post_with_string_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/text"))
        .and(header("content-type", "text/plain"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let tool = HttpTool::new();
    let result = tool
        .send_request(&serde_json::json!({
            "url": format!("{}/text", server.uri()),
            "method": "POST",
            "body": "raw text body"
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("HTTP 200 POST"));
}

#[tokio::test]
async fn test_put_method() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/resource"))
        .respond_with(ResponseTemplate::new(200).set_body_string("updated"))
        .mount(&server)
        .await;

    let tool = HttpTool::new();
    let result = tool
        .send_request(&serde_json::json!({
            "url": format!("{}/resource", server.uri()),
            "method": "PUT",
            "body": {"name": "new"}
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("HTTP 200 PUT"));
}

#[tokio::test]
async fn test_patch_method() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/item"))
        .respond_with(ResponseTemplate::new(200).set_body_string("patched"))
        .mount(&server)
        .await;

    let tool = HttpTool::new();
    let result = tool
        .send_request(&serde_json::json!({
            "url": format!("{}/item", server.uri()),
            "method": "PATCH",
            "body": {"field": "updated"}
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("HTTP 200 PATCH"));
}

#[tokio::test]
async fn test_delete_method() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/item/42"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let tool = HttpTool::new();
    let result = tool
        .send_request(&serde_json::json!({
            "url": format!("{}/item/42", server.uri()),
            "method": "DELETE"
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("HTTP 204 DELETE"));
}

#[tokio::test]
async fn test_custom_headers_sent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/auth"))
        .and(header("Authorization", "Bearer token123"))
        .and(header("X-Custom", "myvalue"))
        .respond_with(ResponseTemplate::new(200).set_body_string("authed"))
        .mount(&server)
        .await;

    let tool = HttpTool::new();
    let result = tool
        .send_request(&serde_json::json!({
            "url": format!("{}/auth", server.uri()),
            "headers": {
                "Authorization": "Bearer token123",
                "X-Custom": "myvalue"
            }
        }))
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("authed"));
}

#[tokio::test]
async fn test_json_response_pretty_printed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/json"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"name": "test", "count": 42})),
        )
        .mount(&server)
        .await;

    let tool = HttpTool::new();
    let result = tool
        .send_request(&serde_json::json!({"url": format!("{}/json", server.uri())}))
        .await
        .unwrap();

    assert!(!result.is_error);
    // Pretty-printed JSON should have newlines and indentation
    assert!(result.content.contains("\"name\": \"test\""));
    assert!(result.content.contains("\"count\": 42"));
    assert!(result.content.contains("content-type: application/json"));
}

#[tokio::test]
async fn test_response_header_filtering() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/headers"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/plain")
                .insert_header("content-length", "5")
                .insert_header("x-request-id", "req-123")
                .insert_header("x-internal-secret", "should-not-appear")
                .set_body_string("hello"),
        )
        .mount(&server)
        .await;

    let tool = HttpTool::new();
    let result = tool
        .send_request(&serde_json::json!({"url": format!("{}/headers", server.uri())}))
        .await
        .unwrap();

    assert!(result.content.contains("content-type: text/plain"));
    assert!(result.content.contains("x-request-id: req-123"));
    // Internal headers should be filtered out
    assert!(!result.content.contains("x-internal-secret"));
    assert!(!result.content.contains("should-not-appear"));
}

#[tokio::test]
async fn test_location_header_shown() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/create"))
        .respond_with(
            ResponseTemplate::new(201)
                .insert_header("location", "/items/99")
                .set_body_string(""),
        )
        .mount(&server)
        .await;

    let tool = HttpTool::new();
    let result = tool
        .send_request(&serde_json::json!({
            "url": format!("{}/create", server.uri()),
            "method": "POST"
        }))
        .await
        .unwrap();

    assert!(result.content.contains("location: /items/99"));
}

#[tokio::test]
async fn test_response_truncation() {
    let server = MockServer::start().await;
    let large_body = "x".repeat(MAX_RESPONSE_CHARS + 1000);
    Mock::given(method("GET"))
        .and(path("/large"))
        .respond_with(ResponseTemplate::new(200).set_body_string(&large_body))
        .mount(&server)
        .await;

    let tool = HttpTool::new();
    let result = tool
        .send_request(&serde_json::json!({"url": format!("{}/large", server.uri())}))
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("[truncated]"));
    // Total content should be bounded
    assert!(result.content.len() < MAX_RESPONSE_CHARS + 500);
}

#[tokio::test]
async fn test_404_error_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/missing"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .mount(&server)
        .await;

    let tool = HttpTool::new();
    let result = tool
        .send_request(&serde_json::json!({"url": format!("{}/missing", server.uri())}))
        .await
        .unwrap();

    // HTTP errors are not tool errors — they're valid responses
    assert!(!result.is_error);
    assert!(result.content.contains("HTTP 404 GET"));
    assert!(result.content.contains("not found"));
}

#[tokio::test]
async fn test_500_error_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/error"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .mount(&server)
        .await;

    let tool = HttpTool::new();
    let result = tool
        .send_request(&serde_json::json!({"url": format!("{}/error", server.uri())}))
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("HTTP 500 GET"));
    assert!(result.content.contains("internal error"));
}

#[tokio::test]
async fn test_default_method_is_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/default"))
        .respond_with(ResponseTemplate::new(200).set_body_string("got it"))
        .mount(&server)
        .await;

    let tool = HttpTool::new();
    // No method specified — should default to GET
    let result = tool
        .send_request(&serde_json::json!({"url": format!("{}/default", server.uri())}))
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("HTTP 200 GET"));
}

#[tokio::test]
async fn test_non_json_response_not_pretty_printed() {
    let server = MockServer::start().await;
    // set_body_string sets content-type to text/plain by default
    Mock::given(method("GET"))
        .and(path("/plain"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"not":"pretty printed"}"#))
        .mount(&server)
        .await;

    let tool = HttpTool::new();
    let result = tool
        .send_request(&serde_json::json!({"url": format!("{}/plain", server.uri())}))
        .await
        .unwrap();

    // text/plain content should NOT be pretty-printed even if it looks like JSON
    assert!(result.content.contains(r#"{"not":"pretty printed"}"#));
}

#[tokio::test]
async fn test_empty_response_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/empty"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let tool = HttpTool::new();
    let result = tool
        .send_request(&serde_json::json!({"url": format!("{}/empty", server.uri())}))
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("HTTP 200 GET"));
}

#[test]
fn test_http_capabilities() {
    use crate::agent::tools::Tool;
    use crate::agent::tools::base::SubagentAccess;
    let tool = HttpTool::new();
    let caps = tool.capabilities();
    assert!(caps.built_in);
    assert!(caps.network_outbound);
    assert_eq!(caps.subagent_access, SubagentAccess::Denied);
    assert!(caps.actions.is_empty());
}
