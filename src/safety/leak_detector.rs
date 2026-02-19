use base64::Engine;
use regex::Regex;
use std::fmt::Write;
use tracing::warn;

struct LeakPattern {
    name: &'static str,
    regex: Regex,
}

/// A runtime-added pattern for a known secret value (raw, base64, hex).
struct KnownSecretPattern {
    name: String,
    regex: Regex,
}

/// Detects and redacts leaked secrets in outbound text.
pub struct LeakDetector {
    patterns: Vec<LeakPattern>,
    known_secrets: Vec<KnownSecretPattern>,
    /// Pre-compiled regex for base64 candidate extraction (20+ chars)
    base64_candidate_re: Regex,
    /// Pre-compiled regex for hex candidate extraction (40+ chars)
    hex_candidate_re: Regex,
}

/// A match found by the leak detector.
#[derive(Debug)]
pub struct LeakMatch {
    pub name: &'static str,
    pub start: usize,
    pub end: usize,
}

/// A match from a known secret pattern (owned name).
#[derive(Debug)]
pub struct KnownSecretMatch {
    pub name: String,
}

impl LeakDetector {
    pub fn new() -> Self {
        let patterns = vec![
            // Anthropic API keys
            ("anthropic_api_key", r"sk-ant-api[0-9a-zA-Z\-_]{20,200}"),
            // OpenAI API keys: project (sk-proj-...), org (sk-org-...),
            // service account (sk-svcacct-...), and legacy (sk-[20+ alphanum]).
            // Legacy pattern excludes sk-ant- (Anthropic, caught separately)
            // by requiring a non-'a' first char, or 'a' followed by non-'n'.
            (
                "openai_api_key",
                r"sk-(?:proj|org|svcacct)-[a-zA-Z0-9\-_]{20,200}|sk-(?:[b-zB-Z0-9]|a[^n]|an[^t])[a-zA-Z0-9]{17,197}",
            ),
            // Slack bot tokens
            ("slack_bot_token", r"xoxb-[0-9]+-[0-9]+-[a-zA-Z0-9]+"),
            // Slack app tokens
            (
                "slack_app_token",
                r"xapp-[0-9]+-[A-Z0-9]+-[0-9]+-[A-Fa-f0-9]+",
            ),
            // GitHub PATs
            ("github_pat", r"ghp_[a-zA-Z0-9]{36}"),
            // Groq API keys
            ("groq_api_key", r"gsk_[a-zA-Z0-9]{20,200}"),
            // Telegram bot tokens
            ("telegram_bot_token", r"[0-9]+:AA[A-Za-z0-9_\-]{33,}"),
            // Discord bot tokens
            (
                "discord_bot_token",
                r"[A-Za-z0-9_\-]{24}\.[A-Za-z0-9_\-]{6}\.[A-Za-z0-9_\-]{27,200}",
            ),
        ];

        let patterns = patterns
            .into_iter()
            .filter_map(|(name, pattern)| match Regex::new(pattern) {
                Ok(regex) => Some(LeakPattern { name, regex }),
                Err(e) => {
                    warn!("failed to compile leak pattern '{}': {}", name, e);
                    None
                }
            })
            .collect();

        Self {
            patterns,
            known_secrets: Vec::new(),
            // Upper bounds prevent DoS via large payloads; API keys never exceed ~200 chars
            base64_candidate_re: Regex::new(r"[A-Za-z0-9+/]{20,500}={0,3}").unwrap(),
            hex_candidate_re: Regex::new(r"[0-9a-fA-F]{40,512}").unwrap(),
        }
    }

