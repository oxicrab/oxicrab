use crate::providers::base::{ChatRequest, LLMProvider, Message};
use anyhow::Result;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tracing::{debug, warn};

const COMPACTION_PROMPT: &str = "Summarize this conversation history concisely while preserving:\n1. Key decisions made and their reasoning\n2. Important facts, names, dates, and numbers mentioned\n3. User preferences and requests\n4. Pending tasks or commitments\n5. Technical context that may be needed later\n\nPrevious summary (if any):\n{previous_summary}\n\nMessages to summarize:\n{messages}\n\nWrite a concise summary (max 500 words) that captures the essential context. Do not include preamble - just the summary.";

const EXTRACTION_PROMPT: &str = "Review this conversation exchange and extract any facts worth remembering long-term. Focus on:\n- User preferences, habits, or personal details shared\n- Decisions made or commitments given\n- Project names, technical choices, or configuration details\n- Anything the user would expect you to remember next time\n\nUser: {user_message}\n\nAssistant: {assistant_message}\n\nIf there are notable facts, respond with a short bullet list (one line per fact). If nothing is worth remembering, respond with exactly: NOTHING";

const PRE_FLUSH_PROMPT: &str = "Review these conversation messages that are about to be removed from context. Extract any important information worth preserving long-term:\n- User preferences and decisions\n- Project state and progress\n- Key facts, names, dates, or configuration details\n- Commitments or pending items\n\nRespond with a concise bullet list of important items. If nothing is worth preserving, respond with exactly: NOTHING\n\nMessages:\n{messages}";

const COMPACTION_MAX_TOKENS: u32 = 2000;
const EXTRACTION_MAX_TOKENS: u32 = 500;
const PRE_FLUSH_MAX_TOKENS: u32 = 800;
const COMPACTION_TEMPERATURE: f32 = 0.3;
const EXTRACTION_TEMPERATURE: f32 = 0.0;
const PRE_FLUSH_TEMPERATURE: f32 = 0.0;
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

/// Remove orphaned tool messages from a message list.
///
/// After compaction removes older messages, a `tool_result` (role="tool")
/// may reference a `tool_use` that no longer exists, or an assistant message
/// with `tool_calls` may lack corresponding `tool_result` responses. These
/// orphans can cause API errors with providers that enforce strict pairing
/// (e.g. Anthropic).
///
/// Scans the message list in two passes:
/// 1. Collect all `tool_call` IDs from assistant messages and all
///    `tool_call` IDs referenced by tool-result messages.
/// 2. Remove tool-result messages whose ID has no matching assistant
///    `tool_call`, and remove assistant `tool_calls` whose ID has no
///    matching tool-result.
#[allow(clippy::implicit_hasher)]
pub fn strip_orphaned_tool_messages(messages: &mut Vec<HashMap<String, Value>>) -> (usize, usize) {
    // Collect tool_call IDs from assistant messages (tool_use side)
    let mut assistant_tool_ids: HashSet<String> = HashSet::new();
    for msg in messages.iter() {
        if msg.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        // tool_calls stored as array in extra map
        if let Some(tool_calls) = msg.get("tool_calls").and_then(Value::as_array) {
            for tc in tool_calls {
                if let Some(id) = tc.get("id").and_then(Value::as_str) {
                    assistant_tool_ids.insert(id.to_string());
                }
            }
        }
        // Also check content array for Anthropic-style tool_use blocks
        if let Some(content) = msg.get("content").and_then(Value::as_array) {
            for block in content {
                if block.get("type").and_then(Value::as_str) == Some("tool_use")
                    && let Some(id) = block.get("id").and_then(Value::as_str)
                {
                    assistant_tool_ids.insert(id.to_string());
                }
            }
        }
    }

    // Collect tool_call IDs from tool-result messages
    let mut result_tool_ids: HashSet<String> = HashSet::new();
    for msg in messages.iter() {
        if msg.get("role").and_then(Value::as_str) != Some("tool") {
            continue;
        }
        if let Some(id) = msg.get("tool_call_id").and_then(Value::as_str) {
            result_tool_ids.insert(id.to_string());
        }
    }

    // Remove orphaned tool-result messages (no matching assistant tool_call)
    let before_len = messages.len();
    messages.retain(|msg| {
        if msg.get("role").and_then(Value::as_str) != Some("tool") {
            return true;
        }
        if let Some(id) = msg.get("tool_call_id").and_then(Value::as_str) {
            assistant_tool_ids.contains(id)
        } else {
            // No tool_call_id — malformed, remove
            false
        }
    });
    let orphaned_results = before_len - messages.len();

    // Count orphaned tool_calls (assistant has tool_call with no matching result)
    // We don't remove assistant messages, but we log the count
    let orphaned_calls = assistant_tool_ids
        .iter()
        .filter(|id| !result_tool_ids.contains(*id))
        .count();

    if orphaned_results > 0 || orphaned_calls > 0 {
        debug!(
            "stripped {} orphaned tool_result message(s) and found {} orphaned tool_call(s)",
            orphaned_results, orphaned_calls
        );
    }

    (orphaned_results, orphaned_calls)
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
                response_format: None,
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

    /// Review messages about to be compacted and extract important context.
    /// Returns extracted facts as a string, or empty if nothing worth preserving.
    pub async fn flush_to_memory(&self, messages: &[HashMap<String, Value>]) -> Result<String> {
        debug!(
            "pre-compaction flush: reviewing {} messages",
            messages.len()
        );
        let formatted: Vec<String> = messages
            .iter()
            .map(|m| {
                let role = m.get("role").and_then(|v| v.as_str()).unwrap_or("");
                let content = extract_message_text(m.get("content"));
                format!("{}: {}", role, content)
            })
            .collect();

        let messages_text = formatted.join("\n");
        let prompt = PRE_FLUSH_PROMPT.replace("{messages}", &messages_text);

        let llm_messages = vec![Message::user(prompt)];

        let response = self
            .provider
            .chat(ChatRequest {
                messages: llm_messages,
                tools: None,
                model: self.model.as_deref(),
                max_tokens: PRE_FLUSH_MAX_TOKENS,
                temperature: PRE_FLUSH_TEMPERATURE,
                tool_choice: None,
                response_format: None,
            })
            .await?;

        let content = response.content.unwrap_or_default();
        if content.trim().to_ascii_uppercase().starts_with("NOTHING") {
            debug!("pre-compaction flush: nothing worth preserving");
            Ok(String::new())
        } else {
            debug!("pre-compaction flush: extracted {} bytes", content.len());
            Ok(content)
        }
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
                response_format: None,
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
