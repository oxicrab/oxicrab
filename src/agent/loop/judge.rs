//! LLM-as-Judge: poison-resistant semantic gate for tool calls.
//!
//! Adopted from [IronClaw PR #2845](https://github.com/nearai/ironclaw/pull/2845).
//!
//! ## Why
//!
//! oxicrab already gates tool calls with several layers — the
//! exfiltration guard (network-outbound denylist), the prompt guard
//! (injection patterns), the approval workflow (operator click). This
//! adds a *semantic* layer: a small LLM looks at `(tool_name, args,
//! original_user_intent)` and decides whether the call matches what
//! the user asked for. It catches the case where a tool argument was
//! poisoned by injection content (e.g. a fetched web page convinces
//! the agent to email the page contents to an attacker) and pattern-based
//! guards miss the specific shape.
//!
//! ## Poison resistance
//!
//! The judge prompt **only** sees:
//! - the tool name
//! - the tool args (after credential scrubbing)
//! - the user's *original* message text
//!
//! It does NOT see the conversation history or prior tool results.
//! Including those would let an attacker poison the judge with the
//! same injection that poisoned the agent.
//!
//! ## Fail-open by default
//!
//! When the judge LLM call times out, errors, or returns malformed
//! JSON, the verdict defaults to `allow`. The judge is defense-in-depth,
//! not the only gate — silent fail-open is acceptable so a flaky
//! sidecar provider doesn't brick the agent.

use crate::providers::base::{ChatRequest, LLMProvider, Message};
use serde::Deserialize;
use std::time::Duration;
use tokio::time::timeout;
use tracing::{debug, warn};

/// Outcome of one judge call. `allow=true` means proceed; `allow=false`
/// means block the tool call with `reason` shown to the LLM as the
/// tool error so it can re-plan.
#[derive(Debug, Clone)]
pub struct JudgeVerdict {
    pub allow: bool,
    pub reason: String,
}

#[derive(Deserialize)]
struct RawVerdict {
    #[serde(default)]
    verdict: String,
    #[serde(default)]
    reason: String,
}

/// Run the judge for one tool call. Returns `None` when:
/// - the judge is disabled or doesn't cover this tool
/// - `user_intent` is empty (no anchor to compare against — fail open)
/// - the LLM call fails or times out (fail-open)
/// - the response can't be parsed (fail-open)
///
/// Returns `Some(JudgeVerdict { allow: false, ... })` only when the
/// LLM explicitly returned a `block` verdict.
pub async fn judge_tool_call(
    config: &crate::config::JudgeConfig,
    provider: &dyn LLMProvider,
    model: &str,
    tool_name: &str,
    tool_args: &serde_json::Value,
    user_intent: &str,
) -> Option<JudgeVerdict> {
    if !config.enabled {
        return None;
    }
    if !config.covers_tool(tool_name) {
        return None;
    }
    if user_intent.trim().is_empty() {
        debug!("judge: empty user_intent — failing open");
        return None;
    }

    // Scrub credentials from args BEFORE sending to the judge — the
    // judge LLM is a third party from the agent's perspective and
    // shouldn't see secrets.
    let scrubbed_args = oxicrab_safety::scrub_credentials_in_json(tool_args);

    let prompt = build_prompt(tool_name, &scrubbed_args, user_intent);
    let req = ChatRequest {
        model: Some(model.to_string()),
        messages: vec![Message::user(prompt)],
        temperature: Some(0.0),
        max_tokens: config.max_tokens,
        ..Default::default()
    };

    let timeout_duration = Duration::from_secs(config.timeout_seconds.into());
    let chat_future = provider.chat(&req);
    let response = match timeout(timeout_duration, chat_future).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            warn!("judge: provider error: {e} — failing open");
            return None;
        }
        Err(_) => {
            warn!(
                "judge: timed out after {}s — failing open",
                config.timeout_seconds
            );
            return None;
        }
    };
    let text = response.content.unwrap_or_default();
    parse_verdict(&text)
}

