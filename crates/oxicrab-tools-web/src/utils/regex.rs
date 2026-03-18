use anyhow::{Context, Result};
use regex::Regex;
use std::sync::LazyLock;

/// Compiled regex patterns reused across web tools.
pub struct RegexPatterns;

impl RegexPatterns {
    /// Regex for matching HTML script tags
    pub fn html_script() -> &'static Regex {
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(?i)<script[\s\S]*?</script>")
                .expect("Failed to compile HTML script regex")
        });
        &RE
    }

    /// Regex for matching HTML style tags
    pub fn html_style() -> &'static Regex {
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(?i)<style[\s\S]*?</style>").expect("Failed to compile HTML style regex")
        });
        &RE
    }

    /// Regex for matching HTML tags
    pub fn html_tags() -> &'static Regex {
        static RE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"<[^>]+>").expect("Failed to compile HTML tags regex"));
        &RE
    }

    /// Regex for matching whitespace
    pub fn whitespace() -> &'static Regex {
        static RE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"[ \t]+").expect("Failed to compile whitespace regex"));
        &RE
    }

    /// Regex for matching multiple newlines
    pub fn newlines() -> &'static Regex {
        static RE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"\n{3,}").expect("Failed to compile newlines regex"));
        &RE
    }
}

/// Compile a regex pattern with proper error handling.
pub fn compile_regex(pattern: &str) -> Result<Regex> {
    Regex::new(pattern).with_context(|| format!("Failed to compile regex pattern: {pattern}"))
}
