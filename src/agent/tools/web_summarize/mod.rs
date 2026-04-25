//! `web_fetch_summary` — fetch a URL and return a small-model summary
//! instead of the full extracted text. Adopted from
//! [IronClaw PR #2959](https://github.com/nearai/ironclaw/pull/2959).
//!
//! The cheap-LLM step avoids dragging tens of KB of HTML into the
//! main agent context just so the model can answer "what does this
//! page say?". Routes through the configured `web_summary` task
//! override in `model_routing.tasks` when set; falls back to the
//! main provider/model otherwise.
//!
//! A 15-minute LRU cache keyed by `(url, prompt, summary_max_tokens)`
//! suppresses the second hit on the same prompt. Clears at process
//! restart.

use crate::providers::base::{ChatRequest, LLMProvider, Message};
use async_trait::async_trait;
use lru::LruCache;
use oxicrab_core::actions;
use oxicrab_core::tools::base::{
    ExecutionContext, Tool, ToolCapabilities, ToolCategory, ToolConcurrency, ToolResult,
};
use oxicrab_tools_web::web::WebFetchTool;
use serde_json::{Value, json};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{debug, warn};

#[cfg(test)]
mod tests;

const DEFAULT_PROMPT: &str = "Summarize the key points of this content concisely.";
const DEFAULT_INPUT_CAP: usize = 12_000;
const DEFAULT_SUMMARY_TOKENS: u32 = 600;
const CACHE_TTL: Duration = Duration::from_secs(15 * 60);
const CACHE_CAPACITY: usize = 64;

#[derive(Clone)]
struct CachedSummary {
    body: String,
    inserted_at: Instant,
}

pub struct WebFetchSummaryTool {
    /// Provider used to summarise. Pre-resolved via the
    /// `web_summary` task override when set; otherwise the main
    /// provider.
    provider: Arc<dyn LLMProvider>,
    /// Model to summarise with (matches `provider`).
    model: String,
    /// Underlying fetch tool — reuses oxicrab-tools-web's HTML
    /// extraction + SSRF validation.
    fetch: Arc<WebFetchTool>,
    /// Default cap on input chars sent to the LLM.
    input_cap: usize,
    /// 15-minute LRU cache of `(url, prompt, summary_max_tokens) → summary`.
    cache: Arc<Mutex<LruCache<String, CachedSummary>>>,
}

impl WebFetchSummaryTool {
    pub fn new(provider: Arc<dyn LLMProvider>, model: String, fetch: Arc<WebFetchTool>) -> Self {
        Self {
            provider,
            model,
            fetch,
            input_cap: DEFAULT_INPUT_CAP,
            cache: Arc::new(Mutex::new(LruCache::new(
                NonZeroUsize::new(CACHE_CAPACITY).expect("CACHE_CAPACITY > 0"),
            ))),
        }
    }

    fn cache_key(url: &str, prompt: &str, max_tokens: u32) -> String {
        format!("{url}\u{1f}{prompt}\u{1f}{max_tokens}")
    }

    fn cache_get(&self, key: &str) -> Option<String> {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = cache.get(key)?;
        if entry.inserted_at.elapsed() > CACHE_TTL {
            cache.pop(key);
            return None;
        }
        Some(entry.body.clone())
    }

    fn cache_put(&self, key: String, body: String) {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.put(
            key,
            CachedSummary {
                body,
                inserted_at: Instant::now(),
            },
        );
    }

