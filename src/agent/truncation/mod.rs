use crate::utils::regex::RegexPatterns;
use serde_json::Value;

/// Replace base64 data URIs, long base64 sequences, and long hex sequences
/// with descriptive placeholders. Applied before truncation so that text
/// content gets priority over binary blobs.
pub fn strip_binary_blobs(text: &str) -> String {
    // 1. Data URIs first (more specific pattern matches first)
    let result = RegexPatterns::data_uri().replace_all(text, |caps: &regex::Captures| {
        let full = caps.get(0).unwrap().as_str();
        // Extract MIME type from "data:mime/type;base64,..."
        let mime = full
            .strip_prefix("data:")
            .and_then(|s| s.split(';').next())
            .unwrap_or("unknown");
        let data_start = full.find(',').map_or(0, |i| i + 1);
        let data_len = full.len() - data_start;
        format!("[{} data, {} bytes]", mime, data_len)
    });

    // 2. Long hex sequences (before base64, since hex is a subset of base64 alphabet).
    // Only replace if the match contains both digits and hex letters to avoid
    // false positives on normal text (e.g., repeated 'a' chars).
    let result = RegexPatterns::long_hex().replace_all(&result, |caps: &regex::Captures| {
        let matched = caps.get(0).unwrap().as_str();
        let has_digit = matched.bytes().any(|b| b.is_ascii_digit());
        let has_letter = matched.bytes().any(|b| b.is_ascii_alphabetic());
        if has_digit && has_letter {
            format!("[hex data, {} bytes]", matched.len())
        } else {
            matched.to_string()
        }
    });

    // 3. Long base64 sequences (not already caught by data URI).
    // Only replace if the match contains at least one base64-specific character
    // (+, /, or =) to avoid false positives on normal text.
    let result = RegexPatterns::long_base64().replace_all(&result, |caps: &regex::Captures| {
        let matched = caps.get(0).unwrap().as_str();
        if matched.contains('+') || matched.contains('/') || matched.contains('=') {
            format!("[base64 data, {} bytes]", matched.len())
        } else {
            matched.to_string()
        }
    });

    result.into_owned()
}

pub fn truncate_tool_result(result: &str, max_chars: usize) -> String {
    // Strip ANSI escape codes
    let clean = RegexPatterns::ansi_escape()
        .replace_all(result, "")
        .to_string();

    // Strip binary blobs before truncation so text gets priority
    let clean = strip_binary_blobs(&clean);

    if clean.len() <= max_chars {
        return clean;
    }

    // For very small limits, just return a short indicator rather than
    // a truncation message that would exceed max_chars itself
    if max_chars < 120 {
        let safe = floor_char_boundary(&clean, max_chars);
        return clean[..safe].to_string();
    }

    let stripped = clean.trim_start();
    if (stripped.starts_with('{') || stripped.starts_with('['))
        && let Ok(parsed) = serde_json::from_str::<Value>(&clean)
        && let Ok(pretty) = serde_json::to_string_pretty(&parsed)
    {
        if pretty.len() <= max_chars {
            return pretty;
        }
        let budget = max_chars - 120;
        let safe_budget = floor_char_boundary(&pretty, budget);
        return format!(
            "{}\n\n... [JSON truncated - showed {} of {} chars. Do NOT re-run this tool to see more.]",
            &pretty[..safe_budget],
            safe_budget,
            pretty.len()
        );
    }

    let budget = max_chars - 100;
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
mod tests;
