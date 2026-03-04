use super::*;

#[test]
fn test_markdown_to_html_plain_text() {
    assert_eq!(markdown_to_telegram_html("hello world"), "hello world");
}

#[test]
fn test_markdown_to_html_empty() {
    assert_eq!(markdown_to_telegram_html(""), "");
}

#[test]
fn test_markdown_to_html_bold() {
    assert_eq!(
        markdown_to_telegram_html("hello **world**"),
        "hello <b>world</b>"
    );
}

#[test]
fn test_markdown_to_html_italic() {
    // Italic regex uses underscores, not asterisks
    assert_eq!(
        markdown_to_telegram_html("hello _world_"),
        "hello <i>world</i>"
    );
}

#[test]
fn test_markdown_to_html_code() {
    assert_eq!(
        markdown_to_telegram_html("run `cargo test`"),
        "run <code>cargo test</code>"
    );
}

#[test]
fn test_markdown_to_html_link() {
    assert_eq!(
        markdown_to_telegram_html("[click](https://example.com)"),
        r#"<a href="https://example.com">click</a>"#
    );
}

#[test]
fn test_markdown_to_html_escapes_html() {
    let result = markdown_to_telegram_html("<script>alert('xss')</script>");
    assert!(result.contains("&lt;script&gt;"));
    assert!(result.contains("&lt;/script&gt;"));
    // Single quotes are escaped (encoding may vary)
    assert!(!result.contains("<script>"));
}

#[test]
fn test_markdown_to_html_ampersand() {
    assert_eq!(markdown_to_telegram_html("A & B"), "A &amp; B");
}

#[test]
fn test_markdown_to_html_mixed() {
    let input = "**bold** and _italic_ with `code`";
    let output = markdown_to_telegram_html(input);
    assert!(output.contains("<b>bold</b>"));
    assert!(output.contains("<i>italic</i>"));
    assert!(output.contains("<code>code</code>"));
}

#[test]
fn test_markdown_to_html_multiple_bold() {
    assert_eq!(
        markdown_to_telegram_html("**a** and **b**"),
        "<b>a</b> and <b>b</b>"
    );
}

#[test]
fn test_markdown_to_html_multiple_links() {
    let input = "[one](https://one.com) and [two](https://two.com)";
    let expected = r#"<a href="https://one.com">one</a> and <a href="https://two.com">two</a>"#;
    assert_eq!(markdown_to_telegram_html(input), expected);
}

#[test]
fn test_markdown_to_html_link_with_ampersand_in_url() {
    let input = "[search](https://example.com?a=1&b=2)";
    let output = markdown_to_telegram_html(input);
    // URL should NOT be double-escaped -- links are extracted before HTML escaping
    assert!(output.contains("href=\"https://example.com?a=1&b=2\""));
}

#[test]
fn test_markdown_to_html_newlines_preserved() {
    let input = "line 1\nline 2";
    assert_eq!(markdown_to_telegram_html(input), "line 1\nline 2");
}

#[test]
fn test_markdown_to_html_angle_brackets() {
    assert_eq!(markdown_to_telegram_html("a > b < c"), "a &gt; b &lt; c");
}

#[test]
fn test_markdown_to_html_code_preserves_special_chars() {
    // Inside inline code, special chars should be HTML-escaped too
    // (escaping happens before markdown conversion)
    let input = "`a < b`";
    let output = markdown_to_telegram_html(input);
    assert!(output.contains("<code>"));
    assert!(output.contains("&lt;"));
}
