//! Complexity-aware message routing: scores each inbound user message across
//! 7 dimensions using Aho-Corasick + regex (sub-millisecond, zero API calls)
//! and maps the score to a model tier for cost-efficient routing.

#[cfg(test)]
mod tests;

use aho_corasick::AhoCorasick;
use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

use crate::config::schema::{ComplexityRoutingConfig, ComplexityWeights};

/// Pre-built scoring engine constructed once at startup. Holds AC automata and
/// config references so per-message scoring is allocation-free hot-path only.
pub struct ComplexityScorer {
    reasoning_ac: AhoCorasick,
    technical_ac: AhoCorasick,
    greeting_ac: AhoCorasick,
    filler_ac: AhoCorasick,
    weights: ComplexityWeights,
    light_standard: f64,
    standard_heavy: f64,
    light_tier: String,
    medium_tier: String,
    heavy_tier: String,
}

/// Per-dimension breakdown for a single message.
#[derive(Debug, Clone)]
pub struct ComplexityScore {
    /// Final composite score (0.0-1.0) after weighted sum + sigmoid.
    pub composite: f64,
    /// Whether a force override was applied.
    pub forced: Option<&'static str>,
}

// ---------------------------------------------------------------------------
// Keyword lists for AC automata
// ---------------------------------------------------------------------------

const REASONING_KEYWORDS: &[&str] = &[
    "analyze",
    "analyse",
    "synthesize",
    "evaluate",
    "compare",
    "contrast",
    "explain why",
    "step by step",
    "trade-off",
    "tradeoff",
    "trade off",
    "pros and cons",
    "reason about",
    "reasoning",
    "think through",
    "break down",
    "critically",
    "implications",
    "consequences",
    "hypothesis",
    "derive",
    "prove",
    "justify",
    "argue",
];

const TECHNICAL_VOCABULARY: &[&str] = &[
    "algorithm",
    "architecture",
    "api",
    "database",
    "schema",
    "migration",
    "middleware",
    "authentication",
    "authorization",
    "encryption",
    "concurrency",
    "async",
    "mutex",
    "deadlock",
    "microservice",
    "kubernetes",
    "docker",
    "terraform",
    "pipeline",
    "compiler",
    "runtime",
    "garbage collection",
    "memory leak",
    "stack overflow",
    "race condition",
];

// ---------------------------------------------------------------------------
// Exact-match sets for greeting/filler force-override (avoids AC substring
// false positives like "nah analyze" matching "nah" as filler)
// ---------------------------------------------------------------------------

static GREETING_SET: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "hi",
        "hello",
        "hey",
        "yo",
        "sup",
        "thanks",
        "thank you",
        "thx",
        "ty",
        "bye",
        "goodbye",
        "good morning",
        "good evening",
        "good night",
        "good afternoon",
        "gm",
        "gn",
    ]
    .into_iter()
    .collect()
});

static FILLER_SET: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "ok",
        "okay",
        "sure",
        "yes",
        "no",
        "yep",
        "nope",
        "yeah",
        "nah",
        "cool",
        "nice",
        "great",
        "awesome",
        "got it",
        "understood",
        "alright",
        "right",
        "fine",
        "lol",
        "haha",
        "lmao",
        "hmm",
        "hm",
        "ah",
        "oh",
        "uh",
        "um",
        "wow",
        "k",
        "kk",
    ]
    .into_iter()
    .collect()
});

// ---------------------------------------------------------------------------
// Regex patterns (compiled once via LazyLock)
// ---------------------------------------------------------------------------

/// Simple questions: "what is", "when is", "where is", "who is"
static SIMPLE_QUESTION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(what|when|where|who|which)\s+(is|are|was|were|do|does|did)\b").unwrap()
});

/// Comparative questions: "which is better", "difference between", "vs"
static COMPARATIVE_QUESTION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(which\s+(is|are)\s+(better|best|faster|preferred)|difference\s+between|compared?\s+to|\bvs\.?\b)",
    )
    .unwrap()
});

/// Analytical questions: "why does", "how does X work", "what causes"
static ANALYTICAL_QUESTION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(why\s+(does|do|is|are|did|would|should)|how\s+does\b.*\bwork|what\s+causes|what\s+happens\s+when)")
        .unwrap()
});

/// Multi-part questions: multiple question marks or "and also"
static MULTIPART_QUESTION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(\?[^?]*\?|and\s+also\b|additionally\b.*\?)").unwrap());

/// Code fences: triple-backtick blocks
static CODE_FENCE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"```[\s\S]*?```").unwrap());

