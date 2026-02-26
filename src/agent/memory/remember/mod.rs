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
        if line.is_empty() || line.starts_with('#') || (line.starts_with("- ") && line.len() < 4) {
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
mod tests;
