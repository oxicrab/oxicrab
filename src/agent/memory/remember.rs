use std::collections::HashSet;

const REMEMBER_PATTERNS: &[&str] = &[
    "remember that ",
    "remember: ",
    "please remember ",
    "don't forget ",
    "note that ",
    "keep in mind ",
];

const MIN_CONTENT_LEN: usize = 8;

/// Extract memorable content from a message that starts with a "remember" trigger.
/// Returns `None` if the message doesn't match or should be rejected.
pub fn extract_remember_content(message: &str) -> Option<String> {
    let lower = message.to_lowercase();
    let trimmed = lower.trim();

    for pattern in REMEMBER_PATTERNS {
        if let Some(rest) = trimmed.strip_prefix(pattern) {
            let content = rest.trim();
            if content.len() < MIN_CONTENT_LEN {
                return None;
            }
            // Reject questions
            if content.ends_with('?') {
                return None;
            }
            // Reject interrogative forms ("remember when...", "remember how...")
            let interrogatives = ["when ", "how ", "what ", "why ", "if ", "whether "];
            if interrogatives.iter().any(|q| content.starts_with(q)) {
                return None;
            }
            // Return the original-case content (trim same prefix length from original)
            let original_trimmed = message.trim();
            let prefix_start = original_trimmed.to_lowercase().find(pattern).unwrap_or(0);
            let original_rest = &original_trimmed[prefix_start + pattern.len()..];
            return Some(original_rest.trim().to_string());
        }
    }

    None
}

/// Compute Jaccard similarity between two strings using word-level unigrams.
pub fn jaccard_similarity(a: &str, b: &str) -> f64 {
    let words_a = word_set(a);
    let words_b = word_set(b);

    if words_a.is_empty() && words_b.is_empty() {
        return 1.0;
    }

    let intersection = words_a.intersection(&words_b).count();
    let union = words_a.union(&words_b).count();

    if union == 0 {
        return 0.0;
    }

    intersection as f64 / union as f64
}

/// Check if content is a near-duplicate of any existing note line.
pub fn is_duplicate(content: &str, existing_notes: &str) -> bool {
    let threshold = 0.7;
    for line in existing_notes.lines() {
        let line = line.trim();
        // Skip empty lines and headers
        if line.is_empty() || line.starts_with('#') || line.starts_with("- ") && line.len() < 4 {
            continue;
        }
        // Strip leading "- " for comparison
        let line_content = line.strip_prefix("- ").unwrap_or(line);
        if jaccard_similarity(content, line_content) >= threshold {
            return true;
        }
    }
    false
}

fn word_set(text: &str) -> HashSet<String> {
    text.split_whitespace().map(str::to_lowercase).collect()
}

#[cfg(test)]
mod tests {
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
}
