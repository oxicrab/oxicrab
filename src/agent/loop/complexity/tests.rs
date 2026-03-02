use super::*;
use crate::config::schema::ComplexityRoutingConfig;

fn default_scorer() -> ComplexityScorer {
    ComplexityScorer::new(&ComplexityRoutingConfig::default())
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
    let scorer = default_scorer();
    let s = scorer.score_reasoning_keywords("please send me the file");
    assert!(
        s < 0.01,
        "no reasoning keywords should score near 0, got {s}"
    );
}

#[test]
fn single_reasoning_keyword() {
    let scorer = default_scorer();
    let s = scorer.score_reasoning_keywords("can you analyze this data?");
    assert!(
        (0.3..=0.4).contains(&s),
        "one reasoning keyword should score ~0.33, got {s}"
    );
}

#[test]
fn reasoning_keywords_saturate_at_3() {
    let scorer = default_scorer();
    let s = scorer.score_reasoning_keywords(
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
    let scorer = default_scorer();
    let s = scorer.score_technical_vocabulary("what should I have for lunch?");
    assert!(s < 0.01, "no technical terms should score near 0, got {s}");
}

#[test]
fn some_technical_terms() {
    let scorer = default_scorer();
    let s = scorer.score_technical_vocabulary(
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
    let scorer = default_scorer();
    let s = scorer.score_conversational_simplicity("hello!");
    assert!(s > 0.9, "pure greeting should score ~1.0, got {s}");
}

#[test]
fn pure_filler() {
    let scorer = default_scorer();
    let s = scorer.score_conversational_simplicity("ok");
    assert!(s > 0.9, "pure filler should score ~1.0, got {s}");
}

#[test]
fn not_conversational() {
    let scorer = default_scorer();
    let s = scorer
        .score_conversational_simplicity("implement the new authentication system with JWT tokens");
    assert!(
        s < 0.5,
        "technical message should have low conversational score, got {s}"
    );
}

// ---------------------------------------------------------------------------
// Composite scoring & tier mapping
// ---------------------------------------------------------------------------

#[test]
fn greeting_routes_to_light_tier() {
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
    assert_eq!(scorer.resolve_tier(&score), "lightweight");
}

#[test]
fn complex_reasoning_routes_to_heavy_tier() {
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
    assert_eq!(scorer.resolve_tier(&score), "heavy");
}

#[test]
fn very_long_message_routes_to_heavy() {
    let scorer = default_scorer();
    let long_msg = "word ".repeat(15_000);
    let score = scorer.score(&long_msg);
    assert_eq!(
        score.forced,
        Some("message_length"),
        ">50KB should force heavy"
    );
    assert_eq!(scorer.resolve_tier(&score), "heavy");
}

#[test]
fn moderate_question_routes_to_standard_or_higher() {
    let scorer = default_scorer();
    // Analytical question + technical vocab + reasoning keywords → should be standard or heavy
    let score = scorer.score(
        "why does the authentication middleware reject valid tokens? analyze the architecture and explain the trade-offs",
    );
    let tier = scorer.resolve_tier(&score);
    assert!(
        tier == "standard" || tier == "heavy",
        "moderate-to-high complexity should route to standard or heavy, got {} (composite={})",
        tier,
        score.composite
    );
}

#[test]
fn simple_factual_routes_to_light_or_standard() {
    let scorer = default_scorer();
    let score = scorer.score("what is rust?");
    let tier = scorer.resolve_tier(&score);
    assert!(
        tier == "lightweight" || tier == "standard",
        "simple factual question should be light or standard, got {}",
        tier
    );
}

// ---------------------------------------------------------------------------
// Custom config
// ---------------------------------------------------------------------------

#[test]
fn custom_tier_names() {
    let config = ComplexityRoutingConfig {
        enabled: true,
        tier_mapping: crate::config::schema::ComplexityTierMapping {
            light: "cheap".to_string(),
            heavy: "expensive".to_string(),
            ..Default::default()
        },
        ..Default::default()
    };
    let scorer = ComplexityScorer::new(&config);
    let score = scorer.score("hi");
    assert_eq!(scorer.resolve_tier(&score), "cheap");

    let score = scorer.score("analyze the trade-offs and synthesize a plan step by step");
    assert_eq!(scorer.resolve_tier(&score), "expensive");
}

#[test]
fn custom_thresholds() {
    let config = ComplexityRoutingConfig {
        enabled: true,
        thresholds: crate::config::schema::ComplexityThresholds {
            light_standard: 0.1,
            standard_heavy: 0.9,
        },
        ..Default::default()
    };
    let scorer = ComplexityScorer::new(&config);

    // A message with moderate technical content should land in "standard" with these thresholds
    let score = scorer
        .score("explain how the database algorithm handles concurrent authentication requests");
    let tier = scorer.resolve_tier(&score);
    assert_eq!(
        tier, "standard",
        "with tight thresholds, moderate message should be standard (composite={})",
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
    assert_eq!(scorer.resolve_tier(&score), "lightweight");
}

#[test]
fn case_insensitive_keywords() {
    let scorer = default_scorer();
    let upper = scorer.score_reasoning_keywords("ANALYZE this DATA");
    let lower = scorer.score_reasoning_keywords("analyze this data");
    assert!(
        (upper - lower).abs() < 0.01,
        "keywords should be case-insensitive"
    );
}
