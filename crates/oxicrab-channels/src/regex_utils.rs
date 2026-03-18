#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-slack",
    feature = "channel-whatsapp",
    feature = "channel-twilio",
))]
use anyhow::{Context, Result};
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-slack",
    feature = "channel-whatsapp",
    feature = "channel-twilio",
))]
use regex::Regex;
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-slack",
    feature = "channel-whatsapp",
    feature = "channel-twilio",
))]
use std::sync::LazyLock;

/// Compiled regex patterns used by channel implementations.
pub struct RegexPatterns;

impl RegexPatterns {
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
    #[cfg(feature = "channel-telegram")]
    pub fn markdown_italic() -> &'static Regex {
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"_(.+?)_").expect("Failed to compile markdown italic regex")
        });
        &RE
    }

    /// Regex for matching markdown code (`code`)
    #[cfg(feature = "channel-telegram")]
    pub fn markdown_code() -> &'static Regex {
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"`([^`]+)`").expect("Failed to compile markdown code regex")
        });
        &RE
    }

    /// Fenced code blocks: ```lang\n...\n```
    #[cfg(feature = "channel-telegram")]
    pub fn markdown_code_block() -> &'static Regex {
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(?s)```(\w*)\n(.*?)```")
                .expect("Failed to compile markdown code block regex")
        });
        &RE
    }

    /// Regex for matching markdown table separator rows (e.g. `|---|---|`)
    #[cfg(any(
        feature = "channel-slack",
        feature = "channel-discord",
        feature = "channel-whatsapp",
        feature = "channel-twilio",
    ))]
    pub fn markdown_table_separator() -> &'static Regex {
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^\|[-| :]+\|\s*$")
                .expect("Failed to compile markdown table separator regex")
        });
        &RE
    }
}

/// Compile a regex pattern with proper error handling
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-slack",
    feature = "channel-whatsapp",
    feature = "channel-twilio",
))]
pub fn compile_regex(pattern: &str) -> Result<Regex> {
    Regex::new(pattern).with_context(|| format!("Failed to compile regex pattern: {pattern}"))
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
    let pattern = format!(r"<@{escaped_id}\s*>\s*");
    compile_regex(&pattern)
        .with_context(|| format!("Failed to compile Slack mention regex for bot_id: {bot_id}"))
}
