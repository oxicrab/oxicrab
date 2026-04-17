//! Reflexion-style failure reflection.
//!
//! When a tool call returns `is_error = true`, optionally invoke a small
//! LLM call to produce a structured hypothesis about what went wrong and
//! a one-line retry strategy. The reflection is appended to the tool
//! result content for the next iteration so the LLM has explicit
//! guidance, and is persisted to the `tool_reflections` table for
//! offline analysis.
//!
//! Bounded both per-request and per-tool to keep cost predictable.
//!
//! ## Routing
//!
//! The reflection LLM call uses the **same provider and model** as the
//! current agent run. It does **not** consult `model_routing.tasks` for
//! a separate "reflection" task type — that level of indirection isn't
//! configurable today. Operators who want to route reflections to a
//! cheaper model should keep the per-request and per-tool budgets tight
//! and use a small `max_tokens` cap. A future enhancement could add a
//! `reflection` task type to `ResolvedRouting` and resolve it the same
//! way `chat` and `subagent` are resolved.

use crate::agent::memory::memory_db::MemoryDB;
use crate::providers::base::{ChatRequest, LLMProvider, Message};
use crate::safety::LeakDetector;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, warn};

/// Maximum characters of the original error message to send to the
/// reflection LLM call. Prevents one massive error from inflating cost.
const ERROR_EXCERPT_CAP: usize = 500;

/// Maximum characters per LLM-produced field (`hypothesis`,
/// `retry_strategy`). Sanity bound; the prompt also asks for ≤200 chars.
const FIELD_CAP: usize = 400;

/// One reflection entry in memory. Persists to the `tool_reflections`
/// table when `ReflectionConfig.persist_to_db` is true.
#[derive(Debug, Clone)]
pub(super) struct ToolFailureReflection {
    pub tool: String,
    pub action: Option<String>,
    pub attempt_number: u32,
    pub error_excerpt: String,
    pub hypothesis: String,
    pub retry_strategy: String,
}

impl ToolFailureReflection {
    /// Render as a `<reflection>…</reflection>` block to inject into the
    /// next iteration's tool result content.
    pub fn render_block(&self) -> String {
        format!(
            "\n<reflection>\nattempt: {}\nhypothesis: {}\nretry_strategy: {}\n</reflection>",
            self.attempt_number, self.hypothesis, self.retry_strategy
        )
    }
}

/// Per-request budget tracking. One instance per agent run; lives in
/// [`super::AgentLoop::execute_tools`].
#[derive(Default)]
pub(super) struct ReflectionBudget {
    used_total: u8,
    /// Per-(tool, action) reflections used so far in this request.
    used_per_tool: HashMap<(String, Option<String>), u8>,
}

impl ReflectionBudget {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if a reflection can be produced under the configured
    /// caps for this `(tool, action)` pair.
    pub fn allows(
        &self,
        tool: &str,
        action: Option<&str>,
        max_per_request: u8,
        max_per_tool: u8,
    ) -> bool {
        if self.used_total >= max_per_request {
            return false;
        }
        let key = (tool.to_string(), action.map(str::to_string));
        let used = self.used_per_tool.get(&key).copied().unwrap_or(0);
        used < max_per_tool
    }

    /// Record that a reflection was produced for this `(tool, action)`.
    pub fn record(&mut self, tool: &str, action: Option<&str>) {
        self.used_total = self.used_total.saturating_add(1);
        let key = (tool.to_string(), action.map(str::to_string));
        let entry = self.used_per_tool.entry(key).or_insert(0);
        *entry = entry.saturating_add(1);
    }

    pub fn attempt_for(&self, tool: &str, action: Option<&str>) -> u32 {
        let key = (tool.to_string(), action.map(str::to_string));
        u32::from(self.used_per_tool.get(&key).copied().unwrap_or(0)) + 1
    }
}

/// Build the reflection request. Tiny prompt, fixed schema-ish format
/// in the response so parsing is robust.
fn build_reflection_request(
    tool: &str,
    action: Option<&str>,
    error_excerpt: &str,
    config: &crate::config::ReflectionConfig,
    model: &str,
) -> ChatRequest {
    let action_hint = action.map_or_else(String::new, |a| format!(" action={a}"));
    let prompt = format!(
        "A tool call failed. Reflect briefly so the next attempt has a chance.\n\n\
         Tool: {tool}{action_hint}\n\
         Error excerpt:\n```\n{error_excerpt}\n```\n\n\
         Reply in exactly two lines, each ≤200 characters:\n\
         hypothesis: <one-sentence cause>\n\
         retry_strategy: <one concrete instruction for the next call>"
    );

    ChatRequest {
        model: Some(model.to_string()),
        messages: vec![Message::user(prompt)],
        temperature: Some(config.temperature),
        max_tokens: config.max_tokens,
        ..Default::default()
    }
}

