use crate::providers::base::Message;
use tracing::warn;

pub use super::helpers::contains_action_claims;

/// Result of [`handle_text_response`] — either continue the loop
/// (a correction was injected) or return the final text to the caller.
pub(super) enum TextAction {
    /// A correction was injected; the loop should `continue`.
    Continue,
    /// The response is final; the caller should return it.
    Return,
}

pub(super) fn record_detection() {
    metrics::counter!("agent_hallucination_detected_total", "layer" => "regex_l1").increment(1);
}

pub(super) fn record_retry_success() {
    metrics::counter!("agent_hallucination_retry_total", "layer" => "regex_l1", "outcome" => "succeeded").increment(1);
}

pub(super) fn record_retry_failure() {
    metrics::counter!("agent_hallucination_retry_total", "layer" => "regex_l1", "outcome" => "failed").increment(1);
}

/// Single-layer hallucination detection: catches action claims without tool calls.
///
/// If the LLM claims to have performed actions (regex match) but never called
/// any tools, inject a correction and retry once.
pub(super) fn handle_text_response(
    content: &str,
    messages: &mut Vec<Message>,
    any_tools_called: bool,
    layer1_fired: &mut bool,
    tool_names: &[String],
) -> TextAction {
    // Layer 1 only: action claims without tool calls. Single retry.
    //
    // Skip when the user's message is a "remember" request — the LLM echoing
    // back "I'll remember..." or "I've saved..." is legitimate, not hallucination.
    let is_remember_echo = messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .is_some_and(|m| {
            let lower = m.content.to_lowercase();
            lower.starts_with("remember ")
                || lower.starts_with("please remember ")
                || lower.starts_with("don't forget ")
                || lower.starts_with("note that ")
                || lower.starts_with("keep in mind ")
                || lower.starts_with("remember that ")
                || lower.starts_with("remember: ")
        });
    if !*layer1_fired
        && !any_tools_called
        && !tool_names.is_empty()
        && !is_remember_echo
        && contains_action_claims(content)
    {
        warn!("hallucination layer 1: action claims detected without tool calls");
        record_detection();
        *layer1_fired = true;

        // Inject correction as a user message so it's valid for all providers
        // (orphan tool_result messages without a matching assistant tool_calls
        // entry are rejected by both Anthropic and OpenAI APIs)
        messages.push(Message::user(
            "You claimed to perform actions but did not call any tools. \
             Please use the available tools to perform the requested actions."
                .to_string(),
        ));
        return TextAction::Continue;
    }

    TextAction::Return
}
