use super::*;

#[test]
fn test_extract_basic_patterns() {
    assert_eq!(
        extract_remember_content("remember that I prefer dark mode"),
        Some("I prefer dark mode".to_string())
    );
    assert_eq!(
        extract_remember_content("Remember: my API key is in .env"),
        Some("my API key is in .env".to_string())
    );
    assert_eq!(
        extract_remember_content("please remember I use vim"),
        Some("I use vim".to_string())
    );
    assert_eq!(
        extract_remember_content("don't forget the deploy deadline is Friday"),
        Some("the deploy deadline is Friday".to_string())
    );
    assert_eq!(
        extract_remember_content("note that the server IP is 10.0.0.1"),
        Some("the server IP is 10.0.0.1".to_string())
    );
    assert_eq!(
        extract_remember_content("keep in mind I'm allergic to peanuts"),
        Some("I'm allergic to peanuts".to_string())
    );
}

#[test]
fn test_extract_rejects_questions() {
    assert_eq!(
        extract_remember_content("remember that time we went fishing?"),
        None
    );
}

#[test]
fn test_extract_rejects_interrogatives() {
    assert_eq!(
        extract_remember_content("remember when we deployed v2?"),
        None
    );
    assert_eq!(
        extract_remember_content("remember how to configure nginx?"),
        None
    );
    assert_eq!(
        extract_remember_content("remember what the password was"),
        None
    );
    assert_eq!(extract_remember_content("remember why we chose Rust"), None);
    assert_eq!(
        extract_remember_content("remember if the server is running"),
        None
    );
}

#[test]
fn test_extract_rejects_short() {
    assert_eq!(extract_remember_content("remember that hi"), None);
    assert_eq!(extract_remember_content("remember that a"), None);
}

#[test]
fn test_extract_no_match() {
    assert_eq!(
        extract_remember_content("Can you help me with this code?"),
        None
    );
    assert_eq!(extract_remember_content("I remember that day"), None);
}

#[test]
fn test_jaccard_identical() {
    let sim = jaccard_similarity("the quick brown fox", "the quick brown fox");
    assert!((sim - 1.0).abs() < f64::EPSILON);
}

#[test]
fn test_jaccard_different() {
    let sim = jaccard_similarity("hello world today", "completely different sentence here");
    assert!(sim < 0.1);
}

#[test]
fn test_jaccard_partial_overlap() {
    let sim = jaccard_similarity("I prefer dark mode", "I prefer light mode");
    assert!(sim > 0.3);
    assert!(sim < 0.8);
}

#[test]
fn test_jaccard_single_word() {
    let sim = jaccard_similarity("hello", "world");
    assert!((sim - 0.0).abs() < f64::EPSILON);
}

#[test]
fn test_is_duplicate_finds_match() {
    let notes = "# Notes\n\n- I prefer dark mode for all editors\n- Deploy on Fridays\n";
    assert!(is_duplicate("I prefer dark mode for editors", notes));
}

#[test]
fn test_is_duplicate_no_match() {
    let notes = "# Notes\n\n- I prefer dark mode\n- Deploy on Fridays\n";
    assert!(!is_duplicate(
        "The server runs on port 8080 with TLS enabled",
        notes
    ));
}

#[test]
fn test_is_duplicate_skips_headers() {
    let notes = "# Remember\n\n## Section\n\n- actual note here about something";
    assert!(!is_duplicate("Remember", notes));
}
