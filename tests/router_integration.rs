mod common;

use common::{MockLLMProvider, TestAgentOverrides, create_test_agent_with, text_response};
use oxicrab::agent::AgentRunOverrides;
use oxicrab::dispatch::{ActionDispatch, ActionSource};
use oxicrab::router::context::{ActionDirective, DirectiveTrigger, RouterContext};
use oxicrab::router::rules::{ConfigRule, StaticRule, parse_prefixed_command};
use oxicrab::router::{MessageRouter, RoutingDecision};
use serde_json::json;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// RouterContext — session metadata round-trip
// ---------------------------------------------------------------------------

#[test]
fn test_router_context_session_roundtrip_preserves_fields() {
    let now = oxicrab::router::now_ms();
    let ctx = RouterContext {
        active_tool: Some("rss".to_string()),
        action_directives: vec![ActionDirective {
            trigger: DirectiveTrigger::OneOf(vec!["yes".into(), "accept".into()]),
            tool: "rss".into(),
            params: json!({"action": "accept", "article_ids": ["id1"]}),
            single_use: true,
            ttl_ms: 300_000,
            created_at_ms: now,
        }],
        updated_at_ms: now,
    };

    let mut metadata = std::collections::HashMap::new();
    ctx.to_session_metadata(&mut metadata);

    let restored = RouterContext::from_session_metadata(&metadata);
    assert_eq!(restored.active_tool, Some("rss".to_string()));
    assert_eq!(restored.action_directives.len(), 1);
    assert_eq!(restored.updated_at_ms, now);

    let directive = &restored.action_directives[0];
    assert_eq!(directive.tool, "rss");
    assert!(directive.single_use);
    assert_eq!(directive.params["action"], "accept");
}

#[test]
fn test_router_context_session_roundtrip_missing_key_is_default() {
    let metadata = std::collections::HashMap::new();
    let ctx = RouterContext::from_session_metadata(&metadata);
    assert!(ctx.active_tool.is_none());
    assert!(ctx.action_directives.is_empty());
}

#[test]
fn test_router_context_session_roundtrip_malformed_is_default() {
    let mut metadata = std::collections::HashMap::new();
    metadata.insert(
        "router_context".to_string(),
        serde_json::Value::String("not valid json object".into()),
    );
    let ctx = RouterContext::from_session_metadata(&metadata);
    assert!(ctx.active_tool.is_none());
}

// ---------------------------------------------------------------------------
// DirectiveTrigger matching edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_directive_trigger_exact_case_insensitive() {
    let t = DirectiveTrigger::Exact("NEXT".into());
    assert!(t.matches("next"));
    assert!(t.matches("Next"));
    assert!(t.matches("  NEXT  "));
    assert!(!t.matches("next please"));
}

#[test]
fn test_directive_trigger_one_of_empty_vec_never_matches() {
    let t = DirectiveTrigger::OneOf(vec![]);
    assert!(!t.matches("yes"));
    assert!(!t.matches(""));
}

#[test]
fn test_directive_trigger_pattern_oversized_rejected() {
    let giant_pattern = "a".repeat(300);
    let t = DirectiveTrigger::Pattern(giant_pattern);
    // Must not panic, must return false
    assert!(!t.matches("a"));
}

#[test]
fn test_directive_trigger_pattern_invalid_regex_no_panic() {
    let t = DirectiveTrigger::Pattern("[invalid".into());
    assert!(!t.matches("any text"));
}

#[test]
fn test_directive_trigger_pattern_with_captures() {
    let t = DirectiveTrigger::Pattern(r"^accept\s+\S+$".into());
    assert!(t.matches("accept abc123"));
    assert!(t.matches("ACCEPT xyz"));
    assert!(!t.matches("accept"));
    assert!(!t.matches("reject abc123"));
}

// ---------------------------------------------------------------------------
// Directive TTL expiry
// ---------------------------------------------------------------------------

