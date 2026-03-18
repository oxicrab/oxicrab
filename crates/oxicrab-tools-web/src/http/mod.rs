use crate::utils::media::{extension_from_content_type, save_media_file};
use anyhow::Result;
use async_trait::async_trait;
use oxicrab_core::tools::base::{ExecutionContext, ToolCapabilities, ToolCategory};
use oxicrab_core::tools::base::{Tool, ToolResult};
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;
use tracing::warn;

const MAX_RESPONSE_CHARS: usize = 50000;

/// HTTP headers that must not be set by LLM-generated tool calls.
const BLOCKED_HEADERS: &[&str] = &[
    "host",
    "authorization",
    "cookie",
    "set-cookie",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-real-ip",
    "proxy-authorization",
];
const DEFAULT_HTTP_TIMEOUT_SECS: u64 = 30;
const MAX_HTTP_TIMEOUT_SECS: u64 = 120;

#[derive(Default)]
pub struct HttpTool {
    /// Only used by test helpers (`send_request`); production path builds a
    /// per-request pinned client via `pinned_client()`.
    #[cfg(test)]
    client: Client,
}

impl HttpTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// HTTP execution with DNS-pinned client (used by `execute()` for SSRF-safe requests).
    async fn send_request_pinned(
        &self,
        params: &Value,
        resolved: &crate::utils::url_security::ResolvedUrl,
    ) -> Result<ToolResult> {
        let ua = format!("oxicrab/{}", env!("CARGO_PKG_VERSION"));
        let client = match crate::utils::http::build_pinned_client(
            resolved,
            Duration::from_secs(30),
            Some(&ua),
        ) {
            Ok(c) => c,
            Err(e) => return Ok(ToolResult::error(format!("{e}"))),
        };
        self.send_request_with_client(params, &client).await
    }

    /// Core HTTP execution logic (without SSRF validation).
    /// Separated from `execute()` so tests can call it directly with wiremock URLs.
    #[cfg(test)]
    async fn send_request(&self, params: &Value) -> Result<ToolResult> {
        self.send_request_with_client(params, &self.client).await
    }

    async fn send_request_with_client(
        &self,
        params: &Value,
        client: &Client,
    ) -> Result<ToolResult> {
        let Some(url) = params["url"].as_str() else {
            return Ok(ToolResult::error("missing 'url' parameter".to_string()));
        };

        let method = params["method"].as_str().unwrap_or("GET").to_uppercase();
        let timeout_secs = params["timeout_secs"]
            .as_u64()
            .unwrap_or(DEFAULT_HTTP_TIMEOUT_SECS)
            .min(MAX_HTTP_TIMEOUT_SECS);

        let mut request = match method.as_str() {
            "GET" => client.get(url),
            "POST" => client.post(url),
            "PUT" => client.put(url),
            "PATCH" => client.patch(url),
            "DELETE" => client.delete(url),
            _ => return Ok(ToolResult::error(format!("unsupported method: {method}"))),
        };

        request = request.timeout(Duration::from_secs(timeout_secs));

        // Apply custom headers (block sensitive headers to prevent injection)
        if let Some(headers) = params["headers"].as_object() {
            for (key, val) in headers {
                if let Some(v) = val.as_str() {
                    if BLOCKED_HEADERS.contains(&key.to_lowercase().as_str()) {
                        warn!("blocked sensitive header '{}' in http tool request", key);
                        continue;
                    }
                    request = request.header(key.as_str(), v);
                }
            }
        }

        // Apply body (after headers so user Content-Type is not overwritten)
        if !params["body"].is_null() {
            if params["body"].is_string() {
                let has_content_type = params["headers"]
                    .as_object()
                    .is_some_and(|h| h.keys().any(|k| k.eq_ignore_ascii_case("content-type")));
                if !has_content_type {
                    request = request.header("Content-Type", "text/plain");
                }
                request = request.body(params["body"].as_str().unwrap_or_default().to_string());
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
                    .unwrap_or_default()
                    .to_string();

                let header_str = if headers.is_empty() {
                    String::new()
                } else {
                    format!("\n{}", headers.join("\n"))
                };

                // Handle binary content -- save to disk
                if let Some(ext) = extension_from_content_type(&content_type) {
                    let (bytes, _truncated) = match crate::utils::http::limited_body(
                        resp,
                        crate::utils::http::DEFAULT_MAX_BODY_BYTES,
                    )
                    .await
                    {
                        Ok(result) => result,
                        Err(e) => {
                            return Ok(ToolResult::error(format!(
                                "HTTP {status} {method} — binary download failed: {e}"
                            )));
                        }
                    };
                    return match save_media_file(&bytes, "http", ext) {
                        Ok(path) => Ok(ToolResult::new(format!(
                            "HTTP {} {}{}\n\nBinary content saved to: {}\nSize: {} bytes\nType: {}",
                            status,
                            method,
                            header_str,
                            path,
                            bytes.len(),
                            content_type
                        ))),
                        Err(e) => Ok(ToolResult::error(format!(
                            "HTTP {status} {method} — failed to save binary response: {e}"
                        ))),
                    };
                }

                let body_text = crate::utils::http::limited_text(
                    resp,
                    crate::utils::http::DEFAULT_MAX_BODY_BYTES,
                )
                .await
                .unwrap_or_default();

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
                    format!("{truncated_text}...\n[truncated]")
                } else {
                    body_display
                };

                Ok(ToolResult::new(format!(
                    "HTTP {status} {method}{header_str}\n\n{final_body}"
                )))
            }
            Err(e) => Ok(ToolResult::error(format!("HTTP error: {e}"))),
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

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: true,
            category: ToolCategory::Web,
            ..Default::default()
        }
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

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let Some(url) = params["url"].as_str() else {
            return Ok(ToolResult::error("missing 'url' parameter".to_string()));
        };

        // Validate URL and resolve DNS for pinning (prevents TOCTOU rebinding)
        let resolved = match crate::utils::url_security::validate_and_resolve(url).await {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::error(e)),
        };

        self.send_request_pinned(&params, &resolved).await
    }
}

#[cfg(test)]
mod tests;
