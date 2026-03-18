//! Security regex patterns for command validation.

use anyhow::{Context, Result};
use regex::Regex;

/// Compile a regex pattern with proper error handling.
fn compile_regex(pattern: &str) -> Result<Regex> {
    Regex::new(pattern).with_context(|| format!("Failed to compile regex pattern: {pattern}"))
}

/// Get cached security patterns for command validation.
/// Patterns are compiled once on first call and reused.
pub fn compile_security_patterns() -> Result<Vec<Regex>> {
    static PATTERNS: std::sync::LazyLock<Result<Vec<Regex>, String>> = std::sync::LazyLock::new(
        || {
            let patterns = vec![
                r"\brm\s+-[rf]{1,2}\b",
                r"\brm\s+--(?:recursive|force)\b",
                r"\bdel\s+/[fq]\b",
                r"\brmdir\s+/s\b",
                r"(?:^|\s)(format|mkfs|diskpart)\b",
                r"\bdd\s+if=",
                r">\s*/dev/sd",
                r"\b(shutdown|reboot|poweroff)\b",
                r":\(\)\s*\{.{0,100}\};\s*:",
                r"\beval\b",
                r"\bbase64\b.*\|\s*(sh|bash|zsh)\b",
                r"\b(curl|wget)\b.*\|\s*(sh|bash|zsh|python)\b",
                r"\b(curl|wget)\b.*(-d\s*@|--data(-binary|-raw|-urlencode)?\s*@|-F\s|--form\s|--post-file)",
                r"\bpython[23]?(?:\.[0-9]+)?\s+-c\b",
                r"\b(perl|ruby)\b\s+-[EeXx]",
                r"\bchmod\b.*\bo?[0-7]*7[0-7]{0,2}\b",
                r"\bchown\b",
                r"\b(useradd|userdel|usermod|passwd|adduser|deluser)\b",
                r"\$\(",
                r"`[^`]+`",
                r"\$\{[^}]+\}",
                r"<\s*/|<\s*~",
                r"\$[A-Za-z_][A-Za-z0-9_]*",
                r"\b(nc|ncat|netcat)\b.*-[elp]",
                r"\bxxd\b.*-r.*\|\s*(sh|bash|zsh)\b",
                r"\bprintf\b.*\\x.*\|\s*(sh|bash|zsh)\b",
                r"\bnode\b\s+-e\b",
                r"\bphp\b\s+-r\b",
            ];

            let mut compiled = Vec::new();
            for pattern in patterns {
                compiled.push(compile_regex(pattern).map_err(|e| e.to_string())?);
            }
            Ok(compiled)
        },
    );

    PATTERNS
        .as_ref()
        .map(Clone::clone)
        .map_err(|e| anyhow::anyhow!("{e}"))
}
