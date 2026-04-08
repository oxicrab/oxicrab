//! JSON-based LLM response fixtures for testing.
//!
//! Loads LLM responses from JSON files or strings, making test fixtures
//! editable without recompiling. Supports both sequential and hint-based
//! response matching.

use crate::providers::base::{ChatRequest, LLMProvider, LLMResponse};
use anyhow::{Context, bail};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A single fixture entry that pairs an optional hint with a response.
/// When `hint` is present, the entry matches if the last user message
/// contains the hint substring and the message count meets `min_messages`.
/// When `hint` is absent, the entry participates in sequential fallback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureEntry {
    /// Substring to match in the last user message. When absent,
    /// this entry is only used in sequential order.
    #[serde(default)]
    pub hint: Option<String>,
    /// Minimum message count required for this hint to match.
    /// Useful for sequencing when the same hint appears multiple times.
    #[serde(default)]
    pub min_messages: Option<usize>,
    /// The LLM response to return when this entry matches.
    pub response: LLMResponse,
}

/// An LLM provider that serves responses from JSON fixtures.
///
/// Two matching modes:
/// 1. **Hint matching**: if any entry has a `hint` field, check the last
///    user message for substring matches (with optional `min_messages`).
/// 2. **Sequential fallback**: returns the next unmatched entry in order,
///    cycling back to the last entry when exhausted.
pub struct JsonFixtureLLMProvider {
    entries: Vec<FixtureEntry>,
    sequential_index: AtomicUsize,
}

impl JsonFixtureLLMProvider {
    /// Load fixture entries from a JSON file.
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("reading fixture file {}", path.display()))?;
        Self::from_json(&content)
    }

    /// Parse fixture entries from a JSON string.
    pub fn from_json(json: &str) -> anyhow::Result<Self> {
        let entries: Vec<FixtureEntry> =
            serde_json::from_str(json).context("parsing fixture JSON")?;
        if entries.is_empty() {
            bail!("fixture must contain at least one entry");
        }
        Ok(Self {
            entries,
            sequential_index: AtomicUsize::new(0),
        })
    }

    /// Extract the last user message content from a chat request.
    fn last_user_message(req: &ChatRequest) -> Option<&str> {
        req.messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.as_str())
    }

    /// Try hint-based matching: find the first entry whose hint is a
    /// substring of the last user message and whose min_messages is met.
    fn find_hint_match(&self, req: &ChatRequest) -> Option<&LLMResponse> {
        let user_msg = Self::last_user_message(req)?;
        let msg_count = req.messages.len();

        self.entries.iter().find_map(|entry| {
            let hint = entry.hint.as_deref()?;
            if !user_msg.contains(hint) {
                return None;
            }
            if let Some(min) = entry.min_messages
                && msg_count < min
            {
                return None;
            }
            Some(&entry.response)
        })
    }

    /// Return the next sequential response from entries without hints,
    /// clamping at the last such entry. Falls back to the last entry
    /// overall if no hint-free entries exist.
    fn next_sequential(&self) -> &LLMResponse {
        let sequential: Vec<&FixtureEntry> =
            self.entries.iter().filter(|e| e.hint.is_none()).collect();
        if sequential.is_empty() {
            return &self.entries[self.entries.len() - 1].response;
        }
        let idx = self.sequential_index.fetch_add(1, Ordering::Relaxed);
        let clamped = idx.min(sequential.len() - 1);
        &sequential[clamped].response
    }
}

#[async_trait]
impl LLMProvider for JsonFixtureLLMProvider {
    async fn chat(&self, req: &ChatRequest) -> anyhow::Result<LLMResponse> {
        // Try hint matching first, fall back to sequential
        let response = self
            .find_hint_match(req)
            .unwrap_or_else(|| self.next_sequential());
        Ok(response.clone())
    }

    fn default_model(&self) -> &str {
        "fixture-model"
    }
}

#[cfg(test)]
mod tests;
