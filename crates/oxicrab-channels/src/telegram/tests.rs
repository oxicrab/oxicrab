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
    // URL ampersands must be HTML-escaped in href attributes
    assert!(output.contains("href=\"https://example.com?a=1&amp;b=2\""));
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

#[test]
fn test_markdown_to_html_strikethrough() {
    assert_eq!(markdown_to_telegram_html("~~deleted~~"), "<s>deleted</s>");
}

#[test]
fn test_markdown_to_html_strikethrough_mixed() {
    let input = "**bold** and ~~struck~~ text";
    let output = markdown_to_telegram_html(input);
    assert!(output.contains("<b>bold</b>"));
    assert!(output.contains("<s>struck</s>"));
}

#[test]
fn test_extension_from_tg_path_with_extension() {
    assert_eq!(extension_from_tg_path("photos/file_42.png", "jpg"), "png");
}

#[test]
fn test_extension_from_tg_path_no_extension() {
    assert_eq!(extension_from_tg_path("photos/file_42", "jpg"), "jpg");
}

#[test]
fn test_extension_from_tg_path_webp() {
    assert_eq!(extension_from_tg_path("photos/file_42.webp", "jpg"), "webp");
}

#[test]
fn test_build_inline_keyboard_no_buttons() {
    let msg = OutboundMessage::builder("telegram", "123", "hello").build();
    assert!(build_inline_keyboard(&msg, None).is_none());
}

#[test]
fn test_build_inline_keyboard_with_buttons() {
    let msg = OutboundMessage::builder("telegram", "123", "hello")
        .meta(
            meta::BUTTONS,
            serde_json::json!([
                {"id": "yes", "label": "Yes", "style": "primary"},
                {"id": "no", "label": "No", "style": "danger"}
            ]),
        )
        .build();
    let kb = build_inline_keyboard(&msg, None);
    assert!(kb.is_some());
    let kb = kb.unwrap();
    assert_eq!(kb.inline_keyboard.len(), 2);
    assert_eq!(kb.inline_keyboard[0][0].text, "Yes");
    assert_eq!(kb.inline_keyboard[1][0].text, "No");
}

#[test]
fn test_build_inline_keyboard_truncates_callback_data() {
    let long_context = "x".repeat(100);
    let msg = OutboundMessage::builder("telegram", "123", "hello")
        .meta(
            meta::BUTTONS,
            serde_json::json!([
                {"id": "act", "label": "Click", "context": long_context}
            ]),
        )
        .build();
    let kb = build_inline_keyboard(&msg, None).unwrap();
    // callback_data = "act|" + 100 x's = 104 bytes, must be truncated to 64
    let btn = &kb.inline_keyboard[0][0];
    if let teloxide::types::InlineKeyboardButtonKind::CallbackData(ref data) = btn.kind {
        assert!(data.len() <= CALLBACK_DATA_MAX_BYTES);
    } else {
        panic!("expected CallbackData");
    }
}

// --- utf16_substr tests ---

#[test]
fn test_utf16_substr_ascii() {
    assert_eq!(utf16_substr("abc", 0, 3), Some("abc".to_string()));
}

#[test]
fn test_utf16_substr_with_emoji() {
    // "Hello 🌍 world"
    // UTF-16: H(1) e(1) l(1) l(1) o(1) ' '(1) 🌍(2) ' '(1) w(1) o(1) r(1) l(1) d(1)
    // Offsets: 0    1    2    3    4    5      6-7     8     9   10   11   12   13
    // "world" starts at UTF-16 offset 9, length 5
    assert_eq!(
        utf16_substr("Hello 🌍 world", 9, 5),
        Some("world".to_string())
    );
}

#[test]
fn test_utf16_substr_extract_emoji() {
    // Extract the emoji itself (2 UTF-16 code units)
    assert_eq!(utf16_substr("Hello 🌍 world", 6, 2), Some("🌍".to_string()));
}

#[test]
fn test_utf16_substr_out_of_bounds() {
    assert_eq!(utf16_substr("abc", 2, 5), None);
}

#[test]
fn test_utf16_substr_empty_string() {
    assert_eq!(utf16_substr("", 0, 0), Some(String::new()));
}

#[test]
fn test_utf16_substr_zero_length() {
    assert_eq!(utf16_substr("abc", 1, 0), Some(String::new()));
}

// --- markdown_to_telegram_html code block tests ---

#[test]
fn test_markdown_to_html_code_block_preserves_formatting() {
    // Bold inside a code block should NOT be converted
    let input = "```\n**not bold**\n```";
    let output = markdown_to_telegram_html(input);
    assert!(
        output.contains("<pre><code>"),
        "should have code block: {output}"
    );
    assert!(
        !output.contains("<b>"),
        "bold should not be converted inside code block: {output}"
    );
    assert!(
        output.contains("**not bold**") || output.contains("*not bold*"),
        "raw markers should be preserved inside code block: {output}"
    );
}

#[test]
fn test_markdown_to_html_bold_outside_code_block_converted() {
    // Bold outside a code block SHOULD be converted
    let input = "**bold text** and ```\n**raw**\n```";
    let output = markdown_to_telegram_html(input);
    assert!(
        output.contains("<b>bold text</b>"),
        "bold outside code block should convert: {output}"
    );
    // Inside the code block, bold markers should be preserved
    assert!(
        output.contains("<pre><code>"),
        "code block should exist: {output}"
    );
}

#[test]
fn test_markdown_to_html_code_block_with_language() {
    let input = "```rust\nfn main() {}\n```";
    let output = markdown_to_telegram_html(input);
    assert!(
        output.contains("<pre><code>"),
        "code block should be present: {output}"
    );
    assert!(
        output.contains("fn main()"),
        "code content preserved: {output}"
    );
}

#[test]
fn test_markdown_to_html_mixed_code_block_and_formatting() {
    let input = "**header**\n```\ncode here\n```\n_footer_";
    let output = markdown_to_telegram_html(input);
    assert!(
        output.contains("<b>header</b>"),
        "bold before code block: {output}"
    );
    assert!(
        output.contains("<i>footer</i>"),
        "italic after code block: {output}"
    );
    assert!(
        output.contains("<pre><code>"),
        "code block present: {output}"
    );
}
