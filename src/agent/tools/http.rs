use crate::agent::tools::{Tool, ToolResult, ToolVersion};
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

const MAX_RESPONSE_CHARS: usize = 50000;

pub struct HttpTool {
    client: Client,
}

impl Default for HttpTool {
    fn default() -> Self {
        Self {
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::limited(5))
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| Client::new()),
        }
    }
}

impl HttpTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Core HTTP execution logic (without SSRF validation).
    /// Separated from `execute()` so tests can call it directly with wiremock URLs.
    async fn send_request(&self, params: &Value) -> Result<ToolResult> {
        let url = match params["url"].as_str() {
            Some(u) => u,
            None => return Ok(ToolResult::error("Missing 'url' parameter".to_string())),
        };

        let method = params["method"].as_str().unwrap_or("GET").to_uppercase();
        let timeout_secs = params["timeout_secs"].as_u64().unwrap_or(30).min(120);

        let mut request = match method.as_str() {
            "GET" => self.client.get(url),
            "POST" => self.client.post(url),
            "PUT" => self.client.put(url),
            "PATCH" => self.client.patch(url),
            "DELETE" => self.client.delete(url),
            _ => return Ok(ToolResult::error(format!("Unsupported method: {}", method))),
        };

        request = request.timeout(Duration::from_secs(timeout_secs));

        // Apply custom headers
        if let Some(headers) = params["headers"].as_object() {
            for (key, val) in headers {
                if let Some(v) = val.as_str() {
                    request = request.header(key.as_str(), v);
                }
            }
        }

        // Apply body
        if !params["body"].is_null() {
            if params["body"].is_string() {
                request = request
                    .header("Content-Type", "text/plain")
                    .body(params["body"].as_str().unwrap_or("").to_string());
            } else {
                request = request.json(&params["body"]);
            }
        }

        match request.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let headers: Vec<String> = resp
                    .headers()
                    .iter()
                    .filter(|(k, _)| {
                        matches!(
                            k.as_str(),
                            "content-type" | "content-length" | "location" | "x-request-id"
                        )
                    })
                    .map(|(k, v)| format!("{}: {}", k, v.to_str().unwrap_or("?")))
                    .collect();

                let content_type = resp
                    .headers()
                    .get("content-type")
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or("")
                    .to_string();

                let body_text = resp.text().await.unwrap_or_default();

                // Try to pretty-print JSON
                let body_display = if content_type.contains("json") {
                    serde_json::from_str::<Value>(&body_text)
                        .and_then(|v| serde_json::to_string_pretty(&v))
                        .unwrap_or(body_text)
                } else {
                    body_text
                };

                // Truncate if needed
                let truncated = body_display.len() > MAX_RESPONSE_CHARS;
                let final_body: String = if truncated {
                    let truncated_text: String =
                        body_display.chars().take(MAX_RESPONSE_CHARS).collect();
                    format!("{}...\n[truncated]", truncated_text)
                } else {
                    body_display
                };

                let header_str = if headers.is_empty() {
                    String::new()
                } else {
                    format!("\n{}", headers.join("\n"))
                };

                Ok(ToolResult::new(format!(
                    "HTTP {} {}{}\n\n{}",
                    status, method, header_str, final_body
                )))
            }
            Err(e) => Ok(ToolResult::error(format!("HTTP error: {}", e))),
        }
    }
}

#[async_trait]
impl Tool for HttpTool {
    fn name(&self) -> &str {
        "http"
    }

    fn description(&self) -> &str {
        "Make HTTP requests (GET/POST/PUT/PATCH/DELETE). For REST APIs, webhooks, and services."
    }

    fn version(&self) -> ToolVersion {
        ToolVersion::new(1, 0, 0)
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST", "PUT", "PATCH", "DELETE"],
                    "default": "GET",
                    "description": "HTTP method"
                },
                "url": {
                    "type": "string",
                    "description": "Full URL to request"
                },
                "headers": {
                    "type": "object",
                    "description": "Request headers as key-value pairs"
                },
                "body": {
                    "description": "Request body (string or JSON object). Sent as JSON if object, raw if string."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Request timeout in seconds (default 30, max 120)"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, params: Value) -> Result<ToolResult> {
        let url = match params["url"].as_str() {
            Some(u) => u,
            None => return Ok(ToolResult::error("Missing 'url' parameter".to_string())),
        };

        // Validate URL scheme and block SSRF to internal networks
        if let Err(e) = crate::utils::url_security::validate_url(url) {
            return Ok(ToolResult::error(e));
        }

        self.send_request(&params).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // --- Validation tests (no network) ---

    #[tokio::test]
    async fn test_missing_url() {
        let tool = HttpTool::new();
        let result = tool
            .execute(serde_json::json!({"method": "GET"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("url"));
    }

    #[tokio::test]
    async fn test_ssrf_blocked_localhost() {
        let tool = HttpTool::new();
        let result = tool
            .execute(serde_json::json!({"url": "http://127.0.0.1/admin"}))
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_ssrf_blocked_private_ip() {
        let tool = HttpTool::new();
        let result = tool
            .execute(serde_json::json!({"url": "http://192.168.1.1/secret"}))
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_ssrf_blocked_metadata() {
        let tool = HttpTool::new();
        let result = tool
            .execute(serde_json::json!({"url": "http://169.254.169.254/latest/meta-data/"}))
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
        assert!(result.content.contains("Unsupported method"));
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
}
