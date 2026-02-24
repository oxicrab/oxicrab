//! User message action intent classification.
//!
//! Classifies user messages as action requests vs. conversational/informational.
//! Used as a structural backstop for hallucination detection: if the user asks
//! for an action and the LLM returns text without calling tools, that's a
//! hallucination regardless of how the LLM phrased it.
//!
//! Two classification layers:
//! 1. **Regex-based** (`classify_action_intent`) — fast, microsecond matching
//!    against known action verb patterns. Handles ~90% of cases.
//! 2. **Embedding-based** (`classify_action_intent_semantic`) — cosine similarity
//!    against prototype action phrases. Catches semantic matches that regex misses
//!    (e.g. "put it on my calendar" ≈ "schedule a reminder").

use regex::Regex;
use std::sync::{LazyLock, Mutex};
use tracing::debug;

use crate::agent::memory::embeddings::{EmbeddingService, cosine_similarity};

/// Imperative action verbs that typically require tool calls.
///
/// Anchored to word boundaries and positioned near the start of the message
/// (after optional polite prefixes like "please", "can you", "go ahead and").
static ACTION_INTENT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:^|\b(?:please|pls|can you|could you|would you|go ahead and|i need you to|i want you to|just|don'?t forget to|do not forget to)\s+)(?:create|add|make|delete|remove|close|complete|finish|mark|schedule|cancel|send|show|list|check|set up|setup|update|edit|modify|search|find|look up|lookup|get|fetch|remind|run|execute|open|move|rename|save|write|read|deploy|enable|disable|start|stop|install|configure|test|build|push|pull|commit|merge)\b",
    )
    .unwrap()
});

/// Negative overrides: informational prefixes that indicate the user is asking
/// *about* an action, not requesting one.
static INFORMATIONAL_OVERRIDE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:tell me about|how (?:do|can|should|would) (?:i|you|we)|how to|explain|what (?:is|are|does|would|if|happens)|describe|why (?:does|did|is|would)|when (?:should|did|does|will)|where (?:is|are|did|does))\b",
    )
    .unwrap()
});

/// Negation patterns that cancel action intent.
/// "Don't forget to" is an exception handled separately (see `NEGATION_EXCEPTION_RE`).
static NEGATION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:^|\b)(?:don'?t|do not|never|stop|avoid|without)\s+(?:create|add|make|delete|remove|close|complete|schedule|cancel|send|update|edit|modify|move|rename)\b",
    )
    .unwrap()
});

/// Exception to negation: "don't forget to" is an action request, not a negation.
static NEGATION_EXCEPTION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:^|\b)(?:don'?t|do not)\s+forget\s+to\b").unwrap());

/// Detects if the LLM response is a clarification question rather than
/// a hallucinated action or evasive non-answer.
static CLARIFICATION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:which\s+(?:task|one|item|file|project)|what\s+(?:should|would you like|do you want|task|name|title)|could you (?:specify|clarify|tell me)|can you (?:specify|clarify)|do you (?:want|mean|prefer)|would you like me to|did you mean|what['']?s the)\b",
    )
    .unwrap()
});

// ---------------------------------------------------------------------------
// Embedding-based semantic intent classification
// ---------------------------------------------------------------------------

/// Prototype action phrases spanning the semantic space of "user requesting
/// an action." Each phrase represents a cluster of similar action requests.
/// Chosen to be diverse — task management, file ops, scheduling, deployment.
const ACTION_PROTOTYPES: &[&str] = &[
    "create a new task for me",
    "delete that file",
    "schedule a reminder for later",
    "send a message to the team",
    "show me my tasks",
    "list everything I have",
    "close that task",
    "update the settings",
    "search for it",
    "run the script now",
    "check the current status",
    "add a new item",
    "remove that entry",
    "open the document",
    "save the changes",
    "move it to the archive",
    "rename this file",
    "find the report",
    "cancel the scheduled job",
    "mark it as done",
    "put it on my calendar",
    "set up the configuration",
    "deploy the update",
    "build the project",
    "install the package",
];

/// Cosine similarity threshold for classifying action intent.
/// BGE-small-en-v1.5 normalized embeddings: semantically similar ≈ 0.75–0.95,
/// somewhat related ≈ 0.55–0.75, unrelated ≈ 0.2–0.5.
const SEMANTIC_ACTION_THRESHOLD: f32 = 0.72;

/// Cached prototype embeddings, computed once on first use.
/// `Mutex<Option<...>>` (not `OnceLock`) so initialization can retry if
/// embeddings weren't ready on the first attempt.
static PROTOTYPE_CACHE: LazyLock<Mutex<Option<Vec<Vec<f32>>>>> = LazyLock::new(|| Mutex::new(None));

/// Classify action intent using embedding cosine similarity.
///
/// Compares the user message against `ACTION_PROTOTYPES` and returns `true`
/// if the max similarity exceeds the threshold. Returns `None` if embeddings
/// are unavailable or fail (caller should fall back to regex).
///
/// Prototype embeddings are computed once and cached for the process lifetime.
pub fn classify_action_intent_semantic(
    text: &str,
    embedding_service: &EmbeddingService,
) -> Option<bool> {
    let trimmed = text.trim();
    if trimmed.len() < 5 {
        return Some(false);
    }

    // Informational and negation overrides apply to semantic classification too
    if INFORMATIONAL_OVERRIDE_RE.is_match(trimmed) {
        return Some(false);
    }
    if NEGATION_RE.is_match(trimmed) && !NEGATION_EXCEPTION_RE.is_match(trimmed) {
        return Some(false);
    }

    // Embed the user's message (LRU-cached in EmbeddingService)
    let query_embedding = embedding_service.embed_query(trimmed).ok()?;

    // Get or initialize prototype embeddings, then compute similarity
    let mut guard = PROTOTYPE_CACHE.lock().ok()?;
    if guard.is_none() {
        let protos: Vec<&str> = ACTION_PROTOTYPES.to_vec();
        let embeddings = embedding_service.embed_texts(&protos).ok()?;
        debug!(
            "initialized {} semantic intent prototypes",
            embeddings.len()
        );
        *guard = Some(embeddings);
    }

    let max_sim = guard
        .as_ref()?
        .iter()
        .map(|proto| cosine_similarity(&query_embedding, proto))
        .fold(f32::NEG_INFINITY, f32::max);

    debug!(
        "semantic intent: score={:.3} threshold={:.3} text={:.60}",
        max_sim, SEMANTIC_ACTION_THRESHOLD, trimmed
    );

    Some(max_sim >= SEMANTIC_ACTION_THRESHOLD)
}