    /// Register known secret values for exact-match detection across encodings.
    ///
    /// Takes `(name, value)` pairs. For each secret that is 10+ chars, creates
    /// regex patterns matching the raw value, its base64 encoding, and its hex
    /// encoding. Shorter secrets are skipped to avoid false positives.
    pub fn add_known_secrets(&mut self, secrets: &[(&str, &str)]) {
        for &(name, value) in secrets {
            if value.len() < 10 {
                continue;
            }
            let escaped = regex::escape(value);
            if let Ok(regex) = Regex::new(&escaped) {
                self.known_secrets.push(KnownSecretPattern {
                    name: format!("{}_raw", name),
                    regex,
                });
            }
            // Check both standard and URL-safe base64 encodings
            let b64_standard = base64::engine::general_purpose::STANDARD.encode(value.as_bytes());
            let b64_url_safe =
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value.as_bytes());
            for (suffix, b64) in [("base64", &b64_standard), ("base64url", &b64_url_safe)] {
                let b64_escaped = regex::escape(b64);
                if let Ok(regex) = Regex::new(&b64_escaped) {
                    self.known_secrets.push(KnownSecretPattern {
                        name: format!("{}_{}", name, suffix),
                        regex,
                    });
                }
            }
            let mut hex_str = String::with_capacity(value.len() * 2);
            for b in value.as_bytes() {
                let _ = write!(hex_str, "{:02x}", b);
            }
            let hex_escaped = regex::escape(&hex_str);
            // Match both lowercase and uppercase hex
            if let Ok(regex) = Regex::new(&format!("(?i){}", hex_escaped)) {
                self.known_secrets.push(KnownSecretPattern {
                    name: format!("{}_hex", name),
                    regex,
                });
            }
        }
    }

    /// Scan text for base64/hex encoded blobs and check decoded content against
    /// leak patterns. Returns matches found in decoded content.
    fn scan_encoded(&self, text: &str) -> Vec<LeakMatch> {
        let mut matches = Vec::new();

        // Scan base64 candidates
        for candidate in self.base64_candidate_re.find_iter(text) {
            let candidate_str = candidate.as_str();
            if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(candidate_str)
                && let Ok(decoded_str) = String::from_utf8(decoded)
            {
                for pattern in &self.patterns {
                    if pattern.regex.is_match(&decoded_str) {
                        matches.push(LeakMatch {
                            name: pattern.name,
                            start: candidate.start(),
                            end: candidate.end(),
                        });
                    }
                }
            }
        }

        // Scan hex candidates
        for candidate in self.hex_candidate_re.find_iter(text) {
            let candidate_str = candidate.as_str();
            if let Some(decoded) = decode_hex(candidate_str)
                && let Ok(decoded_str) = String::from_utf8(decoded)
            {
                for pattern in &self.patterns {
                    if pattern.regex.is_match(&decoded_str) {
                        matches.push(LeakMatch {
                            name: pattern.name,
                            start: candidate.start(),
                            end: candidate.end(),
                        });
                    }
                }
            }
        }

        matches
    }

    /// Scan text for potential secret leaks (plaintext, encoded, and known secrets).
    pub fn scan(&self, text: &str) -> Vec<LeakMatch> {
        let mut matches = Vec::new();

        // Plaintext pattern scan
        for pattern in &self.patterns {
            for m in pattern.regex.find_iter(text) {
                matches.push(LeakMatch {
                    name: pattern.name,
                    start: m.start(),
                    end: m.end(),
                });
            }
        }

        // Encoded content scan
        matches.extend(self.scan_encoded(text));

        matches
    }

    /// Scan for known secret exact matches. Returns owned match descriptions.
    pub fn scan_known_secrets(&self, text: &str) -> Vec<KnownSecretMatch> {
        self.known_secrets
            .iter()
            .filter(|ks| ks.regex.is_match(text))
            .map(|ks| KnownSecretMatch {
                name: ks.name.clone(),
            })
            .collect()
    }

    /// Redact any detected secrets in text, replacing them with `[REDACTED]`.
    pub fn redact(&self, text: &str) -> String {
        let mut result = text.to_string();
        for pattern in &self.patterns {
            result = pattern
                .regex
                .replace_all(&result, "[REDACTED]")
                .into_owned();
        }
        for ks in &self.known_secrets {
            result = ks.regex.replace_all(&result, "[REDACTED]").into_owned();
        }
        result
    }
}

/// Decode a hex string to bytes. Returns None if odd length or invalid hex.
fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for chunk in hex.as_bytes().chunks(2) {
        let hi = hex_digit(chunk[0])?;
        let lo = hex_digit(chunk[1])?;
        bytes.push((hi << 4) | lo);
    }
    Some(bytes)
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

impl Default for LeakDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
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
        let text =
            "Keys: ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij and gsk_abcdefghijklmnopqrstuvwx";
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
        let mut hex = String::with_capacity(secret.len() * 2);
        for b in secret.as_bytes() {
            let _ = write!(hex, "{:02x}", b);
        }
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
        let mut hex = String::with_capacity(secret.len() * 2);
        for b in secret.as_bytes() {
            let _ = write!(hex, "{:02x}", b);
        }
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
        assert_eq!(decode_hex("48656c6c6f"), Some(b"Hello".to_vec()));
    }

    #[test]
    fn test_decode_hex_odd_length() {
        assert_eq!(decode_hex("123"), None);
    }

    #[test]
    fn test_decode_hex_invalid_chars() {
        assert_eq!(decode_hex("zzzz"), None);
    }
}