#[test]
fn test_directive_is_expired_when_past_ttl() {
    let now = oxicrab::router::now_ms();
    let d = ActionDirective {
        trigger: DirectiveTrigger::Exact("yes".into()),
        tool: "rss".into(),
        params: json!({}),
        single_use: false,
        ttl_ms: 100,
        created_at_ms: now - 1000, // 1 second old, TTL = 100ms
    };
    assert!(d.is_expired(now));
}

#[test]
fn test_directive_is_not_expired_within_ttl() {
    let now = oxicrab::router::now_ms();
    let d = ActionDirective {
        trigger: DirectiveTrigger::Exact("yes".into()),
        tool: "rss".into(),
        params: json!({}),
        single_use: false,
        ttl_ms: 300_000,
        created_at_ms: now,
    };
    assert!(!d.is_expired(now));
}

#[test]
fn test_prune_expired_removes_only_stale_directives() {
    let now = oxicrab::router::now_ms();
    let mut ctx = RouterContext {
        active_tool: Some("rss".into()),
        action_directives: vec![
            ActionDirective {
                trigger: DirectiveTrigger::Exact("stale".into()),
                tool: "rss".into(),
                params: json!({}),
                single_use: false,
                ttl_ms: 1,
                created_at_ms: now - 2000,
            },
            ActionDirective {
                trigger: DirectiveTrigger::Exact("fresh".into()),
                tool: "rss".into(),
                params: json!({}),
                single_use: false,
                ttl_ms: 300_000,
                created_at_ms: now,
            },
        ],
        updated_at_ms: now,
    };
    ctx.prune_expired(now);
    assert_eq!(ctx.action_directives.len(), 1);
    assert!(
        matches!(&ctx.action_directives[0].trigger, DirectiveTrigger::Exact(s) if s == "fresh")
    );
}

// ---------------------------------------------------------------------------
// Context switch clears directives
// ---------------------------------------------------------------------------

#[test]
fn test_context_switch_to_different_tool_clears_directives() {
    let now = oxicrab::router::now_ms();
    let mut ctx = RouterContext {
        active_tool: Some("rss".into()),
        action_directives: vec![ActionDirective {
            trigger: DirectiveTrigger::Exact("next".into()),
            tool: "rss".into(),
            params: json!({}),
            single_use: false,
            ttl_ms: 300_000,
            created_at_ms: now,
        }],
        updated_at_ms: now,
    };
    ctx.set_active_tool(Some("cron".into()));
    assert!(ctx.action_directives.is_empty());
    assert_eq!(ctx.active_tool, Some("cron".into()));
}

#[test]
fn test_context_switch_to_same_tool_preserves_directives() {
    let now = oxicrab::router::now_ms();
    let mut ctx = RouterContext {
        active_tool: Some("rss".into()),
        action_directives: vec![ActionDirective {
            trigger: DirectiveTrigger::Exact("next".into()),
            tool: "rss".into(),
            params: json!({}),
            single_use: false,
            ttl_ms: 300_000,
            created_at_ms: now,
        }],
        updated_at_ms: now,
    };
    ctx.set_active_tool(Some("rss".into()));
    assert_eq!(ctx.action_directives.len(), 1);
}

#[test]
fn test_context_switch_to_none_clears_directives() {
    let now = oxicrab::router::now_ms();
    let mut ctx = RouterContext {
        active_tool: Some("rss".into()),
        action_directives: vec![ActionDirective {
            trigger: DirectiveTrigger::Exact("next".into()),
            tool: "rss".into(),
            params: json!({}),
            single_use: false,
            ttl_ms: 300_000,
            created_at_ms: now,
        }],
        updated_at_ms: now,
    };
    ctx.set_active_tool(None);
    assert!(ctx.action_directives.is_empty());
    assert!(ctx.active_tool.is_none());
}

// ---------------------------------------------------------------------------
// MAX_DIRECTIVES cap
// ---------------------------------------------------------------------------

