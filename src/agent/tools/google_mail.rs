use crate::agent::tools::base::{
    ActionDescriptor, ExecutionContext, SubagentAccess, ToolCapabilities,
};
use crate::agent::tools::google_common::GoogleApiClient;
use crate::agent::tools::{Tool, ToolResult};
use crate::auth::google::GoogleCredentials;
use anyhow::Result;
use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::Value;

pub struct GoogleMailTool {
    api: GoogleApiClient,
}

impl GoogleMailTool {
    pub fn new(credentials: GoogleCredentials) -> Self {
        Self {
            api: GoogleApiClient::new(credentials, "https://www.googleapis.com/gmail/v1"),
        }
    }
}

#[async_trait]
impl Tool for GoogleMailTool {
    fn name(&self) -> &'static str {
        "google_mail"
    }

    fn description(&self) -> &'static str {
        "Interact with Gmail. Actions: search, read, send, reply, list_labels, label."
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: true,
            subagent_access: SubagentAccess::ReadOnly,
            actions: vec![
                ActionDescriptor {
                    name: "search",
                    read_only: true,
                },
                ActionDescriptor {
                    name: "read",
                    read_only: true,
                },
                ActionDescriptor {
                    name: "send",
                    read_only: false,
                },
                ActionDescriptor {
                    name: "reply",
                    read_only: false,
                },
                ActionDescriptor {
                    name: "list_labels",
                    read_only: true,
                },
                ActionDescriptor {
                    name: "label",
                    read_only: false,
                },
            ],
        }
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["search", "read", "send", "reply", "list_labels", "label"],
                    "description": "Action to perform"
                },
                "query": {
                    "type": "string",
                    "description": "Gmail search query (for search). e.g. 'is:unread from:alice'"
                },
                "message_id": {
                    "type": "string",
                    "description": "Message ID (for read / reply / label)"
                },
                "to": {
                    "type": "string",
                    "description": "Recipient email address (for send)"
                },
                "subject": {
                    "type": "string",
                    "description": "Email subject (for send)"
                },
                "body": {
                    "type": "string",
                    "description": "Email body text (for send / reply)"
                },
                "label_ids": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Label IDs to add (for label)"
                },
                "remove_label_ids": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Label IDs to remove (for label)"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum results to return (for search, default 10)",
                    "minimum": 1,
                    "maximum": 50
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let action = params["action"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

        match action {
            "search" => {
                let query = params["query"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'query' parameter"))?;
                let max_results = params["max_results"].as_u64().unwrap_or(10).min(50) as u32;

                let endpoint = format!(
                    "users/me/messages?q={}&maxResults={}",
                    urlencoding::encode(query),
                    max_results
                );
                let result = self.api.call(&endpoint, "GET", None).await?;
                let empty_messages: Vec<serde_json::Value> = vec![];
                let messages = result["messages"].as_array().unwrap_or(&empty_messages);

                if messages.is_empty() {
                    return Ok(ToolResult::new(format!(
                        "No messages found for query: {}",
                        query
                    )));
                }

                let mut lines = vec![format!(
                    "Found {} message(s) for: {}\n",
                    messages.len(),
                    query
                )];
                for msg_stub in messages {
                    let Some(msg_id) = msg_stub["id"].as_str().filter(|s| !s.is_empty()) else {
                        continue;
                    };
                    let endpoint = format!(
                        "users/me/messages/{}?format=metadata&metadataHeaders=From&metadataHeaders=Subject&metadataHeaders=Date",
                        urlencoding::encode(msg_id)
                    );
                    let msg = self.api.call(&endpoint, "GET", None).await?;
                    let headers: std::collections::HashMap<String, String> =
                        msg["payload"]["headers"]
                            .as_array()
                            .unwrap_or(&vec![])
                            .iter()
                            .filter_map(|h| {
                                let name = h["name"].as_str()?;
                                let value = h["value"].as_str()?;
                                Some((name.to_string(), value.to_string()))
                            })
                            .collect();
                    let snippet = msg["snippet"].as_str().unwrap_or("");

                    lines.push(format!(
                        "- ID: {}\n  From: {}\n  Subject: {}\n  Date: {}\n  Snippet: {}",
                        msg_id,
                        headers.get("From").unwrap_or(&"?".to_string()),
                        headers
                            .get("Subject")
                            .unwrap_or(&"(no subject)".to_string()),
                        headers.get("Date").unwrap_or(&"?".to_string()),
                        snippet
                    ));
                }
                Ok(ToolResult::new(lines.join("\n")))
            }
            "read" => {
                let message_id = params["message_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'message_id' parameter"))?;

                let endpoint = format!(
                    "users/me/messages/{}?format=full",
                    urlencoding::encode(message_id)
                );
                let msg = self.api.call(&endpoint, "GET", None).await?;
                let headers: std::collections::HashMap<String, String> = msg["payload"]["headers"]
                    .as_array()
                    .unwrap_or(&vec![])
                    .iter()
                    .filter_map(|h| {
                        let name = h["name"].as_str()?;
                        let value = h["value"].as_str()?;
                        Some((name.to_string(), value.to_string()))
                    })
                    .collect();
                let body = extract_body(&msg["payload"]);
                let labels: Vec<String> = msg["labelIds"]
                    .as_array()
                    .unwrap_or(&vec![])
                    .iter()
                    .filter_map(|l| l.as_str().map(std::string::ToString::to_string))
                    .collect();

                Ok(ToolResult::new(format!(
                    "From: {}\nTo: {}\nSubject: {}\nDate: {}\nLabels: {}\n---\n{}",
                    headers.get("From").unwrap_or(&"?".to_string()),
                    headers.get("To").unwrap_or(&"?".to_string()),
                    headers
                        .get("Subject")
                        .unwrap_or(&"(no subject)".to_string()),
                    headers.get("Date").unwrap_or(&"?".to_string()),
                    labels.join(", "),
                    body
                )))
            }
            "send" => {
                let to = params["to"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'to' parameter"))?;
                let subject = params["subject"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'subject' parameter"))?;
                let body = params["body"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'body' parameter"))?;

                // Sanitize all fields to prevent header injection via \r\n.
                // Body is sanitized too because it follows the blank line separator —
                // a \r\n in the body before the separator could inject headers.
                let to = to.replace(['\r', '\n'], "");
                let subject = subject.replace(['\r', '\n'], " ");
                let body = body.replace('\r', "");

                let email = format!("To: {}\r\nSubject: {}\r\n\r\n{}", to, subject, body);
                let raw = URL_SAFE_NO_PAD.encode(email.as_bytes());

                let body_json = serde_json::json!({"raw": raw});
                let endpoint = "users/me/messages/send";
                let sent = self.api.call(endpoint, "POST", Some(body_json)).await?;
                Ok(ToolResult::new(format!(
                    "Email sent successfully (ID: {})",
                    sent["id"].as_str().unwrap_or("?")
                )))
            }
            "reply" => {
                let message_id = params["message_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'message_id' parameter"))?;
                let body = params["body"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'body' parameter"))?;

                let endpoint = format!(
                    "users/me/messages/{}?format=metadata&metadataHeaders=From&metadataHeaders=Subject&metadataHeaders=Message-ID",
                    urlencoding::encode(message_id)
                );
                let original = self.api.call(&endpoint, "GET", None).await?;
                let headers: std::collections::HashMap<String, String> =
                    original["payload"]["headers"]
                        .as_array()
                        .unwrap_or(&vec![])
                        .iter()
                        .filter_map(|h| {
                            let name = h["name"].as_str()?;
                            let value = h["value"].as_str()?;
                            Some((name.to_string(), value.to_string()))
                        })
                        .collect();
                let thread_id = original["threadId"].as_str().unwrap_or("");

                let empty_str = String::new();
                let reply_to = headers
                    .get("From")
                    .unwrap_or(&empty_str)
                    .replace(['\r', '\n'], "");
                let mut subject = headers
                    .get("Subject")
                    .unwrap_or(&String::new())
                    .replace(['\r', '\n'], "");
                if !subject.to_lowercase().starts_with("re:") {
                    subject = format!("Re: {}", subject);
                }

                let message_id = headers
                    .get("Message-ID")
                    .unwrap_or(&String::new())
                    .replace(['\r', '\n'], "");
                let email = format!(
                    "To: {}\r\nSubject: {}\r\nIn-Reply-To: {}\r\nReferences: {}\r\n\r\n{}",
                    reply_to, subject, message_id, message_id, body
                );
                let raw = URL_SAFE_NO_PAD.encode(email.as_bytes());

                let body_json = serde_json::json!({
                    "raw": raw,
                    "threadId": thread_id
                });
                let endpoint = "users/me/messages/send";
                let sent = self.api.call(endpoint, "POST", Some(body_json)).await?;
                Ok(ToolResult::new(format!(
                    "Reply sent successfully (ID: {})",
                    sent["id"].as_str().unwrap_or("?")
                )))
            }
            "list_labels" => {
                let result = self.api.call("users/me/labels", "GET", None).await?;
                let empty_labels: Vec<serde_json::Value> = vec![];
                let labels = result["labels"].as_array().unwrap_or(&empty_labels);
                if labels.is_empty() {
                    return Ok(ToolResult::new("No labels found.".to_string()));
                }
                let mut lines = vec!["Gmail Labels:\n".to_string()];
                let mut sorted_labels: Vec<&Value> = labels.iter().collect();
                sorted_labels.sort_by_key(|l| l["name"].as_str().unwrap_or(""));
                for lbl in sorted_labels {
                    lines.push(format!(
                        "- {} (ID: {})",
                        lbl["name"].as_str().unwrap_or("?"),
                        lbl["id"].as_str().unwrap_or("?")
                    ));
                }
                Ok(ToolResult::new(lines.join("\n")))
            }
            "label" => {
                let message_id = params["message_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'message_id' parameter"))?;
                let label_ids = params["label_ids"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(std::string::ToString::to_string))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let remove_label_ids = params["remove_label_ids"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(std::string::ToString::to_string))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                if label_ids.is_empty() && remove_label_ids.is_empty() {
                    return Ok(ToolResult::error(
                        "provide 'label_ids' and/or 'remove_label_ids'".to_string(),
                    ));
                }

                let mut body = serde_json::json!({});
                if !label_ids.is_empty() {
                    body["addLabelIds"] =
                        Value::Array(label_ids.into_iter().map(Value::String).collect());
                }
                if !remove_label_ids.is_empty() {
                    body["removeLabelIds"] =
                        Value::Array(remove_label_ids.into_iter().map(Value::String).collect());
                }

                let endpoint = format!(
                    "users/me/messages/{}/modify",
                    urlencoding::encode(message_id)
                );
                self.api.call(&endpoint, "POST", Some(body)).await?;
                Ok(ToolResult::new(format!(
                    "Labels updated on message {}",
                    message_id
                )))
            }
            _ => Ok(ToolResult::error(format!("unknown action: {}", action))),
        }
    }
}

/// Extract the human-readable body from a Gmail message payload.
fn extract_body(payload: &Value) -> String {
    extract_body_inner(payload, 0)
}

fn extract_body_inner(payload: &Value, depth: u32) -> String {
    if depth > 10 {
        return "(nested too deep)".to_string();
    }

    // Direct body
    if payload["mimeType"].as_str() == Some("text/plain")
        && let Some(data) = payload["body"]["data"].as_str()
        && let Ok(decoded) = URL_SAFE_NO_PAD.decode(data)
        && let Ok(text) = String::from_utf8(decoded)
    {
        return text;
    }

    // Multipart - look for text/plain first, then text/html
    if let Some(parts) = payload["parts"].as_array() {
        for mime in &["text/plain", "text/html"] {
            for part in parts {
                if part["mimeType"].as_str() == Some(mime)
                    && let Some(data) = part["body"]["data"].as_str()
                    && let Ok(decoded) = URL_SAFE_NO_PAD.decode(data)
                    && let Ok(text) = String::from_utf8(decoded)
                {
                    if *mime == "text/html" {
                        // Strip HTML tags using shared regex pattern
                        return crate::utils::regex::RegexPatterns::html_tags()
                            .replace_all(&text, "")
                            .to_string();
                    }
                    return text;
                }
                // Nested multipart
                if part["parts"].is_array() {
                    let nested = extract_body_inner(part, depth + 1);
                    if nested != "(no readable body)" {
                        return nested;
                    }
                }
            }
        }
    }

    "(no readable body)".to_string()
}

#[cfg(test)]
mod tests {
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
}
