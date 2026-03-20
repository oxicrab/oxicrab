use crate::regex_utils::RegexPatterns;

pub(super) fn format_for_slack(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    // Convert markdown tables to Slack-friendly format (before other transforms).
    let text = convert_tables(text);

    // Protect code blocks and inline code from formatting conversions.
    // Split on ``` fences: even-indexed segments are outside code blocks,
    // odd-indexed segments are inside fenced code blocks.
    let fenced_parts: Vec<&str> = text.split("```").collect();
    let mut result = String::with_capacity(text.len());
    for (i, part) in fenced_parts.iter().enumerate() {
        if i > 0 {
            result.push_str("```");
        }
        if i % 2 == 1 {
            // Inside a fenced code block — preserve as-is
            result.push_str(part);
        } else {
            // Outside fenced code blocks — protect inline code with backticks
            result.push_str(&format_non_code_segments(part));
        }
    }
    result
}

/// Apply markdown-to-Slack conversions only to parts outside inline code spans.
fn format_non_code_segments(text: &str) -> String {
    let inline_parts: Vec<&str> = text.split('`').collect();
    let mut result = String::with_capacity(text.len());
    for (i, part) in inline_parts.iter().enumerate() {
        if i > 0 {
            result.push('`');
        }
        if i % 2 == 1 {
            // Inside inline code — preserve as-is
            result.push_str(part);
        } else {
            // Outside all code — apply conversions
            let converted = RegexPatterns::markdown_bold().replace_all(part, r"*$1*");
            let converted = RegexPatterns::markdown_strike().replace_all(&converted, r"~$1~");
            let converted = RegexPatterns::markdown_link().replace_all(&converted, r"<$2|$1>");
            result.push_str(&converted);
        }
    }
    result
}

/// Convert markdown tables to Slack-friendly key-value format.
/// Slack mrkdwn has no table support, so we convert to plain text.
pub(super) fn convert_tables(text: &str) -> String {
    let separator = RegexPatterns::markdown_table_separator();
    let mut result = String::with_capacity(text.len());
    let mut table_lines: Vec<&str> = Vec::new();
    let mut in_table = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('|') && trimmed.ends_with('|') {
            if separator.is_match(trimmed) {
                // Skip separator rows.
                continue;
            }
            in_table = true;
            table_lines.push(trimmed);
        } else {
            if in_table {
                flush_table(&table_lines, &mut result);
                table_lines.clear();
                in_table = false;
            }
            result.push_str(line);
            result.push('\n');
        }
    }

    if in_table {
        flush_table(&table_lines, &mut result);
    }

    // Remove trailing newline added by line iteration.
    if result.ends_with('\n') && !text.ends_with('\n') {
        result.pop();
    }
    result
}

/// Render collected table rows as Slack-friendly bullet list.
///
/// Conversion strategies:
/// - **1-column**: bullet list (`• value`)
/// - **2-column**: bold first column with em dash (`• *col1* — col2`)
/// - **3+ columns**: labeled (`• *H1:* v1 · *H2:* v2`)
/// - **Header only**: joined with ` · `
fn flush_table(lines: &[&str], out: &mut String) {
    use std::fmt::Write;

    if lines.is_empty() {
        return;
    }

    let parse_cells = |line: &str| -> Vec<String> {
        line.trim_matches('|')
            .split('|')
            .map(|c| c.trim().to_string())
            .collect()
    };

    let header = parse_cells(lines[0]);
    let rows = &lines[1..];

    if rows.is_empty() {
        // Header only — join values.
        out.push_str(&header.join(" · "));
        out.push('\n');
        return;
    }

    let col_count = header.len();

    for row in rows {
        let cells = parse_cells(row);
        let non_empty: Vec<_> = cells.iter().filter(|c| !c.is_empty()).collect();
        if non_empty.is_empty() {
            continue;
        }

        match col_count {
            1 => {
                // Single column: bullet list.
                let _ = writeln!(out, "• {}", cells[0]);
            }
            2 => {
                // Two columns: bold first, em dash, second.
                if cells.len() >= 2 && !cells[0].is_empty() {
                    let _ = writeln!(out, "• *{}* — {}", cells[0], cells[1]);
                } else if cells.len() >= 2 {
                    let _ = writeln!(out, "• {}", cells[1]);
                }
            }
            _ => {
                // 3+ columns: labeled with · separator.
                let parts: Vec<String> = header
                    .iter()
                    .zip(cells.iter())
                    .filter(|(_, v)| !v.is_empty() && *v != "\u{2014}")
                    .map(|(h, v)| {
                        if h.is_empty() {
                            v.clone()
                        } else {
                            format!("*{h}:* {v}")
                        }
                    })
                    .collect();
                let _ = writeln!(out, "• {}", parts.join(" · "));
            }
        }
    }
}