#[test]
fn test_install_directives_caps_at_max() {
    use oxicrab::router::context::MAX_DIRECTIVES;

    let now = oxicrab::router::now_ms();
    let mut ctx = RouterContext::default();
    let directives: Vec<ActionDirective> = (0..MAX_DIRECTIVES + 10)
        .map(|i| ActionDirective {
            trigger: DirectiveTrigger::Exact(format!("cmd{i}")),
            tool: "tool".into(),
            params: json!({}),
            single_use: false,
            ttl_ms: 300_000,
            created_at_ms: now,
        })
        .collect();
    ctx.install_directives(directives);
    assert_eq!(ctx.action_directives.len(), MAX_DIRECTIVES);
}

// ---------------------------------------------------------------------------
// remove_directive_at
// ---------------------------------------------------------------------------

#[test]
fn test_remove_directive_at_valid_index() {
    let now = oxicrab::router::now_ms();
    let mut ctx = RouterContext {
        active_tool: None,
        action_directives: vec![
            ActionDirective {
                trigger: DirectiveTrigger::Exact("a".into()),
                tool: "t".into(),
                params: json!({}),
                single_use: true,
                ttl_ms: 300_000,
                created_at_ms: now,
            },
            ActionDirective {
                trigger: DirectiveTrigger::Exact("b".into()),
                tool: "t".into(),
                params: json!({}),
                single_use: true,
                ttl_ms: 300_000,
                created_at_ms: now,
            },
        ],
        updated_at_ms: now,
    };
    ctx.remove_directive_at(0);
    assert_eq!(ctx.action_directives.len(), 1);
    assert!(matches!(&ctx.action_directives[0].trigger, DirectiveTrigger::Exact(s) if s == "b"));
}

#[test]
fn test_remove_directive_at_out_of_bounds_is_noop() {
    let mut ctx = RouterContext::default();
    ctx.remove_directive_at(99); // must not panic
    assert!(ctx.action_directives.is_empty());
}

// ---------------------------------------------------------------------------
// parse_prefixed_command
// ---------------------------------------------------------------------------

#[test]
fn test_parse_prefixed_command_slash_prefix() {
    let (cmd, args) = parse_prefixed_command("/status verbose", "/");
    assert_eq!(cmd, "status");
    assert_eq!(args, vec!["verbose"]);
}

#[test]
fn test_parse_prefixed_command_multi_char_prefix() {
    let (cmd, args) = parse_prefixed_command(">>note buy milk tomorrow", ">>");
    assert_eq!(cmd, "note");
    assert_eq!(args, vec!["buy", "milk", "tomorrow"]);
}

#[test]
fn test_parse_prefixed_command_no_args() {
    let (cmd, args) = parse_prefixed_command("!help", "!");
    assert_eq!(cmd, "help");
    assert!(args.is_empty());
}

#[test]
fn test_parse_prefixed_command_not_prefixed_returns_empty() {
    let (cmd, args) = parse_prefixed_command("hello world", "!");
    assert_eq!(cmd, "");
    assert!(args.is_empty());
}

#[test]
fn test_parse_prefixed_command_only_prefix_returns_empty_cmd() {
    let (cmd, _) = parse_prefixed_command("!", "!");
    assert_eq!(cmd, "");
}

#[test]
fn test_parse_prefixed_command_leading_whitespace_before_prefix() {
    // trim() is called on the message first
    let (cmd, args) = parse_prefixed_command("  !weather london", "!");
    assert_eq!(cmd, "weather");
    assert_eq!(args, vec!["london"]);
}

// ---------------------------------------------------------------------------
// ConfigRule substitution
// ---------------------------------------------------------------------------

