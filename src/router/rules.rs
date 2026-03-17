use std::fmt::Write as _;
use std::sync::LazyLock;

use super::context::DirectiveTrigger;
use serde::{Deserialize, Serialize};

static UNMATCHED_PLACEHOLDER_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\$\d+").unwrap());

/// Tool-declared static routing rule. Compiled at startup.
#[derive(Debug, Clone)]
pub struct StaticRule {
    pub tool: String,
    pub trigger: DirectiveTrigger,
    pub params: serde_json::Value,
    /// Only matches when this tool is the `active_tool` in `RouterContext`.
    pub requires_context: bool,
}

impl StaticRule {
    /// Check if this rule matches the message given the active tool context.
    pub fn matches(&self, message: &str, active_tool: Option<&str>) -> bool {
        let normalized = message.trim().to_lowercase();
        self.matches_normalized(&normalized, active_tool)
    }

    /// Match against a pre-lowercased, pre-trimmed message.
    /// Use this when checking multiple rules against the same message to
    /// avoid redundant `to_lowercase()` allocations.
    pub fn matches_normalized(&self, normalized: &str, active_tool: Option<&str>) -> bool {
        if self.requires_context && active_tool != Some(self.tool.as_str()) {
            return false;
        }
        self.trigger.matches_normalized(normalized)
    }
}

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
mod tests {
    use super::*;

    #[test]
    fn test_config_rule_substitute_positional() {
        let rule = ConfigRule {
            trigger: "weather".into(),
            tool: "weather".into(),
            params: serde_json::json!({"location": "$1"}),
        };
        let result = rule.substitute(&["portland"]);
        assert_eq!(result["location"], "portland");
    }

    #[test]
    fn test_config_rule_substitute_remainder() {
        let rule = ConfigRule {
            trigger: "note".into(),
            tool: "memory".into(),
            params: serde_json::json!({"content": "$*"}),
        };
        let result = rule.substitute(&["buy", "milk", "tomorrow"]);
        assert_eq!(result["content"], "buy milk tomorrow");
    }

    #[test]
    fn test_config_rule_missing_arg() {
        let rule = ConfigRule {
            trigger: "weather".into(),
            tool: "weather".into(),
            params: serde_json::json!({"location": "$1", "units": "$2"}),
        };
        let result = rule.substitute(&["portland"]);
        assert_eq!(result["location"], "portland");
        assert_eq!(result["units"], "");
    }

    #[test]
    fn test_parse_prefixed_command() {
        let (cmd, args) = parse_prefixed_command("!weather portland oregon", "!");
        assert_eq!(cmd, "weather");
        assert_eq!(args, vec!["portland", "oregon"]);
    }

    #[test]
    fn test_parse_prefixed_command_no_args() {
        let (cmd, args) = parse_prefixed_command("!todo", "!");
        assert_eq!(cmd, "todo");
        assert!(args.is_empty());
    }

    #[test]
    fn test_parse_prefixed_not_prefixed() {
        let (cmd, _) = parse_prefixed_command("hello world", "!");
        assert_eq!(cmd, "");
    }

    #[test]
    fn test_static_rule_matches_with_context() {
        let rule = StaticRule {
            tool: "rss".into(),
            trigger: DirectiveTrigger::Exact("next".into()),
            params: serde_json::json!({"action": "next"}),
            requires_context: true,
        };
        assert!(rule.matches("next", Some("rss")));
        assert!(!rule.matches("next", Some("cron")));
        assert!(!rule.matches("next", None));
    }

    #[test]
    fn test_static_rule_matches_without_context() {
        let rule = StaticRule {
            tool: "cron".into(),
            trigger: DirectiveTrigger::Exact("list jobs".into()),
            params: serde_json::json!({"action": "list"}),
            requires_context: false,
        };
        assert!(rule.matches("list jobs", None));
        assert!(rule.matches("list jobs", Some("rss")));
    }

    #[test]
    fn test_parse_multi_char_prefix() {
        let (cmd, args) = parse_prefixed_command(">>weather portland", ">>");
        assert_eq!(cmd, "weather");
        assert_eq!(args, vec!["portland"]);
    }

    #[test]
    fn test_config_rule_substitute_json_escape() {
        let rule = ConfigRule {
            trigger: "test".into(),
            tool: "test".into(),
            params: serde_json::json!({"value": "$1"}),
        };
        // This should NOT inject a new JSON key
        let result = rule.substitute(&[r#"foo","injected":"evil"#]);
        assert_eq!(result["value"], r#"foo","injected":"evil"#);
        assert!(result.get("injected").is_none());
    }

    #[test]
    fn test_config_rule_substitute_double_digit() {
        let rule = ConfigRule {
            trigger: "test".into(),
            tool: "test".into(),
            params: serde_json::json!({"a": "$1", "b": "$10"}),
        };
        let args: Vec<&str> = (0..10)
            .map(|i| ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"][i])
            .collect();
        let result = rule.substitute(&args);
        assert_eq!(result["a"], "a");
        assert_eq!(result["b"], "j"); // $10 = 10th arg, not "$1" + "0"
    }

    #[test]
    fn test_config_rule_substitute_escapes_backslash() {
        let rule = ConfigRule {
            trigger: "test".into(),
            tool: "test".into(),
            params: serde_json::json!({"path": "$1"}),
        };
        let result = rule.substitute(&[r"C:\Users\test"]);
        assert_eq!(result["path"], r"C:\Users\test");
    }

    #[test]
    fn test_config_rule_substitute_no_double_substitution() {
        // If a user arg contains "$2", it should NOT be re-substituted
        let rule = ConfigRule {
            trigger: "cmd".into(),
            tool: "test".into(),
            params: serde_json::json!({"content": "$*", "first": "$1"}),
        };
        let result = rule.substitute(&["$2", "foo"]);
        assert_eq!(result["first"], "$2"); // literal $2, not "foo"
        assert_eq!(result["content"], "$2 foo"); // $* contains the literal $2
    }
}
