use crate::providers::base::{LLMProvider, Message};
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

const COMPACTION_PROMPT: &str = r#"Summarize this conversation history concisely while preserving:
1. Key decisions made and their reasoning
2. Important facts, names, dates, and numbers mentioned
3. User preferences and requests
4. Pending tasks or commitments
5. Technical context that may be needed later

Previous summary (if any):
{previous_summary}

Messages to summarize:
{messages}

Write a concise summary (max 500 words) that captures the essential context. Do not include preamble - just the summary."#;

const EXTRACTION_PROMPT: &str = r#"Review this conversation exchange and extract any facts worth remembering long-term. Focus on:
- User preferences, habits, or personal details shared
- Decisions made or commitments given
- Project names, technical choices, or configuration details
- Anything the user would expect you to remember next time

User: {user_message}

Assistant: {assistant_message}

If there are notable facts, respond with a short bullet list (one line per fact). If nothing is worth remembering, respond with exactly: NOTHING"#;

pub fn estimate_tokens(text: &str) -> usize {
    text.len() / 4
}

pub fn estimate_messages_tokens(messages: &[HashMap<String, Value>]) -> usize {
    let mut total = 0;
    for m in messages {
        if let Some(content) = m.get("content") {
            if let Some(text) = content.as_str() {
                total += estimate_tokens(text);
            } else if let Some(arr) = content.as_array() {
                for part in arr {
                    if let Some(obj) = part.as_object() {
                        if obj.get("type") == Some(&Value::String("text".to_string())) {
                            if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                                total += estimate_tokens(text);
                            }
                        }
                    }
                }
            }
        }
    }
    total
}

pub struct MessageCompactor {
    provider: Arc<dyn LLMProvider>,
    model: Option<String>,
}

impl MessageCompactor {
    pub fn new(provider: Arc<dyn LLMProvider>, model: Option<String>) -> Self {
        Self { provider, model }
    }

    pub async fn compact(
        &self,
        messages: &[HashMap<String, Value>],
        previous_summary: &str,
    ) -> Result<String> {
        let formatted: Vec<String> = messages
            .iter()
            .map(|m| {
                let role = m.get("role").and_then(|v| v.as_str()).unwrap_or("");
                let content = m.get("content").and_then(|v| v.as_str()).unwrap_or("");
                format!("{}: {}", role, content)
            })
            .collect();

        let messages_text = formatted.join("\n");
        let prompt = COMPACTION_PROMPT
            .replace("{previous_summary}", previous_summary)
            .replace("{messages}", &messages_text);

        let llm_messages = vec![Message {
            role: "user".to_string(),
            content: prompt,
            tool_calls: None,
            tool_call_id: None,
        }];

        let response = self
            .provider
            .chat(llm_messages, None, self.model.as_deref(), 2000, 0.3)
            .await?;

        Ok(response.content.unwrap_or_default())
    }

    pub async fn extract_facts(
        &self,
        user_message: &str,
        assistant_message: &str,
    ) -> Result<String> {
        let prompt = EXTRACTION_PROMPT
            .replace("{user_message}", user_message)
            .replace("{assistant_message}", assistant_message);

        let llm_messages = vec![Message {
            role: "user".to_string(),
            content: prompt,
            tool_calls: None,
            tool_call_id: None,
        }];

        let response = self
            .provider
            .chat(llm_messages, None, self.model.as_deref(), 500, 0.3)
            .await?;

        let content = response.content.unwrap_or_default();
        if content.trim().eq_ignore_ascii_case("NOTHING") {
            Ok(String::new())
        } else {
            Ok(content)
        }
    }
}
