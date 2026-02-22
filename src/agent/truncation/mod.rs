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
