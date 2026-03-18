use super::*;

#[test]
fn ansi_escape_matches() {
    assert!(RegexPatterns::ansi_escape().is_match("\x1b[31m"));
    assert!(RegexPatterns::ansi_escape().is_match("\x1b[0m"));
    assert!(!RegexPatterns::ansi_escape().is_match("plain text"));
}

// NOTE: markdown_bold, markdown_link, compile_slack_mention tests moved to oxicrab-channels
// NOTE: compile_regex, compile_security_patterns tests moved to oxicrab-tools-system

// NOTE: html_tags_matches moved to oxicrab-tools-rss