fn build_prompt(tool_name: &str, args: &serde_json::Value, user_intent: &str) -> String {
    // Cap inputs so a malicious tool arg or wall-of-text user message
    // can't blow the prompt.
    let user_excerpt = excerpt(user_intent, 1_500);
    let args_excerpt = excerpt(&args.to_string(), 2_000);
    format!(
        "You are a security judge. Decide whether the agent's planned tool call is consistent \
         with what the user asked for. You see only the user's intent, the tool name, and the \
         tool arguments — NOT the agent's reasoning or prior tool results.\n\n\
         User's request: {user_excerpt}\n\n\
         Planned tool call:\n  name: {tool_name}\n  args: {args_excerpt}\n\n\
         Reply with valid JSON only — no preamble, no code fences:\n\
         {{\"verdict\": \"allow\" | \"block\", \"reason\": string ≤200 chars}}\n\n\
         Allow when the call could plausibly serve the user's request. Block only when the \
         call is clearly off-topic, exfiltratory, destructive, or contains data the user did \
         not provide. When in doubt, allow."
    )
}

fn excerpt(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    format!("{}…", &s[..end])
}

fn parse_verdict(text: &str) -> Option<JudgeVerdict> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    // Strip any code fences first.
    let stripped = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```"))
        .map(|s| s.trim_end_matches("```").trim())
        .unwrap_or(text);

    let raw: RawVerdict = if let Ok(v) = serde_json::from_str(stripped) {
        v
    } else if let (Some(start), Some(end)) = (stripped.find('{'), stripped.rfind('}'))
        && end > start
    {
        let candidate = &stripped[start..=end];
        serde_json::from_str::<RawVerdict>(candidate).ok()?
    } else {
        return None;
    };

    let lower = raw.verdict.trim().to_ascii_lowercase();
    let allow = match lower.as_str() {
        "allow" | "ok" | "yes" => true,
        "block" | "deny" | "no" => false,
        _ => {
            debug!(
                "judge: unrecognized verdict '{}' — failing open",
                raw.verdict
            );
            return None;
        }
    };
    let reason = if raw.reason.trim().is_empty() {
        if allow {
            "judge allowed".to_string()
        } else {
            "judge blocked the call".to_string()
        }
    } else {
        excerpt(raw.reason.trim(), 200)
    };
    Some(JudgeVerdict { allow, reason })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_allow_verdict() {
        let v = parse_verdict(r#"{"verdict": "allow", "reason": "matches user request"}"#).unwrap();
        assert!(v.allow);
        assert!(v.reason.contains("matches"));
    }

    #[test]
    fn parses_block_verdict() {
        let v = parse_verdict(r#"{"verdict": "block", "reason": "off-topic"}"#).unwrap();
        assert!(!v.allow);
    }

    #[test]
    fn parses_with_code_fence() {
        let v =
            parse_verdict("```json\n{\"verdict\": \"allow\", \"reason\": \"ok\"}\n```").unwrap();
        assert!(v.allow);
    }

    #[test]
    fn parses_with_leading_prose() {
        let v = parse_verdict("Sure! {\"verdict\": \"block\", \"reason\": \"x\"} done").unwrap();
        assert!(!v.allow);
    }

    #[test]
    fn unrecognized_verdict_fails_open() {
        // Garbage verdict → None → caller fails open.
        assert!(parse_verdict(r#"{"verdict": "maybe", "reason": "..."}"#).is_none());
    }

    #[test]
    fn malformed_json_fails_open() {
        assert!(parse_verdict("not json at all").is_none());
        assert!(parse_verdict("").is_none());
    }

    #[test]
    fn case_insensitive_verdict() {
        assert!(
            parse_verdict(r#"{"verdict": "ALLOW", "reason": ""}"#)
                .unwrap()
                .allow
        );
        assert!(
            !parse_verdict(r#"{"verdict": "Block", "reason": ""}"#)
                .unwrap()
                .allow
        );
    }
}
