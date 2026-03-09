use super::*;
use crate::config::schema::ComplexityWeights;

fn default_scorer() -> ComplexityScorer {
    ComplexityScorer::new(&ComplexityWeights::default())
}

// ---------------------------------------------------------------------------
// D1: Message length
// ---------------------------------------------------------------------------

#[test]
fn short_message_low_length_score() {
    let s = score_message_length("hi there");
    assert!(
        s < 0.2,
        "short message should have low length score, got {s}"
    );
}

#[test]
fn long_message_high_length_score() {
    let long_msg = "a ".repeat(1000);
    let s = score_message_length(&long_msg);
    assert!(
        s > 0.8,
        "long message should have high length score, got {s}"
    );
}

// ---------------------------------------------------------------------------
// D2: Reasoning keywords
// ---------------------------------------------------------------------------

#[test]
fn no_reasoning_keywords() {
    let s = score_reasoning_keywords("please send me the file");
    assert!(
        s < 0.01,
        "no reasoning keywords should score near 0, got {s}"
    );
}

#[test]
fn single_reasoning_keyword() {
    let s = score_reasoning_keywords("can you analyze this data?");
    assert!(
        (0.3..=0.4).contains(&s),
        "one reasoning keyword should score ~0.33, got {s}"
    );
}

#[test]
fn reasoning_keywords_saturate_at_3() {
    let s = score_reasoning_keywords(
        "analyze the trade-offs, synthesize results, and evaluate the hypothesis",
    );
    assert!(
        (0.99..=1.0).contains(&s),
        "3+ reasoning keywords should saturate at 1.0, got {s}"
    );
}

// ---------------------------------------------------------------------------
// D3: Technical vocabulary
// ---------------------------------------------------------------------------

#[test]
fn no_technical_vocabulary() {
    let s = score_technical_vocabulary("what should I have for lunch?");
    assert!(s < 0.01, "no technical terms should score near 0, got {s}");
}

#[test]
fn some_technical_terms() {
    let s = score_technical_vocabulary(
        "implement the algorithm using the API with proper authentication",
    );
    assert!(s > 0.4, "3 technical terms should score > 0.4, got {s}");
}

// ---------------------------------------------------------------------------
// D4: Question complexity
// ---------------------------------------------------------------------------

#[test]
fn simple_question() {
    let s = score_question_complexity("what is a mutex?");
    assert!(
        (0.05..=0.15).contains(&s),
        "simple question should score ~0.1, got {s}"
    );
}

#[test]
fn comparative_question() {
    let s = score_question_complexity("which is better: Redis vs Memcached?");
    assert!(
        (0.45..=0.55).contains(&s),
        "comparative question should score ~0.5, got {s}"
    );
}

#[test]
fn analytical_question() {
    let s = score_question_complexity("why does the garbage collector pause here?");
    assert!(
        (0.65..=0.75).contains(&s),
        "analytical question should score ~0.7, got {s}"
    );
}

#[test]
fn multipart_question() {
    let s = score_question_complexity("what is a mutex? and also how does async work?");
    assert!(
        (0.85..=0.95).contains(&s),
        "multi-part question should score ~0.9, got {s}"
    );
}

// ---------------------------------------------------------------------------
// D5: Code presence
// ---------------------------------------------------------------------------

#[test]
fn no_code() {
    let s = score_code_presence("please explain the concept");
    assert!(s < 0.01, "no code should score 0, got {s}");
}

#[test]
fn code_fence() {
    let s = score_code_presence("fix this:\n```rust\nfn main() {}\n```");
    assert!(
        (0.75..=0.85).contains(&s),
        "code fence should score ~0.8, got {s}"
    );
}

#[test]
fn inline_code_only() {
    let s = score_code_presence("what does `unwrap()` do?");
    // inline code + code pattern (unwrap())
    assert!(s > 0.2, "inline code should score > 0.2, got {s}");
}

// ---------------------------------------------------------------------------
// D6: Instruction complexity
// ---------------------------------------------------------------------------

#[test]
fn no_instructions() {
    let s = score_instruction_complexity("what time is it?");
    assert!(s < 0.01, "no instructions should score 0, got {s}");
}

#[test]
fn multi_step_instructions() {
    let s = score_instruction_complexity(
        "1. Create the database schema\n2. Add authentication middleware\n3. Write the tests\n4. Deploy to staging",
    );
    assert!(
        s > 0.9,
        "4-step numbered list should saturate near 1.0, got {s}"
    );
}

#[test]
fn imperative_verbs() {
    let s = score_instruction_complexity("create a new endpoint and add error handling");
    assert!(s > 0.2, "imperative verbs should score > 0, got {s}");
}