fn cap(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// Parse the two-line response. Tolerates extra whitespace, list-style
/// prefixes (`- `, `* `, `1. `), case differences (`Hypothesis:`,
/// `RETRY_STRATEGY:`), and a single bold/italic marker around the
/// label. Returns `None` when neither field can be extracted.
fn parse_reflection(response: &str) -> Option<(String, String)> {
    let mut hypothesis = String::new();
    let mut retry_strategy = String::new();
    for line in response.lines() {
        let trimmed = strip_line_prefix(line.trim());
        let lower = trimmed.to_ascii_lowercase();
        if let Some(idx) = lower.find("hypothesis:") {
            let rest = &trimmed[idx + "hypothesis:".len()..];
            hypothesis = cap(strip_value_decoration(rest.trim()), FIELD_CAP);
        } else if let Some(idx) = lower.find("retry_strategy:") {
            let rest = &trimmed[idx + "retry_strategy:".len()..];
            retry_strategy = cap(strip_value_decoration(rest.trim()), FIELD_CAP);
        }
    }
    if hypothesis.is_empty() && retry_strategy.is_empty() {
        return None;
    }
    if hypothesis.is_empty() {
        hypothesis = "(no hypothesis returned)".to_string();
    }
    if retry_strategy.is_empty() {
        retry_strategy = "(no retry strategy returned)".to_string();
    }
    Some((hypothesis, retry_strategy))
}

/// Strip leading and trailing markdown decoration (`**`, `__`, `*`, `_`)
/// from a captured value. Used after the label is matched so the
/// extracted value is the human-readable text only.
fn strip_value_decoration(s: &str) -> &str {
    let s = s
        .trim_start_matches("**")
        .trim_start_matches("__")
        .trim_start_matches('*')
        .trim_start_matches('_')
        .trim_start();
    s.trim_end_matches("**")
        .trim_end_matches("__")
        .trim_end_matches('*')
        .trim_end_matches('_')
        .trim_end()
}

/// Strip a leading list marker (`- `, `* `, `+ `, `1.`, `2.`, …),
/// a leading single bold/italic marker (`*`, `_`, `**`), and any
/// surrounding whitespace. Returns the cleaned slice.
fn strip_line_prefix(s: &str) -> &str {
    let mut s = s.trim_start();
    // List marker.
    if let Some(rest) = s
        .strip_prefix("- ")
        .or_else(|| s.strip_prefix("* "))
        .or_else(|| s.strip_prefix("+ "))
    {
        s = rest;
    } else if let Some((digits, rest)) =
        s.find(|c: char| !c.is_ascii_digit()).map(|i| s.split_at(i))
        && digits.chars().all(|c| c.is_ascii_digit())
        && !digits.is_empty()
        && let Some(rest) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") "))
    {
        s = rest;
    }
    // Single bold/italic marker.
    s = s
        .strip_prefix("**")
        .or_else(|| s.strip_prefix("__"))
        .or_else(|| s.strip_prefix('*'))
        .or_else(|| s.strip_prefix('_'))
        .unwrap_or(s);
    s.trim_start()
}

