use std::sync::LazyLock;

use aho_corasick::AhoCorasick;
use base64::Engine;
use regex::Regex;
use tracing::warn;

/// A static leak pattern compiled once and shared across all `LeakDetector` instances.
struct StaticPattern {
    name: &'static str,
    regex: Regex,
    /// Index into the Aho-Corasick automaton's pattern list.
    /// `None` if this pattern has no usable AC prefix and should always run.
    ac_index: Option<usize>,
}

/// Pre-compiled base patterns (AC automaton + regexes) shared across all instances.
struct BasePatterns {
    ac: AhoCorasick,
    patterns: Vec<StaticPattern>,
    /// Pre-compiled regex for base64 candidate extraction (20+ chars)
    base64_candidate_re: Regex,
    /// Pre-compiled regex for hex candidate extraction (40+ chars)
    hex_candidate_re: Regex,
}

static BASE_PATTERNS: LazyLock<BasePatterns> = LazyLock::new(|| {
    // Each entry: (name, regex_pattern, literal_prefix for Aho-Corasick).
    // The prefix must be a literal string that appears at the start of any
    // match for this pattern — used for fast first-pass filtering.
    let pattern_defs: Vec<(&str, &str, &str)> = vec![
        // Anthropic API keys
        (
            "anthropic_api_key",
            r"sk-ant-api[0-9a-zA-Z\-_]{16,200}",
            "sk-ant-api",
        ),
        // OpenAI API keys: project (sk-proj-...), org (sk-org-...),
        // service account (sk-svcacct-...), and legacy (sk-[16+ alphanum]).
        // Legacy pattern excludes sk-ant- (Anthropic, caught separately)
        // by requiring a non-'a' first char, or 'a' followed by non-'n'.
        // Uses "sk-" as prefix since all OpenAI variants start with it.
        (
            "openai_api_key",
            r"sk-(?:proj|org|svcacct)-[a-zA-Z0-9\-_]{16,200}|sk-(?:[b-zB-Z0-9]|a[^n]|an[^t])[a-zA-Z0-9]{13,197}",
            "sk-",
        ),
        // Slack bot tokens
        (
            "slack_bot_token",
            r"xoxb-[0-9]+-[0-9]+-[a-zA-Z0-9]+",
            "xoxb-",
        ),
        // Slack app tokens
        (
            "slack_app_token",
            r"xapp-[0-9]+-[A-Z0-9]+-[0-9]+-[A-Fa-f0-9]+",
            "xapp-",
        ),
        // GitHub PATs (classic)
        ("github_pat", r"ghp_[a-zA-Z0-9]{36}", "ghp_"),
        // GitHub fine-grained PATs
        (
            "github_fine_grained_pat",
            r"github_pat_[a-zA-Z0-9]{22}_[a-zA-Z0-9]{59}",
            "github_pat_",
        ),
        // AWS access key IDs
        ("aws_access_key", r"AKIA[0-9A-Z]{16}", "AKIA"),
        // AWS secret access keys (context-anchored to reduce false positives)
        (
            "aws_secret_access_key",
            r"(?i)aws[_\s]?secret[_\s]?access[_\s]?key[^A-Za-z0-9]{0,20}[A-Za-z0-9+/]{40}",
            "aws",
        ),
        // Groq API keys
        ("groq_api_key", r"gsk_[a-zA-Z0-9]{20,200}", "gsk_"),
        // Telegram bot tokens — prefix is ":AA" (digits before colon are variable)
        (
            "telegram_bot_token",
            r"\b[0-9]+:AA[A-Za-z0-9_\-]{33,}",
            ":AA",
        ),
        // Discord bot tokens — word-boundary anchored to reduce false
        // positives on JWTs and base64 content. No distinctive prefix, so
        // skip AC scanning and always run the regex.
        (
            "discord_bot_token",
            r"\b[A-Za-z0-9_\-]{24}\.[A-Za-z0-9_\-]{6}\.[A-Za-z0-9_\-]{27,200}\b",
            "",
        ),
        // Google API keys
        ("google_api_key", r"AIza[0-9A-Za-z_\-]{35}", "AIza"),
        // Stripe secret keys
        ("stripe_secret_key", r"sk_live_[0-9a-zA-Z]{24,}", "sk_live_"),
        // Stripe publishable keys
        (
            "stripe_publishable_key",
            r"pk_live_[0-9a-zA-Z]{24,}",
            "pk_live_",
        ),
        // SendGrid API keys
        (
            "sendgrid_api_key",
            r"SG\.[0-9A-Za-z_\-]{22}\.[0-9A-Za-z_\-]{43}",
            "SG.",
        ),
    ];

    let mut prefixes = Vec::with_capacity(pattern_defs.len());
    let mut patterns = Vec::with_capacity(pattern_defs.len());

    for (name, regex_str, prefix) in pattern_defs {
        match Regex::new(regex_str) {
            Ok(regex) => {
                let ac_index = if prefix.is_empty() {
                    // No usable prefix — pattern will always run unconditionally
                    None
                } else {
                    let idx = prefixes.len();
                    prefixes.push(prefix);
                    Some(idx)
                };
                patterns.push(StaticPattern {
                    name,
                    regex,
                    ac_index,
                });
            }
            Err(e) => {
                warn!("failed to compile leak pattern '{}': {}", name, e);
            }
        }
    }

    let ac = AhoCorasick::new(&prefixes)
        .expect("aho-corasick automaton should build from literal prefixes");

    BasePatterns {
        ac,
        patterns,
        // Upper bounds prevent DoS via large payloads; API keys never exceed ~200 chars
        base64_candidate_re: Regex::new(r"[A-Za-z0-9+/]{20,500}={0,3}").unwrap(),
        hex_candidate_re: Regex::new(r"[0-9a-fA-F]{40,512}").unwrap(),
    }
});

