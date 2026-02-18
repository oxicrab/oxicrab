use regex::Regex;
use tracing::warn;

struct LeakPattern {
    name: &'static str,
    regex: Regex,
}

/// Detects and redacts leaked secrets in outbound text.
pub struct LeakDetector {
    patterns: Vec<LeakPattern>,
}

/// A match found by the leak detector.
#[derive(Debug)]
pub struct LeakMatch {
    pub name: &'static str,
    pub start: usize,
    pub end: usize,
}

impl LeakDetector {
    pub fn new() -> Self {
        let patterns = vec![
            // Anthropic API keys
            ("anthropic_api_key", r"sk-ant-api[0-9a-zA-Z\-_]{20,200}"),
            // OpenAI API keys
            ("openai_api_key", r"sk-[a-zA-Z0-9]{20,200}"),
            // Slack bot tokens
            ("slack_bot_token", r"xoxb-[0-9]+-[0-9]+-[a-zA-Z0-9]+"),
            // Slack app tokens
            ("slack_app_token", r"xapp-[0-9]+-[A-Z0-9]+-[0-9]+-[a-f0-9]+"),
            // GitHub PATs
            ("github_pat", r"ghp_[a-zA-Z0-9]{36}"),
            // Groq API keys
            ("groq_api_key", r"gsk_[a-zA-Z0-9]{20,200}"),
            // Telegram bot tokens
            ("telegram_bot_token", r"[0-9]+:AA[A-Za-z0-9_\-]{33}"),
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

        Self { patterns }
    }

    /// Scan text for potential secret leaks.
    pub fn scan(&self, text: &str) -> Vec<LeakMatch> {
        let mut matches = Vec::new();
        for pattern in &self.patterns {
            for m in pattern.regex.find_iter(text) {
                matches.push(LeakMatch {
                    name: pattern.name,
                    start: m.start(),
                    end: m.end(),
                });
            }
        }
        matches
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
        result
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
}
