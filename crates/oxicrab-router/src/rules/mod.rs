use std::fmt::Write as _;
use std::sync::LazyLock;

pub use oxicrab_core::tools::base::routing_types::{DirectiveTrigger, StaticRule};
use serde::{Deserialize, Serialize};

static UNMATCHED_PLACEHOLDER_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\$\d+").unwrap());

/// User-defined prefix command from config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigRule {
    pub trigger: String,
    pub tool: String,
    pub params: serde_json::Value,
}

impl ConfigRule {
    /// Substitute $1, $2, ... and $* in params with positional args.
    /// All arguments are JSON-escaped before substitution to prevent injection.
    pub fn substitute(&self, args: &[&str]) -> serde_json::Value {
        let template = serde_json::to_string(&self.params).unwrap_or_default();
        let mut result = template;

        // Positional args FIRST (descending order to prevent $1 matching inside $10).
        // Must run before $* to prevent double-substitution: if a user arg contains
        // "$2" and $* expands it, the positional loop would substitute it again.
        for (i, arg) in args.iter().enumerate().rev() {
            let escaped = json_escape(arg);
            result = result.replace(&format!("${}", i + 1), &escaped);
        }

        // $* remainder — user values have $ escaped as \u0024 by json_escape,
        // so they won't match the $N cleanup regex below.
        let remainder = json_escape(&args.join(" "));
        result = result.replace("$*", &remainder);

        // Clean up unmatched $N template references (only real placeholders remain,
        // since user-provided $ is escaped to \u0024 by json_escape).
        result = UNMATCHED_PLACEHOLDER_RE
            .replace_all(&result, "")
            .to_string();

        serde_json::from_str(&result).unwrap_or(self.params.clone())
    }
}

/// Escape a string for safe embedding inside a JSON string value.
/// Escapes backslash, double quote, and control characters.
fn json_escape(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            // Escape $ to prevent substituted values from being treated as
            // template placeholders by the $N cleanup regex.
            '$' => escaped.push_str("\\u0024"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(escaped, "\\u{:04x}", c as u32);
            }
            c => escaped.push(c),
        }
    }
    escaped
}

/// Parse a prefixed command message. Returns (`command_word`, args).
/// If message doesn't start with the prefix, returns ("", vec![]).
pub fn parse_prefixed_command<'a>(message: &'a str, prefix: &str) -> (&'a str, Vec<&'a str>) {
    let trimmed = message.trim();
    let Some(without_prefix) = trimmed.strip_prefix(prefix) else {
        return ("", vec![]);
    };
    let mut parts = without_prefix.split_whitespace();
    let command = parts.next().unwrap_or("");
    let args: Vec<&str> = parts.collect();
    (command, args)
}

#[cfg(test)]
mod tests;
