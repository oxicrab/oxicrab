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

        // Validate URL scheme
        match url::Url::parse(url) {
            Ok(parsed) => {
                if !matches!(parsed.scheme(), "http" | "https") {
                    return Ok(ToolResult::error(format!(
                        "Only http/https allowed, got '{}'",
                        parsed.scheme()
                    )));
                }
            }
            Err(e) => return Ok(ToolResult::error(format!("Invalid URL: {}", e))),
        }

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
