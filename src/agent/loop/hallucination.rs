use super::intent;
use crate::agent::context::ContextBuilder;
use crate::agent::memory::memory_db::MemoryDB;
use crate::providers::base::Message;
use tracing::{debug, warn};

pub use super::helpers::{
    contains_action_claims, is_false_no_tools_claim, mentions_any_tool, mentions_multiple_tools,
};

/// Result of [`handle_text_response`] — either continue the loop
/// (a nudge/correction was injected) or return the final text to the caller.
pub(super) enum TextAction {
    /// A nudge or correction was injected; the loop should `continue`.
    Continue,
    /// The response is final; the caller should return it.
    Return,
}

/// Tracks how many corrections each hallucination detection layer has sent,
/// preventing infinite correction loops while allowing each layer its own budget.
pub(super) struct CorrectionState {
    /// Layer 0 (false no-tools claim) correction count. Capped at
    /// `MAX_LAYER0_CORRECTIONS` — if the LLM insists it has no tools after
    /// that many corrections, give up.
    pub(super) layer0_count: u8,
    /// Whether Layer 1 (regex action claims) has fired. Fires once — a second
    /// hallucination after correction is accepted as the LLM's final answer.
    pub(super) layer1_fired: bool,
    /// Whether Layer 2 (intent mismatch) has fired. Independent of Layer 1,
    /// so if L1 corrects first and fails, L2 still gets its own attempt.
    pub(super) layer2_fired: bool,
    /// Whether Layer 3 (action gap / partial hallucination) has fired. Catches
    /// cases where some tools were called but the LLM claims actions for tools
    /// it never used.
    pub(super) layer3_fired: bool,
}

impl CorrectionState {
    pub fn new() -> Self {
        Self {
            layer0_count: 0,
            layer1_fired: false,
            layer2_fired: false,
            layer3_fired: false,
        }
    }
}

/// Maximum corrections for Layer 0 (false no-tools claims).
pub(super) const MAX_LAYER0_CORRECTIONS: u8 = 2;

