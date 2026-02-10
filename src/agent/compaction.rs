use crate::providers::base::{ChatRequest, LLMProvider, Message};
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

const COMPACTION_PROMPT: &str = r"Summarize this conversation history concisely while preserving:
1. Key decisions made and their reasoning
2. Important facts, names, dates, and numbers mentioned
3. User preferences and requests
4. Pending tasks or commitments
5. Technical context that may be needed later

Previous summary (if any):
{previous_summary}

Messages to summarize:
{messages}

Write a concise summary (max 500 words) that captures the essential context. Do not include preamble - just the summary.";

const EXTRACTION_PROMPT: &str = r"Review this conversation exchange and extract any facts worth remembering long-term. Focus on:
- User preferences, habits, or personal details shared
- Decisions made or commitments given
- Project names, technical choices, or configuration details
- Anything the user would expect you to remember next time

User: {user_message}

Assistant: {assistant_message}

If there are notable facts, respond with a short bullet list (one line per fact). If nothing is worth remembering, respond with exactly: NOTHING";

pub fn estimate_tokens(text: &str) -> usize {
    // Use char count for better accuracy with non-ASCII text
    text.chars().count() / 4
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

        let llm_messages = vec![Message::user(prompt)];

        let response = self
            .provider
            .chat(ChatRequest {
                messages: llm_messages,
                tools: None,
                model: self.model.as_deref(),
                max_tokens: 2000,
                temperature: 0.3,
                tool_choice: None,
            })
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

        let llm_messages = vec![Message::user(prompt)];

        let response = self
            .provider
            .chat(ChatRequest {
                messages: llm_messages,
                tools: None,
                model: self.model.as_deref(),
                max_tokens: 500,
                temperature: 0.3,
                tool_choice: None,
            })
            .await?;

        let content = response.content.unwrap_or_default();
        // The LLM sometimes returns "NOTHING" with an explanation in parens.
        // Treat any response starting with "NOTHING" as no facts extracted.
        if content.trim().to_ascii_uppercase().starts_with("NOTHING") {
            Ok(String::new())
        } else {
            Ok(content)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_tokens_empty() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn estimate_tokens_ascii() {
        // 20 chars / 4 = 5
        assert_eq!(estimate_tokens("12345678901234567890"), 5);
    }

    #[test]
    fn estimate_tokens_unicode() {
        // Each emoji is 1 char (but 4 bytes). 4 emoji = 4 chars / 4 = 1 token
        assert_eq!(estimate_tokens("\u{1F600}\u{1F601}\u{1F602}\u{1F603}"), 1);
    }

    #[test]
    fn estimate_messages_tokens_empty() {
        assert_eq!(estimate_messages_tokens(&[]), 0);
    }

    #[test]
    fn estimate_messages_tokens_string_content() {
        let msgs = vec![{
            let mut m = HashMap::new();
            m.insert(
                "content".to_string(),
                Value::String("a".repeat(40)), // 40 chars = 10 tokens
            );
            m
        }];
        assert_eq!(estimate_messages_tokens(&msgs), 10);
    }

    #[test]
    fn estimate_messages_tokens_array_content() {
        let msgs = vec![{
            let mut m = HashMap::new();
            m.insert(
                "content".to_string(),
                serde_json::json!([
                    {"type": "text", "text": "a]a]a]a]"}, // 8 chars = 2 tokens
                    {"type": "image", "url": "http://example.com"},
                    {"type": "text", "text": "bbbb"}, // 4 chars = 1 token
                ]),
            );
            m
        }];
        assert_eq!(estimate_messages_tokens(&msgs), 3);
    }

    #[test]
    fn estimate_messages_tokens_missing_content() {
        let msgs = vec![{
            let mut m = HashMap::new();
            m.insert("role".to_string(), Value::String("user".to_string()));
            m
        }];
        assert_eq!(estimate_messages_tokens(&msgs), 0);
    }
}