#[test]
fn test_config_rule_substitute_multiple_positional() {
    let rule = ConfigRule {
        trigger: "route".into(),
        tool: "maps".into(),
        params: json!({"from": "$1", "to": "$2"}),
    };
    let result = rule.substitute(&["portland", "seattle"]);
    assert_eq!(result["from"], "portland");
    assert_eq!(result["to"], "seattle");
}

#[test]
fn test_config_rule_substitute_remainder_multi_word() {
    let rule = ConfigRule {
        trigger: "note".into(),
        tool: "memory".into(),
        params: json!({"content": "$*"}),
    };
    let result = rule.substitute(&["remember", "to", "buy", "groceries"]);
    assert_eq!(result["content"], "remember to buy groceries");
}

#[test]
fn test_config_rule_substitute_missing_arg_replaced_with_empty() {
    let rule = ConfigRule {
        trigger: "weather".into(),
        tool: "weather".into(),
        params: json!({"location": "$1", "units": "$2"}),
    };
    let result = rule.substitute(&["portland"]);
    assert_eq!(result["location"], "portland");
    // $2 was missing — replaced with ""
    assert_eq!(result["units"], "");
}

#[test]
fn test_config_rule_substitute_no_args() {
    let rule = ConfigRule {
        trigger: "list".into(),
        tool: "cron".into(),
        params: json!({"action": "list"}),
    };
    let result = rule.substitute(&[]);
    assert_eq!(result["action"], "list");
}

// ---------------------------------------------------------------------------
// StaticRule matching
// ---------------------------------------------------------------------------

#[test]
fn test_static_rule_context_required_matches_only_right_tool() {
    let rule = StaticRule {
        tool: "rss".into(),
        trigger: DirectiveTrigger::Exact("next".into()),
        params: json!({"action": "next"}),
        requires_context: true,
    };
    assert!(rule.matches("next", Some("rss")));
    assert!(!rule.matches("next", Some("cron")));
    assert!(!rule.matches("next", None));
}

#[test]
fn test_static_rule_no_context_required_matches_any() {
    let rule = StaticRule {
        tool: "cron".into(),
        trigger: DirectiveTrigger::Exact("list jobs".into()),
        params: json!({"action": "list"}),
        requires_context: false,
    };
    assert!(rule.matches("list jobs", None));
    assert!(rule.matches("list jobs", Some("rss")));
    assert!(rule.matches("list jobs", Some("cron")));
}

// ---------------------------------------------------------------------------
// MessageRouter — priority ordering
// ---------------------------------------------------------------------------

fn make_router() -> MessageRouter {
    let static_rules = vec![
        StaticRule {
            tool: "rss".into(),
            trigger: DirectiveTrigger::OneOf(vec!["next".into(), "more".into()]),
            params: json!({"action": "next"}),
            requires_context: true,
        },
        StaticRule {
            tool: "cron".into(),
            trigger: DirectiveTrigger::Exact("list jobs".into()),
            params: json!({"action": "list"}),
            requires_context: false,
        },
    ];
    let mut config_rules = std::collections::HashMap::new();
    config_rules.insert(
        "weather".into(),
        ConfigRule {
            trigger: "weather".into(),
            tool: "weather_tool".into(),
            params: json!({"location": "$1"}),
        },
    );
    MessageRouter::new(static_rules, config_rules, "!".into())
}

#[test]
fn test_router_action_dispatch_takes_priority_over_everything() {
    let router = make_router();
    let mut ctx = RouterContext::default();
    let now = oxicrab::router::now_ms();
    // Even with a matching directive, explicit ActionDispatch wins
    ctx.action_directives.push(ActionDirective {
        trigger: DirectiveTrigger::Exact("yes".into()),
        tool: "rss".into(),
        params: json!({}),
        single_use: false,
        ttl_ms: 300_000,
        created_at_ms: now,
    });

    let dispatch = ActionDispatch {
        tool: "calendar".into(),
        params: json!({"action": "list_events"}),
        source: ActionSource::Button {
            action_id: "btn-1".into(),
        },
    };

    let decision = router.route("yes", &ctx, Some(&dispatch));
    match decision {
        RoutingDecision::DirectDispatch { tool, source, .. } => {
            assert_eq!(tool, "calendar");
            assert!(matches!(source, oxicrab::router::DispatchSource::Button));
        }
        _ => panic!("expected DirectDispatch from Button, got {decision:?}"),
    }
}

