use super::*;

// --- Rejection tests ---

#[test]
fn reject_too_short() {
    assert_eq!(
        check_quality("short"),
        QualityVerdict::Reject(RejectReason::TooShort)
    );
    assert_eq!(
        check_quality("hello world"),
        QualityVerdict::Reject(RejectReason::TooShort)
    );
}

#[test]
fn reject_greetings() {
    // Exact match after punctuation stripping
    assert_eq!(
        check_quality("Good morning!!!"),
        QualityVerdict::Reject(RejectReason::Greeting)
    );
    assert_eq!(
        check_quality("thank you......"),
        QualityVerdict::Reject(RejectReason::Greeting)
    );
    // Not an exact greeting match — extra words
    assert_eq!(
        check_quality("thank you so much for the help"),
        QualityVerdict::Pass
    );
}

#[test]
fn reject_greetings_exact() {
    for greeting in GREETINGS {
        if greeting.len() >= MIN_USEFUL_LEN {
            let result = check_quality(greeting);
            assert_eq!(
                result,
                QualityVerdict::Reject(RejectReason::Greeting),
                "greeting '{}' should be rejected",
                greeting
            );
        }
    }
}

#[test]
fn reject_filler() {
    // "understood" is 10 chars, but we need 15 for the length gate.
    // "understood!!!!!" is 15 chars after trimming becomes "understood" which is in FILLER.
    assert_eq!(
        check_quality("understood!!!!!"),
        QualityVerdict::Reject(RejectReason::Filler)
    );
}

#[test]
fn reject_filler_with_punctuation() {
    // "ok" is too short (< 15 chars), so most filler rejects as TooShort first.
    // Test with padding that still looks like filler.
    assert_eq!(
        check_quality("ok"),
        QualityVerdict::Reject(RejectReason::TooShort)
    );
}

// --- Pass tests ---

#[test]
fn pass_substantive_content() {
    assert_eq!(
        check_quality("I prefer dark mode for all my editors"),
        QualityVerdict::Pass
    );
    assert_eq!(
        check_quality("The server IP is 10.0.0.1 on port 8080"),
        QualityVerdict::Pass
    );
    assert_eq!(
        check_quality("Deploy to staging every Thursday at 2pm"),
        QualityVerdict::Pass
    );
}

#[test]
fn pass_factual_with_negative_word_substring() {
    // "failed" appears as a substring in "failedover" etc but this is a factual preference
    assert_eq!(
        check_quality("The CI pipeline uses cargo test for validation"),
        QualityVerdict::Pass
    );
}

#[test]
fn pass_greeting_as_substring() {
    // "hello" is a greeting, but "hello world service" is substantive
    assert_eq!(
        check_quality("The hello world service runs on port 3000"),
        QualityVerdict::Pass
    );
}

// --- Reframe tests ---

#[test]
fn reframe_negative_memory() {
    let result = check_quality("The tool calls were broken yesterday");
    match result {
        QualityVerdict::Reframed(s) => {
            assert!(s.starts_with("NOTE (reframed):"));
            assert!(s.contains("tool calls were broken"));
            assert!(s.contains("verify current state"));
        }
        other => panic!("expected Reframed, got {:?}", other),
    }
}

#[test]
fn reframe_api_failure() {
    let result = check_quality("The API kept failing with 500 errors");
    match result {
        QualityVerdict::Reframed(s) => {
            assert!(s.contains("API kept failing"));
        }
        other => panic!("expected Reframed, got {:?}", other),
    }
}

#[test]
fn reframe_crash() {
    let result = check_quality("The database crashed after the migration");
    match result {
        QualityVerdict::Reframed(s) => {
            assert!(s.contains("database crashed"));
        }
        other => panic!("expected Reframed, got {:?}", other),
    }
}

#[test]
fn no_reframe_when_already_constructive() {
    // Content has a negative pattern but also a constructive marker
    assert_eq!(
        check_quality("The API was broken, fixed by upgrading to v3"),
        QualityVerdict::Pass
    );
    assert_eq!(
        check_quality("Connection timed out — workaround: increase timeout to 60s"),
        QualityVerdict::Pass
    );
    assert_eq!(
        check_quality("Build crashed on ARM — TODO: add cross-compile flag"),
        QualityVerdict::Pass
    );
}

#[test]
fn no_reframe_factual_statement() {
    assert_eq!(
        check_quality("The deployment process uses blue-green strategy"),
        QualityVerdict::Pass
    );
}

// --- Edge cases ---

#[test]
fn empty_string_rejected() {
    assert_eq!(
        check_quality(""),
        QualityVerdict::Reject(RejectReason::TooShort)
    );
}

#[test]
fn whitespace_only_rejected() {
    assert_eq!(
        check_quality("   \t\n  "),
        QualityVerdict::Reject(RejectReason::TooShort)
    );
}

#[test]
fn boundary_length() {
    // Exactly 15 chars should pass (if not greeting/filler)
    assert_eq!(check_quality("123456789012345"), QualityVerdict::Pass);
    // 14 chars should be rejected
    assert_eq!(
        check_quality("12345678901234"),
        QualityVerdict::Reject(RejectReason::TooShort)
    );
}

// --- filter_lines tests ---

#[test]
fn filter_lines_passes_substantive() {
    let input = "- User prefers dark mode for editing\n- Project uses Rust nightly";
    let result = filter_lines(input);
    assert_eq!(result, input);
}

#[test]
fn filter_lines_drops_filler() {
    let input = "- User prefers dark mode for editing\n- ok\n- The server runs on port 8080";
    let result = filter_lines(input);
    assert!(result.contains("dark mode"));
    assert!(!result.contains("\n- ok\n"));
    assert!(result.contains("port 8080"));
}

#[test]
fn filter_lines_reframes_negative() {
    let input = "- The API was broken after deployment\n- User prefers vim keybindings";
    let result = filter_lines(input);
    assert!(result.contains("NOTE (reframed)"));
    assert!(result.contains("API was broken"));
    assert!(result.contains("vim keybindings"));
}

#[test]
fn filter_lines_preserves_headers() {
    let input = "## Extracted Facts\n\n- The server IP is 10.0.0.1";
    let result = filter_lines(input);
    assert!(result.contains("## Extracted Facts"));
    assert!(result.contains("10.0.0.1"));
}

#[test]
fn filter_lines_all_rejected() {
    let input = "- hi\n- ok\n- sure";
    let result = filter_lines(input);
    assert!(result.trim().is_empty());
}

#[test]
fn filter_lines_empty_input() {
    assert_eq!(filter_lines(""), "");
}

#[test]
fn filter_lines_preserves_indentation_on_reframe() {
    let input = "  - The API was broken after deployment";
    let result = filter_lines(input);
    assert!(result.starts_with("  - NOTE (reframed):"));
    assert!(result.contains("API was broken"));
}
