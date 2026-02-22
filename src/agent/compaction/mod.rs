use crate::providers::base::{ChatRequest, LLMProvider, Message};
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, warn};

const COMPACTION_PROMPT: &str = "Summarize this conversation history concisely while preserving:\n1. Key decisions made and their reasoning\n2. Important facts, names, dates, and numbers mentioned\n3. User preferences and requests\n4. Pending tasks or commitments\n5. Technical context that may be needed later\n\nPrevious summary (if any):\n{previous_summary}\n\nMessages to summarize:\n{messages}\n\nWrite a concise summary (max 500 words) that captures the essential context. Do not include preamble - just the summary.";

const EXTRACTION_PROMPT: &str = "Review this conversation exchange and extract any facts worth remembering long-term. Focus on:\n- User preferences, habits, or personal details shared\n- Decisions made or commitments given\n- Project names, technical choices, or configuration details\n- Anything the user would expect you to remember next time\n\nUser: {user_message}\n\nAssistant: {assistant_message}\n\nIf there are notable facts, respond with a short bullet list (one line per fact). If nothing is worth remembering, respond with exactly: NOTHING";

const COMPACTION_MAX_TOKENS: u32 = 2000;
const EXTRACTION_MAX_TOKENS: u32 = 500;
const COMPACTION_TEMPERATURE: f32 = 0.3;
const EXTRACTION_TEMPERATURE: f32 = 0.0;
const CHARS_PER_TOKEN_ESTIMATE: usize = 4;

pub fn estimate_tokens(text: &str) -> usize {
    // Use char count for better accuracy with non-ASCII text
    text.chars().count() / CHARS_PER_TOKEN_ESTIMATE
}

#[allow(clippy::implicit_hasher)]
pub fn estimate_messages_tokens(messages: &[HashMap<String, Value>]) -> usize {
    let mut total = 0;
    for m in messages {
        if let Some(content) = m.get("content") {
            if let Some(text) = content.as_str() {
                total += estimate_tokens(text);
            } else if let Some(arr) = content.as_array() {
                for part in arr {
                    if let Some(obj) = part.as_object()
                        && obj.get("type") == Some(&Value::String("text".to_string()))
                        && let Some(text) = obj.get("text").and_then(|v| v.as_str())
                    {
                        total += estimate_tokens(text);
                    }
                }
            }
        }
    }
    total
}

/// Extract text from a message content value, handling both plain strings
/// and Anthropic-style content block arrays (text + image blocks).
fn extract_message_text(content: Option<&Value>) -> String {
    let Some(content) = content else {
        return String::new();
    };
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    if let Some(arr) = content.as_array() {
        let mut parts = Vec::new();
        for part in arr {
            if let Some(obj) = part.as_object() {
                match obj.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(text) = obj.get("text").and_then(Value::as_str) {
                            parts.push(text.to_string());
                        }
                    }
                    Some("image" | "image_url") => {
                        parts.push("[image]".to_string());
                    }
                    _ => {}
                }
            }
        }
        return parts.join(" ");
    }
    String::new()
}

pub struct MessageCompactor {
    provider: Arc<dyn LLMProvider>,
    model: Option<String>,
}

impl MessageCompactor {
    pub fn new(provider: Arc<dyn LLMProvider>, model: Option<String>) -> Self {
        Self { provider, model }
    }

    /// Summarize a slice of conversation messages into a concise summary.
    ///
    /// Uses [`estimate_tokens`] (chars/4) to gauge message size. The LLM is asked to preserve
    /// key decisions, facts, preferences, and pending tasks. `previous_summary` is included
    /// in the prompt so summaries build incrementally rather than losing earlier context.
    pub async fn compact(
        &self,
        messages: &[HashMap<String, Value>],
        previous_summary: &str,
    ) -> Result<String> {
        debug!("compaction: summarizing {} messages", messages.len());
        let formatted: Vec<String> = messages
            .iter()
            .map(|m| {
                let role = m.get("role").and_then(|v| v.as_str()).unwrap_or("");
                let content = extract_message_text(m.get("content"));
                format!("{}: {}", role, content)
            })
            .collect();

        let messages_text = formatted.join("\n");
        let effective_summary = if previous_summary.is_empty() {
            "(none)"
        } else {
            previous_summary
        };
        let prompt = COMPACTION_PROMPT
            .replace("{previous_summary}", effective_summary)
            .replace("{messages}", &messages_text);

        let llm_messages = vec![Message::user(prompt)];

        let response = self
            .provider
            .chat(ChatRequest {
                messages: llm_messages,
                tools: None,
                model: self.model.as_deref(),
                max_tokens: COMPACTION_MAX_TOKENS,
                temperature: COMPACTION_TEMPERATURE,
                tool_choice: None,
            })
            .await?;

        let summary = response.content.unwrap_or_default();
        if summary.trim().is_empty() {
            if !previous_summary.is_empty() {
                warn!("compaction returned empty summary, reusing previous summary");
                return Ok(previous_summary.to_string());
            }
            warn!("compaction returned empty summary with no previous summary available");
            return Err(anyhow::anyhow!("compaction produced empty summary"));
        }
        debug!("compaction complete: summary_len={}", summary.len());
        Ok(summary)
    }

    pub async fn extract_facts(
        &self,
        user_message: &str,
        assistant_message: &str,
    ) -> Result<String> {
        debug!("extracting facts from exchange");
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
                max_tokens: EXTRACTION_MAX_TOKENS,
                temperature: EXTRACTION_TEMPERATURE,
                tool_choice: None,
            })
            .await?;

        let content = response.content.unwrap_or_default();
        // The LLM sometimes returns "NOTHING" with an explanation in parens.
        // Treat any response starting with "NOTHING" as no facts extracted.
        if content.trim().to_ascii_uppercase().starts_with("NOTHING") {
            debug!("fact extraction: nothing");
            Ok(String::new())
        } else {
            debug!("fact extraction: facts found");
            Ok(content)
        }
    }
}

#[cfg(test)]
mod tests;