/// Multi-layer hallucination detection and correction.
///
/// Detection is layered:
/// 1. False "no tools" claim detection (LLM says it has no tools)
/// 2. Regex-based action claim detection (fast-path for obvious hallucinations)
/// 3. Intent-based structural detection (backstop: user asked for action + no tools called)
/// 4. Action gap detection (partial hallucination: some tools called, claims actions for uncalled tools)
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_text_response(
    content: &str,
    messages: &mut Vec<Message>,
    reasoning_content: Option<&str>,
    any_tools_called: bool,
    state: &mut CorrectionState,
    tool_names: &[String],
    tools_used: &[String],
    user_has_action_intent: bool,
    db: Option<&MemoryDB>,
    request_id: Option<&str>,
    tool_mention_ac: Option<&aho_corasick::AhoCorasick>,
) -> TextAction {
    // Layer 0: Detect false "no tools" claims and retry with correction.
    // The LLM is factually wrong about not having tools — correct up to
    // MAX_LAYER0_CORRECTIONS times before giving up.
    if !tool_names.is_empty() && is_false_no_tools_claim(content) {
        if state.layer0_count >= MAX_LAYER0_CORRECTIONS {
            warn!(
                "False no-tools claim persists after {} corrections, giving up",
                MAX_LAYER0_CORRECTIONS
            );
            return TextAction::Return;
        }
        warn!(
            "False no-tools claim detected: LLM claims tools unavailable but {} tools are registered (correction {}/{})",
            tool_names.len(),
            state.layer0_count + 1,
            MAX_LAYER0_CORRECTIONS
        );
        if let Some(db) = db
            && let Err(e) = db.record_intent_event(
                "hallucination",
                None,
                None,
                Some("layer0_false_no_tools"),
                content,
                request_id,
            )
        {
            debug!("failed to record hallucination metric: {}", e);
        }
        ContextBuilder::add_assistant_message(
            messages,
            Some(content),
            None,
            reasoning_content,
            None,
        );
        let tool_list = tool_names.join(", ");
        messages.push(Message::user(format!(
            "[Internal: Your previous response was not delivered. \
             You DO have tools available: {tool_list}. \
             Call the appropriate tool now. Do NOT apologize or reference this correction.]"
        )));
        state.layer0_count += 1;
        return TextAction::Continue;
    }

    // Layer 1: Regex-based action claim detection (fast path)
    //
    // When no tools have been called, check for action claims and multi-tool
    // mentions. When tools HAVE been called, action claims (e.g. "I've updated
    // the config") are likely legitimate summaries, so skip that check — but
    // still catch mentions of tools that were never actually called (the LLM
    // embellishing what it did).
    if !state.layer1_fired {
        let trigger = if any_tools_called {
            // Only check for mentions of uncalled tools
            let uncalled: Vec<String> = tool_names
                .iter()
                .filter(|name| !tools_used.iter().any(|u| u == *name))
                .cloned()
                .collect();
            mentions_multiple_tools(content, &uncalled, None)
        } else {
            contains_action_claims(content)
                || mentions_multiple_tools(content, tool_names, tool_mention_ac)
        };
        if trigger {
            warn!("Action hallucination detected: LLM claims actions but tools were not called");
            if let Some(db) = db
                && let Err(e) = db.record_intent_event(
                    "hallucination",
                    None,
                    None,
                    Some("layer1_regex"),
                    content,
                    request_id,
                )
            {
                debug!("failed to record hallucination metric: {}", e);
            }
            ContextBuilder::add_assistant_message(
                messages,
                Some(content),
                None,
                reasoning_content,
                None,
            );
            messages.push(Message::user(
                "[Internal: Your previous response was not delivered to the user. \
                 You must call the appropriate tool to perform the requested action. \
                 Do NOT apologize or mention any previous attempt — the user has no \
                 knowledge of it. Just call the tool and respond normally.]"
                    .to_string(),
            ));
            state.layer1_fired = true;
            return TextAction::Continue;
        }
    }

    // Layer 2: Intent-based structural detection (robust backstop)
    // If the user asked for an action and the LLM returned text without
    // calling tools AND the response isn't a clarification question,
    // this is a hallucination regardless of phrasing.
    if !state.layer2_fired
        && !any_tools_called
        && !tool_names.is_empty()
        && user_has_action_intent
        && !intent::is_clarification_question(content)
        && !is_legitimate_refusal(content)
    {
        warn!("Intent mismatch: user requested action but LLM returned text without calling tools");
        if let Some(db) = db
            && let Err(e) = db.record_intent_event(
                "hallucination",
                None,
                None,
                Some("layer2_intent"),
                content,
                request_id,
            )
        {
            debug!("failed to record hallucination metric: {}", e);
        }
        ContextBuilder::add_assistant_message(
            messages,
            Some(content),
            None,
            reasoning_content,
            None,
        );
        messages.push(Message::user(
            "[Internal: Your previous response was not delivered to the user. \
             The user is requesting an action that requires a tool call. \
             Call the appropriate tool now. Do NOT apologize or reference \
             this correction — the user has no knowledge of it.]"
                .to_string(),
        ));
        state.layer2_fired = true;
        return TextAction::Continue;
    }

    // Layer 3: Partial hallucination detection (action gap)
    //
    // When tools HAVE been called, Layers 1 and 2 are largely disabled to avoid
    // flagging legitimate summaries. But the LLM may call SOME tools (e.g. gmail)
    // then hallucinate actions for OTHER tools (e.g. calendar). Detect this by
    // checking: does the response mention uncalled tools AND contain action claims?
    if !state.layer3_fired && any_tools_called && !tool_names.is_empty() {
        let uncalled: Vec<String> = tool_names
            .iter()
            .filter(|name| !tools_used.iter().any(|u| u == *name))
            .cloned()
            .collect();
        let mentions_uncalled = mentions_any_tool(content, &uncalled);
        if mentions_uncalled && contains_action_claims(content) {
            warn!(
                "Partial hallucination: LLM called some tools but claims actions for uncalled tools"
            );
            if let Some(db) = db
                && let Err(e) = db.record_intent_event(
                    "hallucination",
                    None,
                    None,
                    Some("layer3_action_gap"),
                    content,
                    request_id,
                )
            {
                debug!("failed to record hallucination metric: {}", e);
            }
            ContextBuilder::add_assistant_message(
                messages,
                Some(content),
                None,
                reasoning_content,
                None,
            );
            messages.push(Message::user(
                "[Internal: Your previous response was not delivered to the user. \
                 You described performing actions but did not call the required tools. \
                 Call the appropriate tool now to complete the remaining actions. \
                 Do NOT apologize or mention any previous attempt.]"
                    .to_string(),
            ));
            state.layer3_fired = true;
            return TextAction::Continue;
        }
    }

    TextAction::Return
}

/// Check if the response is a legitimate refusal to perform an action
/// (as opposed to a hallucinated "I did it" without calling tools).
fn is_legitimate_refusal(content: &str) -> bool {
    let lower = content.to_lowercase();
    let refusal_patterns = [
        "i don't have a tool",
        "i don't have access",
        "no tool available",
        "isn't configured",
        "not configured",
        "i'm unable to",
        "i am unable to",
        "i cannot perform",
        "i can't perform",
        "this requires manual",
        "requires manual",
        "not supported",
        "beyond my capabilities",
        "outside my capabilities",
        "i don't have the ability",
    ];
    refusal_patterns.iter().any(|p| lower.contains(p))
}