/// Generate one reflection for a failed tool call. Returns `None` when
/// disabled, over budget, or the LLM call fails.
///
/// Output is sanitized through the leak detector before persisting and
/// before being returned for injection into the agent context.
///
/// When `db` is `Some`, token usage from the reflection LLM call is
/// recorded into `llm_cost_log` with caller `"reflection"` so operators
/// can isolate reflection cost from main-loop cost.
#[allow(clippy::too_many_arguments)]
pub(super) async fn reflect_on_failure(
    config: &crate::config::ReflectionConfig,
    budget: &mut ReflectionBudget,
    provider: &dyn LLMProvider,
    model: &str,
    leak_detector: &LeakDetector,
    db: Option<&Arc<MemoryDB>>,
    request_id: &str,
    tool: &str,
    action: Option<&str>,
    error_message: &str,
) -> Option<ToolFailureReflection> {
    if !config.enabled {
        return None;
    }
    if !config.covers_tool(tool) {
        debug!(
            "reflection: tool='{}' filtered by allowed/blocked list",
            tool
        );
        return None;
    }
    if !budget.allows(tool, action, config.max_per_request, config.max_per_tool) {
        debug!(
            "reflection: budget exhausted for tool='{}' action={:?}",
            tool, action
        );
        return None;
    }

    let error_excerpt = cap(&leak_detector.redact(error_message), ERROR_EXCERPT_CAP);
    let req = build_reflection_request(tool, action, &error_excerpt, config, model);
    metrics::counter!("oxicrab_reflection_triggered_total",
        "tool" => tool.to_string(),
        "action" => action.unwrap_or("").to_string(),
    )
    .increment(1);

    let response = match provider.chat(&req).await {
        Ok(r) => r,
        Err(e) => {
            warn!("reflection: LLM call failed for tool='{}': {}", tool, e);
            metrics::counter!("oxicrab_reflection_llm_error_total",
                "tool" => tool.to_string(),
            )
            .increment(1);
            return None;
        }
    };

    // Record reflection token usage with a distinct caller tag so
    // operators can break out reflection cost from main-loop cost in
    // `oxicrab stats tokens`.
    if let Some(db) = db {
        let actual_model = response
            .actual_model
            .as_deref()
            .unwrap_or(model)
            .to_string();
        let input = response.input_tokens.unwrap_or(0);
        let output = response.output_tokens.unwrap_or(0);
        let cache_create = response.cache_creation_input_tokens.unwrap_or(0);
        let cache_read = response.cache_read_input_tokens.unwrap_or(0);
        let req_id = request_id.to_string();
        let db = db.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = db.record_tokens(
                &actual_model,
                input,
                output,
                cache_create,
                cache_read,
                "reflection",
                Some(&req_id),
            ) {
                warn!("reflection: failed to record token usage: {e}");
            }
        });
    }

    let body = response.content.unwrap_or_default();
    let Some((hypothesis, retry_strategy)) = parse_reflection(&body) else {
        warn!(
            "reflection: failed to parse LLM response for tool='{}': {}",
            tool,
            cap(&body, 200)
        );
        return None;
    };

    let hypothesis = leak_detector.redact(&hypothesis);
    let retry_strategy = leak_detector.redact(&retry_strategy);
    let attempt_number = budget.attempt_for(tool, action);
    budget.record(tool, action);

    let reflection = ToolFailureReflection {
        tool: tool.to_string(),
        action: action.map(str::to_string),
        attempt_number,
        error_excerpt,
        hypothesis,
        retry_strategy,
    };

    debug!(
        "reflection produced: tool='{}' action={:?} attempt={} hypothesis_len={}",
        tool,
        action,
        attempt_number,
        reflection.hypothesis.len()
    );

    let _ = request_id; // request_id is used by the persistence wrapper, not here
    Some(reflection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_respects_per_request_cap() {
        let mut b = ReflectionBudget::new();
        assert!(b.allows("a", None, 2, 5));
        b.record("a", None);
        assert!(b.allows("a", Some("x"), 2, 5));
        b.record("a", Some("x"));
        assert!(
            !b.allows("b", None, 2, 5),
            "should hit per-request cap of 2"
        );
    }

    #[test]
    fn budget_respects_per_tool_cap() {
        let mut b = ReflectionBudget::new();
        assert!(b.allows("a", Some("x"), 10, 1));
        b.record("a", Some("x"));
        assert!(
            !b.allows("a", Some("x"), 10, 1),
            "should hit per-tool cap of 1"
        );
        assert!(
            b.allows("a", Some("y"), 10, 1),
            "different action should still be allowed"
        );
    }

    #[test]
    fn parse_handles_two_line_response() {
        let r = parse_reflection("hypothesis: file not found\nretry_strategy: read parent dir");
        assert_eq!(
            r,
            Some(("file not found".to_string(), "read parent dir".to_string()))
        );
    }

    #[test]
    fn parse_tolerates_missing_field() {
        let r = parse_reflection("hypothesis: only this");
        assert!(r.is_some());
        let (h, s) = r.unwrap();
        assert_eq!(h, "only this");
        assert!(s.starts_with("(no retry"));
    }

    #[test]
    fn parse_returns_none_when_blank() {
        let r = parse_reflection("blah blah\nno fields here");
        assert!(r.is_none());
    }

    #[test]
    fn cap_respects_char_boundaries() {
        let s = "héllo wörld";
        let capped = cap(s, 6);
        assert!(capped.is_char_boundary(capped.find('…').unwrap_or(0)));
    }

    #[test]
    fn cap_with_zero_returns_empty_without_panic() {
        // Regression: previous version computed `max - 1` later in the
        // function, which would underflow for max == 0. The early-return
        // guard prevents that.
        assert_eq!(cap("anything", 0), "");
        assert_eq!(cap("", 0), "");
    }

    #[test]
    fn cap_handles_short_input() {
        assert_eq!(cap("hi", 100), "hi");
        assert_eq!(cap("", 100), "");
    }

    #[test]
    fn parse_is_case_insensitive() {
        let r = parse_reflection("Hypothesis: cause\nRETRY_STRATEGY: do thing");
        assert_eq!(r, Some(("cause".to_string(), "do thing".to_string())));
    }

    #[test]
    fn parse_strips_list_markers_and_bold() {
        let r = parse_reflection("- **hypothesis:** something\n* retry_strategy: do thing");
        assert_eq!(r, Some(("something".to_string(), "do thing".to_string())));
    }

    #[test]
    fn parse_strips_numbered_list() {
        let r = parse_reflection("1. hypothesis: cause\n2. retry_strategy: do thing");
        assert_eq!(r, Some(("cause".to_string(), "do thing".to_string())));
    }

    #[test]
    fn budget_allows_returns_false_when_exhausted() {
        let mut b = ReflectionBudget::new();
        b.record("shell", Some("execute"));
        assert!(!b.allows("shell", Some("execute"), 1, 1));
        assert!(b.allows("shell", Some("write"), 5, 1));
    }
}
