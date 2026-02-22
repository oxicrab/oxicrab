use super::*;

#[test]
fn test_short_message_no_split() {
    let result = split_message("hello world", 100);
    assert_eq!(result, vec!["hello world"]);
}

#[test]
fn test_exact_limit_no_split() {
    let msg = "a".repeat(100);
    let result = split_message(&msg, 100);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].len(), 100);
}

#[test]
fn test_split_at_paragraph_boundary() {
    let msg = "first paragraph\n\nsecond paragraph";
    let result = split_message(msg, 25);
    assert_eq!(result, vec!["first paragraph", "second paragraph"]);
}

#[test]
fn test_split_at_newline_boundary() {
    let msg = "first line\nsecond line\nthird line";
    let result = split_message(msg, 20);
    assert_eq!(result[0], "first line");
}

#[test]
fn test_hard_cut_no_boundary() {
    let msg = "a".repeat(200);
    let result = split_message(&msg, 100);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].len(), 100);
    assert_eq!(result[1].len(), 100);
}

#[test]
fn test_utf8_multibyte_boundary_safety() {
    // Each emoji is 4 bytes. 25 chars * 4 bytes = 100 bytes
    let msg = "\u{1F600}".repeat(25);
    assert_eq!(msg.len(), 100);
    // Split at 10 bytes — should not land in the middle of a 4-byte char
    let result = split_message(&msg, 10);
    for chunk in &result {
        // Each chunk must be valid UTF-8 (would panic on construction if not)
        assert!(!chunk.is_empty());
        // Verify all chars are complete
        for c in chunk.chars() {
            assert_eq!(c, '\u{1F600}');
        }
    }
}

#[test]
fn test_utf8_two_byte_chars() {
    // é is 2 bytes in UTF-8
    let msg = "é".repeat(60); // 120 bytes
    let result = split_message(&msg, 50);
    for chunk in &result {
        for c in chunk.chars() {
            assert_eq!(c, 'é');
        }
    }
}

#[test]
fn test_empty_message() {
    let result = split_message("", 100);
    assert_eq!(result, vec![""]);
}

#[test]
fn test_paragraph_preferred_over_newline() {
    let msg = "line1\nline2\n\nline3\nline4";
    let result = split_message(msg, 20);
    // Should split at \n\n first
    assert_eq!(result[0], "line1\nline2");
}

#[test]
fn test_multiple_chunks() {
    let msg = "chunk1\n\nchunk2\n\nchunk3\n\nchunk4";
    let result = split_message(msg, 10);
    assert!(result.len() >= 4);
}