#[test]
fn test_router_empty_message_returns_full_llm() {
    let router = make_router();
    let ctx = RouterContext::default();
    let decision = router.route("", &ctx, None);
    assert!(matches!(decision, RoutingDecision::FullLLM));
}

#[test]
fn test_router_expired_directive_is_skipped() {
    let router = make_router();
    let now = oxicrab::router::now_ms();
    let mut ctx = RouterContext::default();
    ctx.action_directives.push(ActionDirective {
        trigger: DirectiveTrigger::Exact("yes".into()),
        tool: "rss".into(),
        params: json!({}),
        single_use: false,
        ttl_ms: 1,
        created_at_ms: now - 5000, // well past TTL
    });
    let decision = router.route("yes", &ctx, None);
    // Expired directive is skipped → falls through to FullLLM (no static rule for "yes")
    assert!(matches!(decision, RoutingDecision::FullLLM));
}

#[test]
fn test_router_live_directive_matched_before_static_rules() {
    let router = make_router();
    let now = oxicrab::router::now_ms();
    let mut ctx = RouterContext {
        active_tool: Some("rss".into()),
        ..Default::default()
    };
    // Add a directive for "next" — it should take priority over the static RSS rule
    ctx.action_directives.push(ActionDirective {
        trigger: DirectiveTrigger::Exact("next".into()),
        tool: "rss".into(),
        params: json!({"action": "next", "source": "directive"}),
        single_use: true,
        ttl_ms: 300_000,
        created_at_ms: now,
    });

    let decision = router.route("next", &ctx, None);
    match decision {
        RoutingDecision::DirectDispatch { source, params, .. } => {
            assert!(
                matches!(source, oxicrab::router::DispatchSource::ActionDirective),
                "expected ActionDirective source"
            );
            assert_eq!(params["source"], "directive");
        }
        _ => panic!("expected ActionDirective dispatch, got {decision:?}"),
    }
}

#[test]
fn test_router_config_command_dispatched() {
    let router = make_router();
    let ctx = RouterContext::default();
    let decision = router.route("!weather seattle", &ctx, None);
    match decision {
        RoutingDecision::DirectDispatch {
            tool,
            params,
            source,
        } => {
            assert_eq!(tool, "weather_tool");
            assert_eq!(params["location"], "seattle");
            assert!(matches!(
                source,
                oxicrab::router::DispatchSource::ConfigRule
            ));
        }
        _ => panic!("expected ConfigRule dispatch, got {decision:?}"),
    }
}

#[test]
fn test_router_config_command_unknown_falls_through() {
    let router = make_router();
    let ctx = RouterContext::default();
    // "!unknown" is not a registered config rule — should fall through to FullLLM
    let decision = router.route("!unknown stuff", &ctx, None);
    assert!(matches!(decision, RoutingDecision::FullLLM));
}

#[test]
fn test_router_static_rule_with_matching_context() {
    let router = make_router();
    let ctx = RouterContext {
        active_tool: Some("rss".into()),
        ..Default::default()
    };
    let decision = router.route("next", &ctx, None);
    assert!(matches!(
        decision,
        RoutingDecision::DirectDispatch {
            source: oxicrab::router::DispatchSource::StaticRule,
            ..
        }
    ));
}

#[test]
fn test_router_static_rule_wrong_context_falls_to_guided_llm() {
    let router = make_router();
    let ctx = RouterContext {
        active_tool: Some("cron".into()),
        ..Default::default()
    };
    // "next" requires rss context, cron context → no static rule match → GuidedLLM
    let decision = router.route("next", &ctx, None);
    assert!(matches!(decision, RoutingDecision::GuidedLLM { .. }));
}

