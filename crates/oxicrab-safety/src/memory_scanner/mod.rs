use regex::Regex;
use std::sync::LazyLock;
use tracing::warn;

use crate::LeakDetector;

struct InjectionPattern {
    name: &'static str,
    regex: Regex,
}

static INJECTION_PATTERNS: LazyLock<Vec<InjectionPattern>> = LazyLock::new(|| {
    let defs: Vec<(&str, &str)> = vec![
        // Role override attempts
        (
            "role_override",
            r"(?i)\b(?:ignore|disregard|forget)\b.{0,30}\b(?:previous|prior|above|all)\b.{0,20}\b(?:instructions?|prompts?|rules?)\b",
        ),
        (
            "new_identity",
            r"(?i)\byou\s+are\s+now\b.{0,50}\b(?:new|different|another)\b",
        ),
        (
            "system_prompt_override",
            r"(?i)\b(?:new|override|replace|overwrite)\b.{0,20}\b(?:system\s+prompt|instructions)\b",
        ),
        // Secret extraction attempts
        (
            "secret_extraction",
            r"(?i)\b(?:reveal|show|output|print|display|leak|expose)\b.{0,30}\b(?:system\s+prompt|api\s*key|secret|credential|password|token)\b",
        ),
        // Instruction hijacking
        (
            "instruction_hijack",
            r"(?i)\b(?:from\s+now\s+on|new\s+instructions?:|override:)\b",
        ),
        // Role markers that could confuse LLMs when surfaced as memory
        ("role_marker", r"(?m)^(?:system|assistant|human|user):\s"),
    ];

    defs.into_iter()
        .filter_map(|(name, pattern)| {
            Regex::new(pattern)
                .ok()
                .map(|regex| InjectionPattern { name, regex })
        })
        .collect()
});

/// Result of scanning memory content for injection patterns.
pub struct MemoryScanResult {
    pub content: String,
    pub secrets_redacted: bool,
    pub injections_stripped: usize,
}

/// Sanitize content before writing to memory.
///
/// Runs leak detection (secret redaction) and injection pattern
/// stripping. Returns the sanitized content with metadata about
/// what was changed. Never errors — returns best-effort sanitized
/// content so memory writes are not blocked.
pub fn scan_memory_content(content: &str, leak_detector: &LeakDetector) -> MemoryScanResult {
    // Phase 1: redact secrets
    let redacted = leak_detector.redact(content);
    let secrets_redacted = redacted != content;
    if secrets_redacted {
        warn!(
            "memory write: redacted secrets from content ({} bytes)",
            content.len()
        );
    }

    // Phase 2: strip injection patterns line-by-line
    let mut injections_stripped = 0;
    let sanitized: Vec<&str> = redacted
        .lines()
        .filter(|line| {
            for pattern in INJECTION_PATTERNS.iter() {
                if pattern.regex.is_match(line) {
                    warn!(
                        "memory write: stripped injection pattern '{}' from content",
                        pattern.name
                    );
                    injections_stripped += 1;
                    return false;
                }
            }
            true
        })
        .collect();

    let content = if injections_stripped > 0 {
        sanitized.join("\n")
    } else {
        redacted
    };

    MemoryScanResult {
        content,
        secrets_redacted,
        injections_stripped,
    }
}

/// Redact secrets from memory search results before returning to the LLM.
///
/// Only runs leak detection (no injection scanning) since the content
/// is being read, not written.
pub fn redact_memory_output(content: &str, leak_detector: &LeakDetector) -> String {
    let redacted = leak_detector.redact(content);
    if redacted != content {
        warn!(
            "memory search: redacted secrets from search results ({} bytes)",
            content.len()
        );
    }
    redacted
}

#[cfg(test)]
mod tests;