/// Classify whether a user message likely requires tool use.
///
/// Returns `true` if the message contains action intent that should
/// result in tool calls. Returns `false` for conversational, informational,
/// or negated messages.
pub fn classify_action_intent(text: &str) -> bool {
    let trimmed = text.trim();

    // Skip very short messages — likely conversational ("hi", "thanks", "ok")
    if trimmed.len() < 5 {
        return false;
    }

    // Negative override: informational queries containing action verbs
    if INFORMATIONAL_OVERRIDE_RE.is_match(trimmed) {
        return false;
    }

    // Negative override: negated actions ("don't create", "do not delete")
    // Exception: "don't forget to X" is still a positive action request
    if NEGATION_RE.is_match(trimmed) && !NEGATION_EXCEPTION_RE.is_match(trimmed) {
        return false;
    }

    // Positive: contains action intent
    ACTION_INTENT_RE.is_match(trimmed)
}

/// Check if the LLM's response is a legitimate clarification question.
///
/// When the LLM asks for more information before acting, that's not a
/// hallucination — it's appropriate behavior, especially for under-specified
/// requests.
pub fn is_clarification_question(text: &str) -> bool {
    let trimmed = text.trim();

    // Short responses ending with ? are likely clarification questions
    if trimmed.ends_with('?') && trimmed.len() < 200 {
        return true;
    }

    // Explicit clarification patterns
    CLARIFICATION_RE.is_match(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_intent_positive() {
        let cases = [
            "Create a task to feed the cat at 9pm",
            "Add a reminder for tomorrow",
            "Delete the old config file",
            "Please schedule a job for 4pm",
            "Can you send an email to the team?",
            "Show me my tasks",
            "List all open issues",
            "Close that task",
            "Complete the first one",
            "Mark it done",
            "Search for the latest report",
            "Run the deployment script",
            "Update the settings",
            "Can you check the status?",
            "Just create it",
            "Go ahead and delete it",
            "I need you to send the form",
            "Could you find the document?",
            "Would you remove the old entries?",
            "Save the changes",
            "open the file",
            "move that to the archive",
            "pls add a task",
            "don't forget to send the report",
        ];
        for text in cases {
            assert!(classify_action_intent(text), "should be action: {}", text);
        }
    }

    #[test]
    fn test_action_intent_negative() {
        let cases = [
            "How are you?",
            "Thanks!",
            "Good morning",
            "ok",
            "Tell me about creating tasks",
            "How do I delete a file?",
            "How to schedule a cron job",
            "Explain how the search works",
            "What is a task?",
            "What does the delete action do?",
            "What if I schedule it for later?",
            "What happens when you close a task?",
            "Describe the update process",
            "Why does the build fail?",
            "Don't create anything yet",
            "Do not delete that",
            "Never remove the config",
            "When should I run the migration?",
            "Where is the config file?",
            "hi",
            "yes",
            "no",
            "",
        ];
        for text in cases {
            assert!(
                !classify_action_intent(text),
                "should NOT be action: {}",
                text
            );
        }
    }

    #[test]
    fn test_clarification_question_positive() {
        let cases = [
            "Which task would you like me to close?",
            "What should the task name be?",
            "Could you specify which file?",
            "Do you want me to delete all of them?",
            "Did you mean the first or second one?",
            "Sure, but which one?",
            "What's the due date?",
        ];
        for text in cases {
            assert!(
                is_clarification_question(text),
                "should be clarification: {}",
                text
            );
        }
    }

    #[test]
    fn test_clarification_question_negative() {
        let cases = [
            "Created: Feed the cat — due today at 9pm.",
            "Both created:\n• Task A\n• Task B",
            "Done! All set.",
            "I've scheduled the job for 4pm.",
            // Long responses with ? aren't simple clarification
            &format!("{}?", "a".repeat(250)),
        ];
        for text in cases {
            assert!(
                !is_clarification_question(text),
                "should NOT be clarification: {}",
                text
            );
        }
    }

    #[test]
    fn test_action_prototypes_are_valid() {
        // Ensure prototypes list is non-empty and has no duplicates
        assert!(!ACTION_PROTOTYPES.is_empty());
        let mut seen = std::collections::HashSet::new();
        for proto in ACTION_PROTOTYPES {
            assert!(seen.insert(*proto), "duplicate action prototype: {}", proto);
            assert!(proto.len() >= 5, "prototype too short: {}", proto);
        }
    }

    #[test]
    fn test_semantic_threshold_is_reasonable() {
        // Threshold should be in a reasonable range for BGE-small-en-v1.5
        const {
            assert!(SEMANTIC_ACTION_THRESHOLD > 0.5);
            assert!(SEMANTIC_ACTION_THRESHOLD < 0.9);
        }
    }
}