/// A runtime-added pattern for a known secret value (raw, base64, hex).
struct KnownSecretPattern {
    name: String,
    regex: Regex,
}

/// Detects and redacts leaked secrets in text (inbound and outbound).
///
/// Uses a two-phase approach for plaintext pattern scanning:
/// 1. **Aho-Corasick automaton** — single-pass scan for literal prefixes
///    (e.g. `sk-ant-api`, `xoxb-`, `ghp_`) to identify candidate regions.
/// 2. **Regex validation** — only runs the full regex on patterns whose
///    literal prefix was found, avoiding unnecessary regex work.
///
/// The base patterns (AC automaton + regexes) are compiled once in a
/// `LazyLock` and shared across all instances. Per-instance state is
/// limited to `known_secrets` added at runtime.
pub struct LeakDetector {
    known_secrets: Vec<KnownSecretPattern>,
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
        // Force initialization of the lazy base patterns so any compilation
        // warnings are emitted early rather than on first scan.
        let _ = &*BASE_PATTERNS;
        Self {
            known_secrets: Vec::new(),
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
                    name: format!("{name}_raw"),
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
                        name: format!("{name}_{suffix}"),
                        regex,
                    });
                }
            }
            let hex_str = hex::encode(value.as_bytes());
            let hex_escaped = regex::escape(&hex_str);
            // Match both lowercase and uppercase hex
            if let Ok(regex) = Regex::new(&format!("(?i){hex_escaped}")) {
                self.known_secrets.push(KnownSecretPattern {
                    name: format!("{name}_hex"),
                    regex,
                });
            }
            // URL-encoded variant (percent-encoding)
            let mut url_encoded = String::with_capacity(value.len() * 3);
            for b in value.bytes() {
                use std::fmt::Write;
                let _ = write!(url_encoded, "%{b:02X}");
            }
            let url_escaped = regex::escape(&url_encoded);
            if let Ok(regex) = Regex::new(&format!("(?i){url_escaped}")) {
                self.known_secrets.push(KnownSecretPattern {
                    name: format!("{name}_urlencoded"),
                    regex,
                });
            }
        }
    }

    /// Scan text for base64/hex encoded blobs and check decoded content against
    /// leak patterns. Returns matches found in decoded content.
    fn scan_encoded(text: &str) -> Vec<LeakMatch> {
        let base = &*BASE_PATTERNS;
        let mut matches = Vec::new();

        // Scan base64 candidates (try both standard and URL-safe decoders)
        for candidate in base.base64_candidate_re.find_iter(text) {
            let candidate_str = candidate.as_str();
            let decoded_str = base64::engine::general_purpose::STANDARD
                .decode(candidate_str)
                .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(candidate_str))
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok());
            if let Some(decoded_str) = decoded_str {
                for pattern in &base.patterns {
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
        for candidate in base.hex_candidate_re.find_iter(text) {
            let candidate_str = candidate.as_str();
            if let Some(decoded) = hex::decode(candidate_str).ok()
                && let Ok(decoded_str) = String::from_utf8(decoded)
            {
                for pattern in &base.patterns {
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

    /// Use the Aho-Corasick automaton to determine which base patterns have at
    /// least one literal prefix hit in `text`. Patterns without a usable AC
    /// prefix are always marked as candidates (their regex runs unconditionally).
    /// Returns a boolean vec indexed by `BASE_PATTERNS.patterns` position.
    fn find_candidate_patterns(text: &str) -> Vec<bool> {
        let base = &*BASE_PATTERNS;
        let mut candidates: Vec<bool> =
            base.patterns.iter().map(|p| p.ac_index.is_none()).collect();
        for ac_match in base.ac.find_overlapping_iter(text) {
            let ac_pattern_id = ac_match.pattern().as_usize();
            for (i, pattern) in base.patterns.iter().enumerate() {
                if pattern.ac_index == Some(ac_pattern_id) {
                    candidates[i] = true;
                }
            }
        }
        candidates
    }

    /// Scan text for potential secret leaks (plaintext, encoded, and known secrets).
    ///
    /// Uses a two-phase approach for plaintext detection:
    /// 1. Aho-Corasick single-pass to find which pattern prefixes appear in the text.
    /// 2. Only runs the full regex for patterns whose prefix was found.
    pub fn scan(&self, text: &str) -> Vec<LeakMatch> {
        let base = &*BASE_PATTERNS;
        let mut matches = Vec::new();

        // Phase 1+2: AC prefix scan → regex validation only on candidates
        let candidate_indices = Self::find_candidate_patterns(text);
        for (i, pattern) in base.patterns.iter().enumerate() {
            if !candidate_indices[i] {
                continue;
            }
            for m in pattern.regex.find_iter(text) {
                matches.push(LeakMatch {
                    name: pattern.name,
                    start: m.start(),
                    end: m.end(),
                });
            }
        }

        // Known secret exact matches (raw, base64, hex encodings)
        for ks in &self.known_secrets {
            for m in ks.regex.find_iter(text) {
                matches.push(LeakMatch {
                    name: "known_secret",
                    start: m.start(),
                    end: m.end(),
                });
            }
        }

        // Encoded content scan (base64/hex decoded then checked)
        matches.extend(Self::scan_encoded(text));

        // URL-decoded scan: if text contains percent-encoded sequences,
        // decode and re-scan for patterns that may be hidden by encoding.
        // NOTE: Matches from URL-decoded text have start/end positions relative
        // to the decoded string, not the original. Callers should use scan()
        // for boolean detection only, not for positional extraction.
        if text.contains('%')
            && let Ok(decoded) = urlencoding::decode(text)
            && decoded != text
        {
            let decoded_candidates = Self::find_candidate_patterns(&decoded);
            for (i, pattern) in base.patterns.iter().enumerate() {
                if !decoded_candidates[i] {
                    continue;
                }
                for m in pattern.regex.find_iter(&decoded) {
                    matches.push(LeakMatch {
                        name: pattern.name,
                        start: m.start(),
                        end: m.end(),
                    });
                }
            }
        }
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
        let result = self.redact_inner(text);

        // URL-decode pass: if the (already redacted) text still contains
        // percent-encoded sequences, decode and re-scan. If new secrets are
        // found in the decoded form, return the redacted decoded version.
        // This trades URL-encoding preservation for security — acceptable
        // because the alternative is leaking secrets.
        if result.contains('%')
            && let Ok(decoded) = urlencoding::decode(&result)
            && *decoded != result
        {
            let redacted_decoded = self.redact_inner(&decoded);
            if redacted_decoded != *decoded {
                return redacted_decoded;
            }
        }

        result
    }

    /// Core redaction logic (plaintext patterns, known secrets, encoded blobs).
    /// Separated from `redact()` to allow the URL-decode pass to call it
    /// without infinite recursion.
    fn redact_inner(&self, text: &str) -> String {
        let base = &*BASE_PATTERNS;
        let mut result = text.to_string();
        // Redact plaintext pattern matches (only check patterns with prefix hits).
        // Note: AC candidates are computed on the original text and may become stale
        // as replacements modify the string. This is safe because replacements only
        // shrink text (secrets → "[REDACTED]"), so any pattern matched here was
        // genuinely present. At worst we run a regex on already-redacted text (no-op).
        let candidate_indices = Self::find_candidate_patterns(&result);
        for (i, pattern) in base.patterns.iter().enumerate() {
            if !candidate_indices[i] {
                continue;
            }
            result = pattern
                .regex
                .replace_all(&result, "[REDACTED]")
                .into_owned();
        }
        // Redact known secret exact matches (raw, base64, hex encodings)
        for ks in &self.known_secrets {
            result = ks.regex.replace_all(&result, "[REDACTED]").into_owned();
        }
        // Redact base64/hex-encoded blobs that decode to match generic patterns
        // (covers cases where LLM encodes a secret to bypass plaintext detection).
        let encoded_matches = Self::scan_encoded(&result);
        if !encoded_matches.is_empty() {
            // Merge overlapping ranges to prevent corruption from overlapping replace_range calls
            let mut ranges: Vec<(usize, usize)> =
                encoded_matches.iter().map(|m| (m.start, m.end)).collect();
            ranges.sort_by_key(|r| r.0);
            let mut merged: Vec<(usize, usize)> = Vec::new();
            for (start, end) in ranges {
                if let Some(last) = merged.last_mut()
                    && start <= last.1
                {
                    last.1 = last.1.max(end);
                    continue;
                }
                merged.push((start, end));
            }
            // Replace from end to start to preserve indices
            for (start, end) in merged.into_iter().rev() {
                if start <= result.len()
                    && end <= result.len()
                    && result.is_char_boundary(start)
                    && result.is_char_boundary(end)
                {
                    result.replace_range(start..end, "[REDACTED]");
                }
            }
        }
        result
    }
}

impl Default for LeakDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl oxicrab_core::safety::LeakRedactor for LeakDetector {
    fn redact(&self, text: &str) -> String {
        self.redact(text)
    }
}

#[cfg(test)]
mod tests;
