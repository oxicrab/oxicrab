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