#[test]
fn test_router_context_free_static_rule_any_context() {
    let router = make_router();
    // "list jobs" is context-free — should match with no active tool
    let ctx_none = RouterContext::default();
    assert!(matches!(
        router.route("list jobs", &ctx_none, None),
        RoutingDecision::DirectDispatch {
            source: oxicrab::router::DispatchSource::StaticRule,
            ..
        }
    ));
    // Also matches with a different active tool
    let ctx_rss = RouterContext {
        active_tool: Some("rss".into()),
        ..Default::default()
    };
    assert!(matches!(
        router.route("list jobs", &ctx_rss, None),
        RoutingDecision::DirectDispatch {
            source: oxicrab::router::DispatchSource::StaticRule,
            ..
        }
    ));
}

#[test]
fn test_router_remember_fast_path() {
    let router = make_router();
    let ctx = RouterContext::default();
    let decision = router.route("remember that my favorite color is blue", &ctx, None);
    assert!(matches!(
        decision,
        RoutingDecision::DirectDispatch {
            source: oxicrab::router::DispatchSource::RememberFastPath,
            ..
        }
    ));
}

#[test]
fn test_router_full_llm_fallback() {
    let router = make_router();
    let ctx = RouterContext::default();
    let decision = router.route("what is the weather today?", &ctx, None);
    assert!(matches!(decision, RoutingDecision::FullLLM));
}

#[test]
fn test_router_guided_llm_with_active_tool_no_match() {
    let router = make_router();
    let ctx = RouterContext {
        active_tool: Some("rss".into()),
        ..Default::default()
    };
    // Message doesn't match any static rule for rss, but active_tool is set → GuidedLLM
    let decision = router.route("show me something interesting", &ctx, None);
    match decision {
        RoutingDecision::GuidedLLM {
            tool_subset,
            context_hint,
        } => {
            assert_eq!(tool_subset, vec!["rss"]);
            assert!(
                context_hint.contains("rss"),
                "context hint should mention the active tool"
            );
        }
        _ => panic!("expected GuidedLLM, got {decision:?}"),
    }
}

#[test]
fn test_router_matched_directive_index() {
    let router = make_router();
    let now = oxicrab::router::now_ms();
    let mut ctx = RouterContext::default();
    ctx.action_directives.push(ActionDirective {
        trigger: DirectiveTrigger::Exact("no".into()),
        tool: "rss".into(),
        params: json!({}),
        single_use: true,
        ttl_ms: 300_000,
        created_at_ms: now,
    });
    ctx.action_directives.push(ActionDirective {
        trigger: DirectiveTrigger::Exact("yes".into()),
        tool: "rss".into(),
        params: json!({}),
        single_use: true,
        ttl_ms: 300_000,
        created_at_ms: now,
    });
    assert_eq!(router.matched_directive_index("yes", &ctx), Some(1));
    assert_eq!(router.matched_directive_index("no", &ctx), Some(0));
    assert_eq!(router.matched_directive_index("maybe", &ctx), None);
}

#[test]
fn test_router_matched_directive_index_expired_is_none() {
    let router = make_router();
    let now = oxicrab::router::now_ms();
    let mut ctx = RouterContext::default();
    ctx.action_directives.push(ActionDirective {
        trigger: DirectiveTrigger::Exact("yes".into()),
        tool: "rss".into(),
        params: json!({}),
        single_use: true,
        ttl_ms: 1,
        created_at_ms: now - 5000,
    });
    assert_eq!(router.matched_directive_index("yes", &ctx), None);
}

// ---------------------------------------------------------------------------
// Integration: ActionDispatch through process_direct_with_overrides
// ---------------------------------------------------------------------------

