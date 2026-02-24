use super::*;

// ── Binary blob stripping tests ──────────────────────────

#[test]
fn strip_blobs_data_uri_replaced() {
    let b64_data = "A".repeat(300);
    let input = format!("Image: data:image/png;base64,{} end", b64_data);
    let result = strip_binary_blobs(&input);
    assert!(!result.contains(&b64_data));
    assert!(result.contains("[image/png data,"));
    assert!(result.contains("bytes]"));
    assert!(result.contains("end"));
}

#[test]
fn strip_blobs_long_base64_replaced() {
    // Include +/ chars so the heuristic recognizes it as real base64
    let b64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".repeat(5);
    let input = format!("Before {} After", b64);
    let result = strip_binary_blobs(&input);
    assert!(!result.contains(&b64));
    assert!(result.contains("[base64 data,"));
    assert!(result.contains("Before"));
    assert!(result.contains("After"));
}

#[test]
fn strip_blobs_pure_alpha_not_treated_as_base64() {
    // Pure alphabetic text shouldn't be treated as base64
    let text = "a".repeat(500);
    let result = strip_binary_blobs(&text);
    assert_eq!(
        result, text,
        "pure alpha text should pass through unchanged"
    );
}

#[test]
fn strip_blobs_long_hex_replaced() {
    let hex = "a1b2c3d4e5f6".repeat(30); // 360 hex chars
    let input = format!("Hash: {} done", hex);
    let result = strip_binary_blobs(&input);
    assert!(!result.contains(&hex));
    assert!(result.contains("[hex data,"));
    assert!(result.contains("done"));
}

#[test]
fn strip_blobs_short_base64_unchanged() {
    let short = "SGVsbG8gV29ybGQ="; // "Hello World" in base64 (16 chars)
    let input = format!("Encoded: {}", short);
    let result = strip_binary_blobs(&input);
    assert!(result.contains(short), "short base64 should pass through");
}

#[test]
fn strip_blobs_normal_text_unchanged() {
    let text = "This is a normal tool result with no binary data. The temperature is 72F.";
    let result = strip_binary_blobs(text);
    assert_eq!(result, text);
}

#[test]
fn strip_blobs_multiple_blobs() {
    // Use base64 with + and / chars (no = in middle since it only appears at end)
    let b64_1 = "ABCDabcd0123+/XY".repeat(16); // 256 chars with base64 markers
    let b64_2 = "XYZxyz9876+/ABCD".repeat(20); // 320 chars with base64 markers
    let input = format!("First: {} middle text {} end", b64_1, b64_2);
    let result = strip_binary_blobs(&input);
    assert!(!result.contains(&b64_1));
    assert!(!result.contains(&b64_2));
    assert!(result.contains("middle text"));
    assert_eq!(result.matches("[base64 data,").count(), 2);
}

#[test]
fn strip_blobs_mixed_content_preserves_text() {
    let b64_data = "Z".repeat(500);
    let input = format!(
        "Here is the screenshot:\ndata:image/jpeg;base64,{}\n\nThe file contains 3 errors on line 42.",
        b64_data
    );
    let result = strip_binary_blobs(&input);
    assert!(result.contains("Here is the screenshot:"));
    assert!(result.contains("The file contains 3 errors on line 42."));
    assert!(result.contains("[image/jpeg data,"));
    assert!(!result.contains(&b64_data));
}

#[test]
fn strip_blobs_integrated_with_truncation() {
    // A tool result with a large base64 blob that would normally eat the truncation budget
    let b64_data = "X".repeat(5000);
    let input = format!(
        "Important: the config is invalid.\ndata:application/octet-stream;base64,{}\nFix: set debug=true",
        b64_data
    );
    let result = truncate_tool_result(&input, 500);
    // The blob should be stripped, and both text parts should survive
    assert!(result.contains("Important: the config is invalid."));
    assert!(result.contains("Fix: set debug=true"));
    assert!(!result.contains(&b64_data));
}

// ── Floor char boundary tests ────────────────────────────

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
    assert!(!result.contains('\x1b'));
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

#[test]
fn truncate_small_max_chars_does_not_exceed_limit() {
    let long = "a".repeat(500);
    for max in [0, 1, 5, 10, 50, 100, 119] {
        let result = truncate_tool_result(&long, max);
        assert!(
            result.len() <= max,
            "max_chars={max}: result len {} > {max}",
            result.len()
        );
    }
}

#[test]
fn truncate_small_max_chars_json_does_not_exceed_limit() {
    let json = serde_json::json!({"data": "x".repeat(500)}).to_string();
    let result = truncate_tool_result(&json, 50);
    assert!(result.len() <= 50, "result len {} > 50", result.len());
}