    /// Pull `extracted_text` out of the inner `web_fetch` JSON envelope.
    /// Falls back to the raw content when the JSON shape doesn't match
    /// what `WebFetchTool::execute` produces today.
    fn extract_content(fetch_result: &ToolResult) -> Option<String> {
        if fetch_result.is_error {
            return None;
        }
        let parsed: Value = serde_json::from_str(&fetch_result.content).ok()?;
        parsed
            .get("extracted_text")
            .or_else(|| parsed.get("text"))
            .or_else(|| parsed.get("content"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }

    fn truncate_input(text: &str, cap: usize) -> String {
        if text.chars().count() <= cap {
            return text.to_string();
        }
        let mut end = cap;
        while !text.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        format!("{}…", &text[..end])
    }
}

#[async_trait]
impl Tool for WebFetchSummaryTool {
    fn name(&self) -> &'static str {
        "web_fetch_summary"
    }

    fn description(&self) -> &'static str {
        "Fetch a URL and return a small-model summary instead of the full extracted text. \
         Use when the user wants 'what does this page say' / 'summarise <url>' rather than \
         a quote or specific extraction. The summary is produced by the configured \
         `web_summary` task model (cheap by default). Identical (url, prompt) requests \
         are cached for 15 minutes."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "URL to fetch. Must be http(s)."
                },
                "prompt": {
                    "type": "string",
                    "description": "Question or framing for the summariser. Defaults to a generic 'summarise the key points' instruction."
                },
                "max_input_chars": {
                    "type": "integer",
                    "minimum": 1000,
                    "maximum": 60_000,
                    "description": "Cap on extracted page chars sent to the summariser. Default 12000."
                },
                "summary_max_tokens": {
                    "type": "integer",
                    "minimum": 64,
                    "maximum": 4096,
                    "description": "Cap on summary length in tokens. Default 600."
                }
            },
            "required": ["url"]
        })
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            actions: actions![summarize: ro],
            category: ToolCategory::Web,
            concurrency: ToolConcurrency::ReadOnly,
            network_outbound: true,
            ..Default::default()
        }
    }

    async fn execute(&self, params: Value, ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
        let Some(url) = params.get("url").and_then(|v| v.as_str()) else {
            return Ok(ToolResult::error(
                "web_fetch_summary: missing 'url'".to_string(),
            ));
        };
        let prompt = params
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_PROMPT)
            .trim();
        let prompt = if prompt.is_empty() {
            DEFAULT_PROMPT
        } else {
            prompt
        };
        let input_cap = params
            .get("max_input_chars")
            .and_then(serde_json::Value::as_u64)
            .map_or(self.input_cap, |n| (n as usize).clamp(1_000, 60_000));
        let summary_tokens = params
            .get("summary_max_tokens")
            .and_then(serde_json::Value::as_u64)
            .map_or(DEFAULT_SUMMARY_TOKENS, |n| (n as u32).clamp(64, 4096));

        let key = Self::cache_key(url, prompt, summary_tokens);
        if let Some(cached) = self.cache_get(&key) {
            debug!("web_fetch_summary: cache hit for {url}");
            return Ok(ToolResult::new(format!("[cached] {cached}")));
        }

        // Delegate the fetch through the existing WebFetchTool so SSRF
        // validation and HTML→markdown extraction stay shared.
        let fetch_params = json!({
            "url": url,
            "extractMode": "markdown",
        });
        let fetch_result = self.fetch.execute(fetch_params, ctx).await?;
        if fetch_result.is_error {
            return Ok(fetch_result);
        }
        let Some(content) = Self::extract_content(&fetch_result) else {
            return Ok(ToolResult::error(format!(
                "web_fetch_summary: could not parse extracted text from web_fetch response (url={url})"
            )));
        };
        let trimmed = Self::truncate_input(&content, input_cap);

        let user_msg = format!(
            "Source URL: {url}\n\nInstruction: {prompt}\n\nContent:\n```\n{trimmed}\n```\n\n\
             Reply with the summary only, no preamble."
        );
        let req = ChatRequest {
            model: Some(self.model.clone()),
            messages: vec![Message::user(user_msg)],
            temperature: Some(0.2),
            max_tokens: summary_tokens,
            ..Default::default()
        };
        match self.provider.chat(&req).await {
            Ok(resp) => {
                let summary = resp.content.unwrap_or_default().trim().to_string();
                if summary.is_empty() {
                    return Ok(ToolResult::error(
                        "web_fetch_summary: summariser returned empty response".to_string(),
                    ));
                }
                self.cache_put(key, summary.clone());
                Ok(ToolResult::new(summary))
            }
            Err(e) => {
                warn!("web_fetch_summary: provider error: {e}");
                Ok(ToolResult::error(format!(
                    "web_fetch_summary: summariser failed: {e}"
                )))
            }
        }
    }
}