// ---------------------------------------------------------------------------
// D7: Conversational simplicity
// ---------------------------------------------------------------------------

#[test]
fn pure_greeting() {
    let s = score_conversational_simplicity("hello!");
    assert!(s > 0.9, "pure greeting should score ~1.0, got {s}");
}

#[test]
fn pure_filler() {
    let s = score_conversational_simplicity("ok");
    assert!(s > 0.9, "pure filler should score ~1.0, got {s}");
}

#[test]
fn not_conversational() {
    let s =
        score_conversational_simplicity("implement the new authentication system with JWT tokens");
    assert!(
        s < 0.5,
        "technical message should have low conversational score, got {s}"
    );
}

// ---------------------------------------------------------------------------
// Composite scoring & force overrides
// ---------------------------------------------------------------------------

#[test]
fn greeting_forces_zero_composite() {
    let scorer = default_scorer();
    let score = scorer.score("hi");
    assert_eq!(
        score.forced,
        Some("conversational_simplicity"),
        "pure greeting should be force-overridden"
    );
    assert!(
        score.composite < 0.01,
        "greeting composite should be 0.0, got {}",
        score.composite
    );
}

#[test]
fn complex_reasoning_forces_max_composite() {
    let scorer = default_scorer();
    let score = scorer.score(
        "analyze the trade-offs between microservices and monoliths, then synthesize a recommendation step by step",
    );
    assert_eq!(
        score.forced,
        Some("reasoning_keywords"),
        "2+ reasoning keywords should force heavy"
    );
    assert!((0.99..=1.0).contains(&score.composite));
}

#[test]
fn very_long_message_forces_max_composite() {
    let scorer = default_scorer();
    let long_msg = "word ".repeat(15_000);
    let score = scorer.score(&long_msg);
    assert_eq!(
        score.forced,
        Some("message_length"),
        ">50KB should force heavy"
    );
    assert!((0.99..=1.0).contains(&score.composite));
}

#[test]
fn moderate_question_scores_moderate_composite() {
    let scorer = default_scorer();
    let score = scorer.score(
        "why does the authentication middleware reject valid tokens? analyze the architecture and explain the trade-offs",
    );
    // Should score moderate-to-high
    assert!(
        score.composite > 0.3,
        "moderate-to-high complexity should have composite > 0.3, got {}",
        score.composite
    );
}

#[test]
fn simple_factual_scores_low_composite() {
    let scorer = default_scorer();
    let score = scorer.score("what is rust?");
    assert!(
        score.composite < 0.65,
        "simple factual question should have composite < 0.65, got {}",
        score.composite
    );
}

// ---------------------------------------------------------------------------
// Performance sanity check
// ---------------------------------------------------------------------------

#[test]
fn scoring_is_fast() {
    let scorer = default_scorer();
    let messages = [
        "hi",
        "what is rust?",
        "analyze the trade-offs between X and Y step by step",
        "```rust\nfn main() { println!(\"hello\"); }\n```\nfix the compilation error",
        "1. Create the schema\n2. Add middleware\n3. Write tests\n4. Deploy",
    ];

    let start = std::time::Instant::now();
    for _ in 0..1000 {
        for msg in &messages {
            let _ = scorer.score(msg);
        }
    }
    let elapsed = start.elapsed();
    // 5000 scorings should complete in well under 1 second
    assert!(
        elapsed.as_millis() < 2000,
        "5000 scorings took {}ms (expected < 2000ms)",
        elapsed.as_millis()
    );
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn empty_message() {
    let scorer = default_scorer();
    let score = scorer.score("");
    assert!(
        score.composite < 0.3,
        "empty message should have low composite, got {}",
        score.composite
    );
}

#[test]
fn case_insensitive_keywords() {
    let upper = score_reasoning_keywords("ANALYZE this DATA");
    let lower = score_reasoning_keywords("analyze this data");
    assert!(
        (upper - lower).abs() < 0.01,
        "keywords should be case-insensitive"
    );
}

#[test]
fn filler_prefix_not_force_overridden() {
    // "nah analyze this" starts with filler "nah" but is a substantive request.
    // Must NOT be force-overridden to lightweight via conversational_simplicity.
    let scorer = default_scorer();
    let score = scorer.score("nah analyze this");
    assert_ne!(
        score.forced,
        Some("conversational_simplicity"),
        "filler-prefixed substantive message must not be force-overridden"
    );
}

#[test]
fn short_mixed_filler_not_force_overridden() {
    let scorer = default_scorer();
    for msg in ["ok implement auth", "yeah debug this", "sure deploy it"] {
        let score = scorer.score(msg);
        assert_ne!(
            score.forced,
            Some("conversational_simplicity"),
            "'{msg}' should not be force-overridden as filler"
        );
    }
}
