use super::*;

#[test]
fn ansi_escape_matches() {
    assert!(RegexPatterns::ansi_escape().is_match("\x1b[31m"));
    assert!(RegexPatterns::ansi_escape().is_match("\x1b[0m"));
    assert!(!RegexPatterns::ansi_escape().is_match("plain text"));
}

// NOTE: markdown_bold, markdown_link, compile_slack_mention tests moved to oxicrab-channels
// NOTE: compile_regex, compile_security_patterns tests moved to oxicrab-tools-system

#[test]
fn html_tags_matches() {
    assert!(RegexPatterns::html_tags().is_match("<div>"));
    assert!(RegexPatterns::html_tags().is_match("</p>"));
    assert!(!RegexPatterns::html_tags().is_match("no tags here"));
}
