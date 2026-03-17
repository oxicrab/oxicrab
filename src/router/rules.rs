use super::context::DirectiveTrigger;
use serde::{Deserialize, Serialize};

/// Tool-declared static routing rule. Compiled at startup.
#[derive(Clone)]
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
        if self.requires_context && active_tool != Some(self.tool.as_str()) {
            return false;
        }
        self.trigger.matches(message)
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
    pub fn substitute(&self, args: &[&str]) -> serde_json::Value {
        let template = serde_json::to_string(&self.params).unwrap_or_default();
        let mut result = template.clone();

        // $* = entire remainder joined by spaces
        let remainder = args.join(" ");
        result = result.replace("$*", &remainder);

        // $1, $2, ... positional
        for (i, arg) in args.iter().enumerate() {
            result = result.replace(&format!("${}", i + 1), arg);
        }

        // Clean up unmatched $N references
        let re = regex::Regex::new(r"\$\d+").unwrap();
        result = re.replace_all(&result, "").to_string();

        serde_json::from_str(&result).unwrap_or(self.params.clone())
    }
}

/// Parse a prefixed command message. Returns (`command_word`, args).
/// If message doesn't start with the prefix, returns ("", vec![]).
pub fn parse_prefixed_command<'a>(message: &'a str, prefix: &str) -> (&'a str, Vec<&'a str>) {
    let trimmed = message.trim();
    if !trimmed.starts_with(prefix) {
        return ("", vec![]);
    }
    let without_prefix = &trimmed[prefix.len()..];
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
}
