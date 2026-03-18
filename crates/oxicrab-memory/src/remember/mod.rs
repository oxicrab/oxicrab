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
            // Return the original-case content (trim same prefix length from original).
            // All REMEMBER_PATTERNS are pure ASCII, so pattern.len() == byte length
            // in both the lowercased and original string. Use get() for safety.
            let original_trimmed = message.trim();
            let original_rest = original_trimmed
                .get(pattern.len()..)
                .unwrap_or(original_trimmed);
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

/// Check if content is a near-duplicate of any recent DB entries.
pub fn is_duplicate_of_entries(content: &str, entries: &[String]) -> bool {
    let threshold = 0.7;
    for entry in entries {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        // Strip leading "- " for comparison
        let entry_content = entry.strip_prefix("- ").unwrap_or(entry);
        if jaccard_similarity(content, entry_content) >= threshold {
            return true;
        }
    }
    false
}

fn word_set(text: &str) -> HashSet<String> {
    text.split_whitespace().map(str::to_lowercase).collect()
}

#[cfg(test)]
mod tests;