/// Verify that an explicit ActionDispatch bypasses the LLM entirely.
/// The MockLLMProvider call count should remain zero after the dispatch.
#[tokio::test]
async fn test_direct_dispatch_via_process_direct_with_overrides_bypasses_llm() {
    let tmp = TempDir::new().expect("create temp dir");
    let provider = MockLLMProvider::with_responses(vec![text_response("Should not be called")]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;

    // Dispatch list_dir directly — no LLM should be invoked
    let dispatch = ActionDispatch {
        tool: "list_dir".into(),
        params: json!({"path": tmp.path().to_str().unwrap()}),
        source: ActionSource::Button {
            action_id: "btn-list-dir".into(),
        },
    };
    let overrides = AgentRunOverrides {
        action: Some(dispatch),
        ..Default::default()
    };

    let result = agent
        .process_direct_with_overrides(
            "ignored content",
            "test:direct_dispatch",
            "telegram",
            "chat1",
            &overrides,
        )
        .await
        .expect("process_direct_with_overrides");

    // Tool ran — result contains directory listing output
    assert!(
        !result.content.is_empty(),
        "direct dispatch should produce tool output"
    );

    // LLM was never called
    let recorded = calls.lock().expect("lock calls");
    assert_eq!(
        recorded.len(),
        0,
        "LLM must not be called for direct dispatch, got {} calls",
        recorded.len()
    );
}

/// Verify that dispatch to an unknown tool returns an error message, not a panic.
#[tokio::test]
async fn test_direct_dispatch_unknown_tool_returns_error() {
    let tmp = TempDir::new().expect("create temp dir");
    let provider = MockLLMProvider::with_responses(vec![]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;

    let dispatch = ActionDispatch {
        tool: "nonexistent_tool_xyz".into(),
        params: json!({}),
        source: ActionSource::Button {
            action_id: "btn-bad".into(),
        },
    };
    let overrides = AgentRunOverrides {
        action: Some(dispatch),
        ..Default::default()
    };

    let result = agent
        .process_direct_with_overrides("", "test:direct_bad", "telegram", "chat1", &overrides)
        .await
        .expect("should not Err");

    assert!(
        result.content.contains("not available") || result.content.contains("Action failed"),
        "expected error message about unavailable tool, got: {}",
        result.content
    );

    // No LLM calls
    let recorded = calls.lock().expect("lock calls");
    assert_eq!(recorded.len(), 0, "LLM must not be called");
}

/// Normal process_direct (no action dispatch) goes through the LLM.
#[tokio::test]
async fn test_normal_process_direct_uses_llm() {
    let tmp = TempDir::new().expect("create temp dir");
    let provider = MockLLMProvider::with_responses(vec![text_response("LLM response here")]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;

    let response = agent
        .process_direct("Hello agent", "test:normal", "telegram", "chat1")
        .await
        .expect("process_direct");

    assert_eq!(response, "LLM response here");

    let recorded = calls.lock().expect("lock calls");
    assert_eq!(recorded.len(), 1, "LLM should be called exactly once");
}

/// Verify that different ActionSource variants are accepted without error.
#[tokio::test]
async fn test_direct_dispatch_with_cron_action_source() {
    let tmp = TempDir::new().expect("create temp dir");
    let provider = MockLLMProvider::with_responses(vec![]);

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;

    let dispatch = ActionDispatch {
        tool: "list_dir".into(),
        params: json!({"path": tmp.path().to_str().unwrap()}),
        source: ActionSource::Cron {
            job_id: "job-123".into(),
        },
    };
    let overrides = AgentRunOverrides {
        action: Some(dispatch),
        ..Default::default()
    };

    let result = agent
        .process_direct_with_overrides(
            "list the directory",
            "test:cron_dispatch",
            "telegram",
            "chat1",
            &overrides,
        )
        .await
        .expect("cron action dispatch should succeed");

    assert!(
        !result.content.is_empty(),
        "cron dispatch should produce output"
    );
}
