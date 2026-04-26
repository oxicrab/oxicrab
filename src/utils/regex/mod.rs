use regex::Regex;
use std::sync::LazyLock;

/// Compiled regex patterns that are reused across the codebase
pub struct RegexPatterns;

impl RegexPatterns {
    /// Regex for matching ANSI escape codes
    pub fn ansi_escape() -> &'static Regex {
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").expect("Failed to compile ANSI escape regex")
        });
        &RE
    }

    /// Regex for matching data URIs (`data:mime/type;base64,...`)
    pub fn data_uri() -> &'static Regex {
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"data:[a-zA-Z0-9][a-zA-Z0-9!#$&\-^_.+]*(?:/[a-zA-Z0-9][a-zA-Z0-9!#$&\-^_.+]*)?;base64,[A-Za-z0-9+/=]{200,}")
                .expect("Failed to compile data URI regex")
        });
        &RE
    }

    /// Regex for matching long base64 sequences (>=200 chars of base64 alphabet)
    pub fn long_base64() -> &'static Regex {
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"[A-Za-z0-9+/]{200,}={0,3}").expect("Failed to compile long base64 regex")
        });
        &RE
    }

    /// Regex for matching long hex sequences (>=200 chars of hex digits)
    pub fn long_hex() -> &'static Regex {
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"\b[0-9a-fA-F]{200,}\b").expect("Failed to compile long hex regex")
        });
        &RE
    }

    /// Regex for matching `<think>...</think>` blocks (models like `DeepSeek`, `Qwen`)
    pub fn think_tags() -> &'static Regex {
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(?si)<think>.*?</think>\s*").expect("Failed to compile think tags regex")
        });
        &RE
    }
}

#[cfg(test)]
mod tests;
