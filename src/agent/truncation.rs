use regex::Regex;
use serde_json::Value;

const _DEFAULT_MAX_CHARS: usize = 3000;

pub fn truncate_tool_result(result: &str, max_chars: usize) -> String {
    truncate_tool_result_internal(result, max_chars)
}

fn truncate_tool_result_internal(result: &str, max_chars: usize) -> String {
    // Strip ANSI escape codes
    let re_ansi = Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").unwrap();
    let clean = re_ansi.replace_all(result, "").to_string();

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
                return format!(
                    "{}\n\n... [JSON truncated - showed {} of {} chars. Do NOT re-run this tool to see more.]",
                    &pretty[..budget.min(pretty.len())],
                    budget,
                    pretty.len()
                );
            }
        }
    }

    let budget = max_chars.saturating_sub(100);
    format!(
        "{}\n\n... [truncated - showed {} of {} chars. Do NOT re-run this tool to see more.]",
        &clean[..budget.min(clean.len())],
        budget,
        clean.len()
    )
}
