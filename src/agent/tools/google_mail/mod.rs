use crate::actions;
use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities, ToolCategory};
use crate::agent::tools::google_common::GoogleApiClient;
use crate::agent::tools::{Tool, ToolResult};
use crate::auth::google::GoogleCredentials;
use crate::require_param;
use anyhow::Result;
use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::Value;
use std::collections::HashMap;

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
        "Interact with Gmail. Actions: search, read, send, reply, list_labels, label. Tip: after reading an email, use add_buttons to offer Reply, Archive, or Label actions."
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: true,
            subagent_access: SubagentAccess::ReadOnly,
            actions: actions![
                search: ro,
                read: ro,
                send,
                reply,
                list_labels: ro,
                label,
            ],
            category: ToolCategory::Communication,
        }
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["search", "read", "send", "reply", "list_labels", "label"],
                    "description": "Action to perform. 'search' finds emails by Gmail \
                     query (returns a list of matches). 'read' gets a specific email's full \
                     content by message_id. 'label' adds or removes labels from an email."
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
        let action = require_param!(params, "action");

        match action {
            "search" => {
                let query = require_param!(params, "query");
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
                        "No messages found for query: {query}"
                    )));
                }

                let mut lines = vec![format!(
                    "Found {} message(s) for: {}\n",
                    messages.len(),
                    query
                )];
                let mut msg_entries: Vec<(String, String)> = Vec::new();
                for msg_stub in messages {
                    let Some(msg_id) = msg_stub["id"].as_str().filter(|s| !s.is_empty()) else {
                        continue;
                    };
                    let endpoint = format!(
                        "users/me/messages/{}?format=metadata&metadataHeaders=From&metadataHeaders=Subject&metadataHeaders=Date",
                        urlencoding::encode(msg_id)
                    );
                    let msg = self.api.call(&endpoint, "GET", None).await?;
                    let headers: HashMap<String, String> = msg["payload"]["headers"]
                        .as_array()
                        .unwrap_or(&vec![])
                        .iter()
                        .filter_map(|h| {
                            let name = h["name"].as_str()?;
                            let value = h["value"].as_str()?;
                            Some((name.to_string(), value.to_string()))
                        })
                        .collect();
                    let snippet = msg["snippet"].as_str().unwrap_or_default();
                    let subject = headers
                        .get("Subject")
                        .unwrap_or(&"(no subject)".to_string())
                        .clone();

                    lines.push(format!(
                        "- ID: {}\n  From: {}\n  Subject: {}\n  Date: {}\n  Snippet: {}",
                        msg_id,
                        headers.get("From").unwrap_or(&"?".to_string()),
                        subject,
                        headers.get("Date").unwrap_or(&"?".to_string()),
                        snippet
                    ));
                    msg_entries.push((msg_id.to_string(), subject));
                }
                let buttons = build_search_buttons(&msg_entries);
                Ok(with_buttons(ToolResult::new(lines.join("\n")), buttons))
            }
            "read" => {
                let message_id = require_param!(params, "message_id");

                let endpoint = format!(
                    "users/me/messages/{}?format=full",
                    urlencoding::encode(message_id)
                );
                let msg = self.api.call(&endpoint, "GET", None).await?;
                let headers: HashMap<String, String> = msg["payload"]["headers"]
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
                let subject = headers
                    .get("Subject")
                    .unwrap_or(&"(no subject)".to_string())
                    .clone();

                let result = ToolResult::new(format!(
                    "From: {}\nTo: {}\nSubject: {}\nDate: {}\nLabels: {}\n---\n{}",
                    headers.get("From").unwrap_or(&"?".to_string()),
                    headers.get("To").unwrap_or(&"?".to_string()),
                    subject,
                    headers.get("Date").unwrap_or(&"?".to_string()),
                    labels.join(", "),
                    body
                ));
                let buttons = build_read_buttons(message_id, &subject);
                Ok(with_buttons(result, buttons))
            }
            "send" => {
                let to = require_param!(params, "to");
                let subject = require_param!(params, "subject");
                let body = require_param!(params, "body");

                // Sanitize all fields to prevent header injection via \r\n.
                // Body is sanitized too because it follows the blank line separator —
                // a \r\n in the body before the separator could inject headers.
                let to = to.replace(['\r', '\n'], "");
                let subject = subject.replace(['\r', '\n'], " ");
                let body = body.replace('\r', "");

                let email = format!("To: {to}\r\nSubject: {subject}\r\n\r\n{body}");
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
                let message_id = require_param!(params, "message_id");
                let body = require_param!(params, "body");
                let body = body.replace('\r', "");

                let endpoint = format!(
                    "users/me/messages/{}?format=metadata&metadataHeaders=From&metadataHeaders=Subject&metadataHeaders=Message-ID",
                    urlencoding::encode(message_id)
                );
                let original = self.api.call(&endpoint, "GET", None).await?;
                let headers: HashMap<String, String> = original["payload"]["headers"]
                    .as_array()
                    .unwrap_or(&vec![])
                    .iter()
                    .filter_map(|h| {
                        let name = h["name"].as_str()?;
                        let value = h["value"].as_str()?;
                        Some((name.to_string(), value.to_string()))
                    })
                    .collect();
                let thread_id = original["threadId"].as_str().unwrap_or_default();

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
                    subject = format!("Re: {subject}");
                }

                let message_id = headers
                    .get("Message-ID")
                    .unwrap_or(&String::new())
                    .replace(['\r', '\n'], "");
                let email = format!(
                    "To: {reply_to}\r\nSubject: {subject}\r\nIn-Reply-To: {message_id}\r\nReferences: {message_id}\r\n\r\n{body}"
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
                sorted_labels.sort_by_key(|l| l["name"].as_str().unwrap_or_default());
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
                let message_id = require_param!(params, "message_id");
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
                    "Labels updated on message {message_id}"
                )))
            }
            _ => Ok(ToolResult::error(format!("unknown action: {action}"))),
        }
    }
}

