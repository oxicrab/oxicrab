use crate::agent::context::ContextBuilder;
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
    if !*layer1_fired
        && !any_tools_called
        && !tool_names.is_empty()
        && contains_action_claims(content)
    {
        warn!("hallucination layer 1: action claims detected without tool calls");
        *layer1_fired = true;

        // Inject correction as a system-style tool result
        ContextBuilder::add_tool_result(
            messages,
            "hallucination-check",
            "system",
            "You claimed to perform actions but did not call any tools. \
             Please use the available tools to perform the requested actions.",
            true,
        );
        return TextAction::Continue;
    }

    TextAction::Return
}
