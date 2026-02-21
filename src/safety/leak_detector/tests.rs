use super::*;
use base64::Engine;

#[test]
fn test_detect_anthropic_key() {
    let detector = LeakDetector::new();
    let text = "My key is sk-ant-api03-abcdefghijklmnopqrst12345";
    let matches = detector.scan(text);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].name, "anthropic_api_key");
}

#[test]
fn test_detect_openai_key() {
    let detector = LeakDetector::new();
    let text = "Use this key: sk-abcdefghijklmnopqrstuvwx";
    let matches = detector.scan(text);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].name, "openai_api_key");
}

#[test]
fn test_detect_github_pat() {
    let detector = LeakDetector::new();
    let text = "Token: ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij";
    let matches = detector.scan(text);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].name, "github_pat");
}

#[test]
fn test_detect_slack_bot_token() {
    let detector = LeakDetector::new();
    let text = "Bot token: xoxb-123456-789012-abcdefghij";
    let matches = detector.scan(text);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].name, "slack_bot_token");
}

#[test]
fn test_detect_groq_key() {
    let detector = LeakDetector::new();
    let text = "Groq key: gsk_abcdefghijklmnopqrstuvwx";
    let matches = detector.scan(text);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].name, "groq_api_key");
}

#[test]
fn test_detect_telegram_token() {
    let detector = LeakDetector::new();
    let text = "Token: 123456789:AAabcdefghijklmnopqrstuvwxyz1234567";
    let matches = detector.scan(text);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].name, "telegram_bot_token");
}

#[test]
fn test_no_false_positives_on_normal_text() {
    let detector = LeakDetector::new();
    let text = "Hello, this is a normal message. The temperature is 72F.";
    let matches = detector.scan(text);
    assert!(matches.is_empty());
}

#[test]
fn test_redact_replaces_secrets() {
    let detector = LeakDetector::new();
    let text = "Key: sk-ant-api03-abcdefghijklmnopqrst12345 is secret";
    let redacted = detector.redact(text);
    assert!(!redacted.contains("sk-ant-api03"));
    assert!(redacted.contains("[REDACTED]"));
    assert!(redacted.contains("is secret"));
}

#[test]
fn test_redact_multiple_secrets() {
    let detector = LeakDetector::new();
    let text = "Keys: ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij and gsk_abcdefghijklmnopqrstuvwx";
    let redacted = detector.redact(text);
    assert!(!redacted.contains("ghp_"));
    assert!(!redacted.contains("gsk_"));
    assert_eq!(redacted.matches("[REDACTED]").count(), 2);
}

#[test]
fn test_short_sk_prefix_no_match() {
    let detector = LeakDetector::new();
    // "sk-" followed by fewer than 20 chars should not match
    let text = "This is sk-short";
    let matches = detector.scan(text);
    assert!(matches.is_empty());
}

// --- Three-encoding tests ---

#[test]
fn test_detect_base64_encoded_anthropic_key() {
    let detector = LeakDetector::new();
    let secret = "sk-ant-api03-abcdefghijklmnopqrst12345";
    let encoded = base64::engine::general_purpose::STANDARD.encode(secret.as_bytes());
    let text = format!("Here is encoded data: {}", encoded);
    let matches = detector.scan(&text);
    assert!(
        !matches.is_empty(),
        "Should detect base64-encoded Anthropic key"
    );
}

#[test]
fn test_detect_hex_encoded_openai_key() {
    let detector = LeakDetector::new();
    let secret = "sk-abcdefghijklmnopqrstuvwx";
    let hex = hex::encode(secret.as_bytes());
    let text = format!("Hex payload: {}", hex);
    let matches = detector.scan(&text);
    assert!(!matches.is_empty(), "Should detect hex-encoded OpenAI key");
}

#[test]
fn test_add_known_secrets_detects_all_encodings() {
    let mut detector = LeakDetector::new();
    let secret = "my-super-secret-api-key-12345";
    detector.add_known_secrets(&[("test_secret", secret)]);

    // Raw
    let raw_matches = detector.scan_known_secrets(secret);
    assert!(!raw_matches.is_empty(), "Should detect raw secret");

    // Base64
    let b64 = base64::engine::general_purpose::STANDARD.encode(secret.as_bytes());
    let b64_matches = detector.scan_known_secrets(&b64);
    assert!(!b64_matches.is_empty(), "Should detect base64 secret");

    // Hex
    let hex = hex::encode(secret.as_bytes());
    let hex_matches = detector.scan_known_secrets(&hex);
    assert!(!hex_matches.is_empty(), "Should detect hex secret");
}

#[test]
fn test_known_secrets_short_value_skipped() {
    let mut detector = LeakDetector::new();
    detector.add_known_secrets(&[("short", "abc")]);
    assert!(
        detector.known_secrets.is_empty(),
        "Secrets shorter than 10 chars should be skipped"
    );
}

#[test]
fn test_no_false_positives_on_normal_base64() {
    let detector = LeakDetector::new();
    // base64 of "Hello, World!" — should not trigger any patterns
    let text = "SGVsbG8sIFdvcmxkIQ==";
    let matches = detector.scan(text);
    assert!(
        matches.is_empty(),
        "Normal base64 should not trigger leak detection"
    );
}

#[test]
fn test_redact_covers_known_secrets() {
    let mut detector = LeakDetector::new();
    let secret = "my-super-secret-api-key-12345";
    detector.add_known_secrets(&[("test", secret)]);

    let b64 = base64::engine::general_purpose::STANDARD.encode(secret.as_bytes());
    let text = format!("Leak: {} and also {}", secret, b64);
    let redacted = detector.redact(&text);
    assert!(!redacted.contains(secret));
    assert!(!redacted.contains(&b64));
}

#[test]
fn test_decode_hex_valid() {
    assert_eq!(hex::decode("48656c6c6f").ok(), Some(b"Hello".to_vec()));
}

#[test]
fn test_decode_hex_odd_length() {
    assert!(hex::decode("123").is_err());
}

#[test]
fn test_decode_hex_invalid_chars() {
    assert!(hex::decode("zzzz").is_err());
}