/// Build suggested "Read" buttons for search results (max 5).
///
/// Each button carries context identifying the message for a follow-up read action.
fn build_search_buttons(messages: &[(String, String)]) -> Vec<Value> {
    let mut buttons = Vec::new();
    for (msg_id, subject) in messages {
        if buttons.len() >= 5 {
            break;
        }
        if msg_id.is_empty() {
            continue;
        }
        let label = truncate_label("Read", subject, 25);
        buttons.push(serde_json::json!({
            "id": format!("read-{msg_id}"),
            "label": label,
            "style": "primary",
            "context": serde_json::json!({
                "tool": "google_mail",
                "message_id": msg_id,
                "action": "read"
            }).to_string()
        }));
    }
    buttons
}

/// Build suggested "Reply" and "Archive" buttons for a read message.
fn build_read_buttons(message_id: &str, subject: &str) -> Vec<Value> {
    if message_id.is_empty() {
        return Vec::new();
    }
    let subject_short: String = subject.chars().take(20).collect();
    let suffix = if subject_short.len() < subject.len() {
        format!("{subject_short}...")
    } else {
        subject_short
    };
    vec![
        serde_json::json!({
            "id": format!("reply-{message_id}"),
            "label": format!("Reply: {suffix}"),
            "style": "primary",
            "context": serde_json::json!({
                "tool": "google_mail",
                "message_id": message_id,
                "action": "reply"
            }).to_string()
        }),
        serde_json::json!({
            "id": format!("archive-{message_id}"),
            "label": "Archive",
            "style": "danger",
            "context": serde_json::json!({
                "tool": "google_mail",
                "message_id": message_id,
                "action": "archive"
            }).to_string()
        }),
    ]
}

/// UTF-8 safe label truncation: `"{prefix}: {text}"` capped at `max_chars` total.
fn truncate_label(prefix: &str, text: &str, max_chars: usize) -> String {
    // "{prefix}: " takes prefix.len() + 2 chars
    let budget = max_chars.saturating_sub(prefix.len() + 2);
    let truncated: String = text.chars().take(budget).collect();
    if truncated.len() < text.len() {
        let trimmed: String = text.chars().take(budget.saturating_sub(3)).collect();
        format!("{prefix}: {trimmed}...")
    } else {
        format!("{prefix}: {truncated}")
    }
}

/// Attach suggested buttons metadata to a `ToolResult` if there are any buttons.
fn with_buttons(result: ToolResult, buttons: Vec<Value>) -> ToolResult {
    if buttons.is_empty() {
        result
    } else {
        result.with_metadata(HashMap::from([(
            "suggested_buttons".to_string(),
            Value::Array(buttons),
        )]))
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
                        // Strip HTML tags (replace with space to preserve word boundaries)
                        let stripped = crate::utils::regex::RegexPatterns::html_tags()
                            .replace_all(&text, " ")
                            .to_string();
                        let cleaned = decode_html_entities(&stripped);
                        let collapsed = collapse_whitespace(&cleaned);
                        if collapsed.len() >= 20 {
                            return collapsed;
                        }
                        return "(HTML email with minimal text content. Subject and headers above may contain the key details.)".to_string();
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

/// Decode common HTML entities into their character equivalents.
fn decode_html_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&#160;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

/// Collapse runs of whitespace into a single space and trim leading/trailing whitespace.
fn collapse_whitespace(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut prev_ws = true; // starts true to trim leading
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_ws {
                result.push(' ');
                prev_ws = true;
            }
        } else {
            result.push(c);
            prev_ws = false;
        }
    }
    if result.ends_with(' ') {
        result.pop();
    }
    result
}

#[cfg(test)]
mod tests;
