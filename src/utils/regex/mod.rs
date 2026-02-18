use anyhow::{Context, Result};
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

    /// Regex for matching markdown bold (**text**)
    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-discord",
        feature = "channel-slack",
        feature = "channel-whatsapp",
        feature = "channel-twilio",
    ))]
    pub fn markdown_bold() -> &'static Regex {
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"\*\*(.+?)\*\*").expect("Failed to compile markdown bold regex")
        });
        &RE
    }

    /// Regex for matching markdown strike-through (~~text~~)
    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-discord",
        feature = "channel-slack",
        feature = "channel-whatsapp",
        feature = "channel-twilio",
    ))]
    pub fn markdown_strike() -> &'static Regex {
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"~~(.+?)~~").expect("Failed to compile markdown strike regex")
        });
        &RE
    }

    /// Regex for matching markdown links ([text](url))
    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-discord",
        feature = "channel-slack",
        feature = "channel-whatsapp",
        feature = "channel-twilio",
    ))]
    pub fn markdown_link() -> &'static Regex {
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").expect("Failed to compile markdown link regex")
        });
        &RE
    }

    /// Regex for matching markdown italic (_text_)
    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-discord",
        feature = "channel-slack",
        feature = "channel-whatsapp",
        feature = "channel-twilio",
    ))]
    pub fn markdown_italic() -> &'static Regex {
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"_(.+?)_").expect("Failed to compile markdown italic regex")
        });
        &RE
    }

    /// Regex for matching markdown code (`code`)
    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-discord",
        feature = "channel-slack",
        feature = "channel-whatsapp",
        feature = "channel-twilio",
    ))]
    pub fn markdown_code() -> &'static Regex {
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"`([^`]+)`").expect("Failed to compile markdown code regex")
        });
        &RE
    }

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

    /// Regex for matching double newlines (paragraph breaks)
    pub fn double_newlines() -> &'static Regex {
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"\n\s*\n+").expect("Failed to compile double newlines regex")
        });
        &RE
    }

    /// Regex for matching words (alphanumeric + underscore, 2+ chars)
    pub fn words() -> &'static Regex {
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"[A-Za-z0-9_]{2,}").expect("Failed to compile words regex")
        });
        &RE
    }
}

/// Compile a regex pattern with proper error handling
pub fn compile_regex(pattern: &str) -> Result<Regex> {
    Regex::new(pattern).with_context(|| format!("Failed to compile regex pattern: {}", pattern))
}

/// Compile a regex pattern for Slack mention matching
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-slack",
    feature = "channel-whatsapp",
    feature = "channel-twilio",
))]
pub fn compile_slack_mention(bot_id: &str) -> Result<Regex> {
    let escaped_id = regex::escape(bot_id);
    let pattern = format!(r"<@{}\s*>\s*", escaped_id);
    compile_regex(&pattern).with_context(|| {
        format!(
            "Failed to compile Slack mention regex for bot_id: {}",
            bot_id
        )
    })
}

/// Get cached security patterns for command validation.
/// Patterns are compiled once on first call and reused.
pub fn compile_security_patterns() -> Result<Vec<Regex>> {
    static PATTERNS: std::sync::LazyLock<Result<Vec<Regex>, String>> =
        std::sync::LazyLock::new(|| {
            let patterns = vec![
                r"\brm\s+-[rf]{1,2}\b",
                r"\brm\s+--(?:recursive|force)\b",
                r"\bdel\s+/[fq]\b",
                r"\brmdir\s+/s\b",
                r"\b(format|mkfs|diskpart)\b",
                r"\bdd\s+if=",
                r">\s*/dev/sd",
                r"\b(shutdown|reboot|poweroff)\b",
                r":\(\)\s*\{.*\};\s*:",
                r"\beval\b",
                r"\bbase64\b.*\|\s*(sh|bash|zsh)\b",
                r"\b(curl|wget)\b.*\|\s*(sh|bash|zsh|python)\b",
                r"\bpython[23]?\s+-c\b",
                r"\bchmod\b.*\bo?[0-7]*7[0-7]{0,2}\b",
                r"\bchown\b",
                r"\b(useradd|userdel|usermod|passwd|adduser|deluser)\b",
                // Shell metacharacter injection: command substitution and variable expansion
                r"\$\(",        // $(command) substitution
                r"`[^`]+`",     // `command` backtick substitution
                r"\$\{[^}]+\}", // ${VAR} variable expansion
            ];

            let mut compiled = Vec::new();
            for pattern in patterns {
                compiled.push(compile_regex(pattern).map_err(|e| e.to_string())?);
            }
            Ok(compiled)
        });

    PATTERNS
        .as_ref()
        .map(Clone::clone)
        .map_err(|e| anyhow::anyhow!("{}", e))
}

#[cfg(test)]
mod tests;