/// Inline code: single-backtick spans
static INLINE_CODE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`[^`]+`").unwrap());

/// Code-like patterns: function calls, method chains, imports
static CODE_PATTERN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(\w+\(.*\)|\w+\.\w+\(|fn\s+\w+|def\s+\w+|class\s+\w+|import\s+\w+|use\s+\w+::|#\[derive|pub\s+(fn|struct|enum|trait))")
        .unwrap()
});

/// Sequential markers: "first", "then", "next", "finally", numbered lists
static SEQUENTIAL_MARKER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?im)(^\s*\d+[\.\)]\s|\bfirst\b.*\bthen\b|\bstep\s+\d|\bnext\b.*\bthen\b|\bfinally\b)",
    )
    .unwrap()
});

/// Imperative verbs at start of sentences
static IMPERATIVE_VERB_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^\s*(create|add|make|build|implement|write|design|set up|configure|deploy|install|remove|delete|update|modify|refactor|optimize|test|fix|debug|review|check|ensure|verify)\b")
        .unwrap()
});

// ---------------------------------------------------------------------------
// Force-override thresholds
// ---------------------------------------------------------------------------

const REASONING_FORCE_THRESHOLD: usize = 2;
/// Byte length (not character count) — multi-byte UTF-8 content hits this sooner.
/// Consistent with D1 scoring which also uses byte length.
const LENGTH_FORCE_THRESHOLD: usize = 50_000;

impl ComplexityScorer {
    pub fn new(config: &ComplexityRoutingConfig) -> Self {
        let reasoning_ac = AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .build(REASONING_KEYWORDS)
            .expect("reasoning AC automaton should build");
        let technical_ac = AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .build(TECHNICAL_VOCABULARY)
            .expect("technical AC automaton should build");

        // Reuse quality.rs GREETINGS/FILLER pattern lists for dimension 7.
        // We can't import the private constants directly, so we duplicate the
        // word lists here (they're small and stable).
        let greetings: &[&str] = &[
            "hi",
            "hello",
            "hey",
            "yo",
            "sup",
            "thanks",
            "thank you",
            "thx",
            "ty",
            "bye",
            "goodbye",
            "good morning",
            "good evening",
            "good night",
            "good afternoon",
            "gm",
            "gn",
        ];
        let filler: &[&str] = &[
            "ok",
            "okay",
            "sure",
            "yes",
            "no",
            "yep",
            "nope",
            "yeah",
            "nah",
            "cool",
            "nice",
            "great",
            "awesome",
            "got it",
            "understood",
            "alright",
            "right",
            "fine",
            "lol",
            "haha",
            "lmao",
            "hmm",
            "hm",
            "ah",
            "oh",
            "uh",
            "um",
            "wow",
            "k",
            "kk",
        ];

        let greeting_ac = AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .build(greetings)
            .expect("greeting AC automaton should build");

        let filler_ac = AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .build(filler)
            .expect("filler AC automaton should build");

        Self {
            reasoning_ac,
            technical_ac,
            greeting_ac,
            filler_ac,
            weights: config.weights.clone(),
            light_standard: config.thresholds.light_standard,
            standard_heavy: config.thresholds.standard_heavy,
            light_tier: config.tier_mapping.light.clone(),
            medium_tier: config.tier_mapping.medium.clone(),
            heavy_tier: config.tier_mapping.heavy.clone(),
        }
    }

    /// Score a user message across all 7 dimensions.
    pub fn score(&self, content: &str) -> ComplexityScore {
        let d1 = score_message_length(content);
        let d2 = self.score_reasoning_keywords(content);
        let d3 = self.score_technical_vocabulary(content);
        let d4 = score_question_complexity(content);
        let d5 = score_code_presence(content);
        let d6 = score_instruction_complexity(content);
        let d7 = self.score_conversational_simplicity(content);

        // Check force overrides
        let reasoning_hits = count_ac_hits(&self.reasoning_ac, content);
        let forced = if reasoning_hits >= REASONING_FORCE_THRESHOLD {
            Some("reasoning_keywords")
        } else if content.len() > LENGTH_FORCE_THRESHOLD {
            Some("message_length")
        } else if is_pure_greeting_or_filler(content) {
            Some("conversational_simplicity")
        } else {
            None
        };

        let composite = match forced {
            Some("reasoning_keywords" | "message_length") => 1.0,
            Some("conversational_simplicity") => 0.0,
            _ => {
                let weighted_sum = d1 * self.weights.message_length
                    + d2 * self.weights.reasoning_keywords
                    + d3 * self.weights.technical_vocabulary
                    + d4 * self.weights.question_complexity
                    + d5 * self.weights.code_presence
                    + d6 * self.weights.instruction_complexity
                    + d7 * self.weights.conversational_simplicity;
                sigmoid(weighted_sum - 0.35, 6.0)
            }
        };

        ComplexityScore { composite, forced }
    }

    /// Map a composite score to the appropriate tier name.
    pub fn resolve_tier<'a>(&'a self, score: &ComplexityScore) -> &'a str {
        if score.composite < self.light_standard {
            &self.light_tier
        } else if score.composite >= self.standard_heavy {
            &self.heavy_tier
        } else {
            &self.medium_tier
        }
    }

    // -----------------------------------------------------------------------
    // Dimension scorers (methods that use self for AC automata)
    // -----------------------------------------------------------------------

    /// D2: Reasoning keywords — AC hit count, saturates at 3.
    fn score_reasoning_keywords(&self, content: &str) -> f64 {
        let hits = count_ac_hits(&self.reasoning_ac, content);
        (hits as f64 / 3.0).min(1.0)
    }

    /// D3: Technical vocabulary — AC hit count, saturates at 5.
    fn score_technical_vocabulary(&self, content: &str) -> f64 {
        let hits = count_ac_hits(&self.technical_ac, content);
        (hits as f64 / 5.0).min(1.0)
    }

    /// D7: Conversational simplicity — high score when message is a greeting
    /// or filler phrase (negative weight will push composite down).
    fn score_conversational_simplicity(&self, content: &str) -> f64 {
        if is_pure_greeting_or_filler(content) {
            return 1.0;
        }
        // Partial: count greeting/filler hits relative to word count
        let word_count = content.split_whitespace().count().max(1);
        let greeting_hits = count_ac_hits(&self.greeting_ac, content);
        let filler_hits = count_ac_hits(&self.filler_ac, content);
        let total_hits = greeting_hits + filler_hits;
        (total_hits as f64 / word_count as f64).min(1.0)
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------
}

// ---------------------------------------------------------------------------
// Free functions (no &self needed)
// ---------------------------------------------------------------------------

/// D1: Message length — sigmoid normalization centered at 500 chars.
fn score_message_length(content: &str) -> f64 {
    let len = content.len() as f64;
    sigmoid(len - 500.0, 0.005)
}

/// D4: Question complexity — regex classification.
fn score_question_complexity(content: &str) -> f64 {
    if MULTIPART_QUESTION_RE.is_match(content) {
        0.9
    } else if ANALYTICAL_QUESTION_RE.is_match(content) {
        0.7
    } else if COMPARATIVE_QUESTION_RE.is_match(content) {
        0.5
    } else if SIMPLE_QUESTION_RE.is_match(content) {
        0.1
    } else {
        0.0
    }
}

/// D5: Code presence — code fences, inline code, code patterns.
fn score_code_presence(content: &str) -> f64 {
    let has_fence = CODE_FENCE_RE.is_match(content);
    let has_inline = INLINE_CODE_RE.is_match(content);
    let has_pattern = CODE_PATTERN_RE.is_match(content);
    match (has_fence, has_inline, has_pattern) {
        (true, _, _) => 0.8,
        (false, true, true) => 0.6,
        (false, true, false) | (false, false, true) => 0.3,
        (false, false, false) => 0.0,
    }
}

/// D6: Instruction complexity — sequential markers + imperative verbs,
/// saturates at 4 steps.
fn score_instruction_complexity(content: &str) -> f64 {
    let seq_count = SEQUENTIAL_MARKER_RE.find_iter(content).count();
    let imp_count = IMPERATIVE_VERB_RE.find_iter(content).count();
    let steps = seq_count.max(imp_count);
    (steps as f64 / 4.0).min(1.0)
}

/// Count distinct AC pattern matches (deduplicated by pattern ID).
fn count_ac_hits(ac: &AhoCorasick, text: &str) -> usize {
    let mut seen = vec![false; ac.patterns_len()];
    let mut count = 0;
    for mat in ac.find_overlapping_iter(text) {
        let pid = mat.pattern().as_usize();
        if !seen[pid] {
            seen[pid] = true;
            count += 1;
        }
    }
    count
}

/// Check if the entire message (after trimming/normalizing) is a single
/// greeting or filler phrase. Uses exact-match lookup against static word
/// lists to avoid false positives from AC substring matching (e.g. "nah
/// analyze this" must NOT be classified as filler).
fn is_pure_greeting_or_filler(content: &str) -> bool {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return true;
    }
    let normalized = trimmed
        .to_lowercase()
        .trim_end_matches(|c: char| c.is_ascii_punctuation())
        .to_string();
    if normalized.is_empty() {
        return true;
    }
    GREETING_SET.contains(normalized.as_str()) || FILLER_SET.contains(normalized.as_str())
}

/// Standard sigmoid function: 1 / (1 + exp(-k*x))
fn sigmoid(x: f64, steepness: f64) -> f64 {
    1.0 / (1.0 + (-steepness * x).exp())
}
