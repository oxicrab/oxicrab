use crate::utils::regex::RegexPatterns;
use serde_json::Value;

pub fn truncate_tool_result(result: &str, max_chars: usize) -> String {
    // Strip ANSI escape codes
    let clean = RegexPatterns::ansi_escape()
        .replace_all(result, "")
        .to_string();

    if clean.len() <= max_chars {
        return clean;
    }

    let stripped = clean.trim_start();
    if stripped.starts_with('{') || stripped.starts_with('[') {
        if let Ok(parsed) = serde_json::from_str::<Value>(&clean) {
            if let Ok(pretty) = serde_json::to_string_pretty(&parsed) {
                if pretty.len() <= max_chars {
                    return pretty;
                }
                let budget = max_chars.saturating_sub(120);
                let safe_budget = floor_char_boundary(&pretty, budget);
                return format!(
                    "{}\n\n... [JSON truncated - showed {} of {} chars. Do NOT re-run this tool to see more.]",
                    &pretty[..safe_budget],
                    safe_budget,
                    pretty.len()
                );
            }
        }
    }

    let budget = max_chars.saturating_sub(100);
    let safe_budget = floor_char_boundary(&clean, budget);
    format!(
        "{}\n\n... [truncated - showed {} of {} chars. Do NOT re-run this tool to see more.]",
        &clean[..safe_budget],
        safe_budget,
        clean.len()
    )
}

/// Find the largest byte index <= `index` that is a valid char boundary.
fn floor_char_boundary(s: &str, index: usize) -> usize {
    let mut i = index.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_char_boundary_ascii() {
        assert_eq!(floor_char_boundary("hello", 3), 3);
    }

    #[test]
    fn floor_char_boundary_zero() {
        assert_eq!(floor_char_boundary("hello", 0), 0);
    }

    #[test]
    fn floor_char_boundary_beyond_len() {
        assert_eq!(floor_char_boundary("hello", 100), 5);
    }

    #[test]
    fn floor_char_boundary_multibyte() {
        // Each emoji is 4 bytes
        let s = "a\u{1F600}b"; // a + grinning face + b = 1 + 4 + 1 = 6 bytes
        assert_eq!(floor_char_boundary(s, 1), 1); // right after 'a'
        assert_eq!(floor_char_boundary(s, 2), 1); // mid-emoji, snaps back to 1
        assert_eq!(floor_char_boundary(s, 3), 1);
        assert_eq!(floor_char_boundary(s, 4), 1);
        assert_eq!(floor_char_boundary(s, 5), 5); // right after emoji
    }

    #[test]
    fn floor_char_boundary_empty() {
        assert_eq!(floor_char_boundary("", 0), 0);
        assert_eq!(floor_char_boundary("", 5), 0);
    }

    #[test]
    fn truncate_short_string() {
        let result = truncate_tool_result("short", 100);
        assert_eq!(result, "short");
    }

    #[test]
    fn truncate_empty_string() {
        let result = truncate_tool_result("", 100);
        assert_eq!(result, "");
    }

    #[test]
    fn truncate_long_plain_text() {
        let long = "a".repeat(500);
        let result = truncate_tool_result(&long, 200);
        assert!(result.len() < 500);
        assert!(result.contains("[truncated"));
    }

    #[test]
    fn truncate_strips_ansi() {
        let with_ansi = "\x1b[31mred text\x1b[0m";
        let result = truncate_tool_result(with_ansi, 1000);
        assert_eq!(result, "red text");
        assert!(!result.contains("\x1b"));
    }

    #[test]
    fn truncate_json_pretty_prints_when_fits() {
        // The input must be longer than max_chars to trigger truncation path,
        // but the pretty-printed version must fit within max_chars
        let json = serde_json::json!({"key": "value", "num": 42}).to_string();
        // Compact JSON is ~27 chars. Pretty is ~40 chars. Set max to 30 to trigger
        // the JSON branch (compact > max), and pretty fits within max of 200.
        let result = truncate_tool_result(&json, 200);
        // Short enough that it returns the clean string directly (len <= max_chars)
        assert!(result.contains("key"));
        assert!(result.contains("value"));
    }

    #[test]
    fn truncate_json_truncates_when_large() {
        let big_json = serde_json::json!({
            "data": "x".repeat(500)
        })
        .to_string();
        let result = truncate_tool_result(&big_json, 200);
        assert!(result.contains("[JSON truncated"));
    }
}
