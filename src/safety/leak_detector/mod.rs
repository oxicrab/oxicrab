use aho_corasick::AhoCorasick;
use base64::Engine;
use regex::Regex;
use tracing::warn;

struct LeakPattern {
    name: &'static str,
    regex: Regex,
    /// Index into the Aho-Corasick automaton's pattern list.
    /// `None` if this pattern has no usable AC prefix and should always run.
    ac_index: Option<usize>,
}

/// A runtime-added pattern for a known secret value (raw, base64, hex).
struct KnownSecretPattern {
    name: String,
    regex: Regex,
}

/// Detects and redacts leaked secrets in outbound text.
///
/// Uses a two-phase approach for plaintext pattern scanning:
/// 1. **Aho-Corasick automaton** — single-pass scan for literal prefixes
///    (e.g. `sk-ant-api`, `xoxb-`, `ghp_`) to identify candidate regions.
/// 2. **Regex validation** — only runs the full regex on patterns whose
///    literal prefix was found, avoiding unnecessary regex work.
pub struct LeakDetector {
    patterns: Vec<LeakPattern>,
    /// Aho-Corasick automaton built from literal prefixes of each pattern.
    ac: AhoCorasick,
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
            // Groq API keys
            ("groq_api_key", r"gsk_[a-zA-Z0-9]{20,200}", "gsk_"),
            // Telegram bot tokens — prefix is ":AA" (digits before colon are variable)
            (
                "telegram_bot_token",
                r"\b[0-9]+:AA[A-Za-z0-9_\-]{33,}",
                ":AA",
            ),
            // Discord bot tokens — no reliable literal prefix exists (pattern
            // is all character classes). Empty prefix means AC cannot filter
            // this pattern, so its regex always runs unconditionally.
            (
                "discord_bot_token",
                r"[A-Za-z0-9_\-]{24}\.[A-Za-z0-9_\-]{6}\.[A-Za-z0-9_\-]{27,200}",
                "",
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
                    patterns.push(LeakPattern {
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

        Self {
            patterns,
            ac,
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
            let hex_str = hex::encode(value.as_bytes());
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

        // Scan base64 candidates (try both standard and URL-safe decoders)
        for candidate in self.base64_candidate_re.find_iter(text) {
            let candidate_str = candidate.as_str();
            let decoded_str = base64::engine::general_purpose::STANDARD
                .decode(candidate_str)
                .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(candidate_str))
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok());
            if let Some(decoded_str) = decoded_str {
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
            if let Some(decoded) = hex::decode(candidate_str).ok()
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

    /// Use the Aho-Corasick automaton to determine which patterns have at least
    /// one literal prefix hit in `text`. Patterns without a usable AC prefix
    /// are always marked as candidates (their regex runs unconditionally).
    /// Returns a boolean vec indexed by `self.patterns` position.
    fn find_candidate_patterns(&self, text: &str) -> Vec<bool> {
        let mut candidates: Vec<bool> =
            self.patterns.iter().map(|p| p.ac_index.is_none()).collect();
        for ac_match in self.ac.find_overlapping_iter(text) {
            let ac_pattern_id = ac_match.pattern().as_usize();
            for (i, pattern) in self.patterns.iter().enumerate() {
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
        let mut matches = Vec::new();

        // Phase 1+2: AC prefix scan → regex validation only on candidates
        let candidate_indices = self.find_candidate_patterns(text);
        for (i, pattern) in self.patterns.iter().enumerate() {
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

        // Encoded content scan (base64/hex decoded then checked)
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
        // Redact plaintext pattern matches (only check patterns with prefix hits)
        let candidate_indices = self.find_candidate_patterns(&result);
        for (i, pattern) in self.patterns.iter().enumerate() {
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
        // (covers cases where LLM encodes a secret to bypass plaintext detection)
        let encoded_matches = self.scan_encoded(&result);
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

#[cfg(test)]
mod tests;
