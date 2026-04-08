use super::*;
use oxicrab_core::dispatch::{ActionDispatch, ActionSource};

fn make_router() -> MessageRouter {
    let static_rules = vec![
        rules::StaticRule {
            tool: "rss".into(),
            trigger: context::DirectiveTrigger::OneOf(vec!["next".into(), "more".into()]),
            params: serde_json::json!({"action": "next"}),
            requires_context: true,
        },
        rules::StaticRule {
            tool: "cron".into(),
            trigger: context::DirectiveTrigger::Exact("list jobs".into()),
            params: serde_json::json!({"action": "list"}),
            requires_context: false,
        },
    ];
    let config_rules = vec![rules::ConfigRule {
        trigger: "weather".into(),
        tool: "weather".into(),
        params: serde_json::json!({"location": "$1"}),
    }];
    MessageRouter::new(static_rules, config_rules, "!".into())
}

#[test]
fn test_config_rule_conflict_keeps_first() {
    let router = MessageRouter::new(
        vec![],
        vec![
            rules::ConfigRule {
                trigger: "weather".into(),
                tool: "weather".into(),
                params: serde_json::json!({"action":"forecast"}),
            },
            rules::ConfigRule {
                trigger: "WeAtHeR".into(),
                tool: "cron".into(),
                params: serde_json::json!({"action":"list"}),
            },
        ],
        "!".into(),
    );
    let ctx = context::RouterContext::default();
    let decision = router.route("!weather seattle", &ctx, None);
    match decision {
        RoutingDecision::DirectDispatch { tool, .. } => assert_eq!(tool, "weather"),
        _ => panic!("expected direct dispatch"),
    }
}

#[test]
fn test_route_with_semantic_emits_semantic_filter() {
    let router = MessageRouter::new(vec![], vec![], "!".into());
    let ctx = context::RouterContext::default();
    let decision = router.route_with_semantic(
        "hello there",
        &ctx,
        None,
        Some(vec!["rss".into(), "cron".into(), "rss".into()]),
    );
    match decision {
        RoutingDecision::SemanticFilter { policy } => {
            assert_eq!(policy.reason, "semantic_filter");
            assert_eq!(policy.allowed_tools, vec!["cron", "rss"]);
        }
        _ => panic!("expected semantic filter"),
    }
}

#[test]
fn test_route_with_semantic_ignores_single_tool() {
    let router = MessageRouter::new(vec![], vec![], "!".into());
    let ctx = context::RouterContext::default();
    let decision = router.route_with_semantic("hello there", &ctx, None, Some(vec!["rss".into()]));
    assert!(matches!(decision, RoutingDecision::FullLLM));
}

#[test]
fn test_route_action_dispatch() {
    let router = make_router();
    let ctx = context::RouterContext::default();
    let dispatch = ActionDispatch {
        tool: "rss".into(),
        params: serde_json::json!({"action": "accept"}),
        source: ActionSource::Button {
            action_id: "btn".into(),
        },
    };
    let decision = router.route("ignored", &ctx, Some(&dispatch));
    assert!(matches!(
        decision,
        RoutingDecision::DirectDispatch {
            source: DispatchSource::Button,
            ..
        }
    ));
}

#[test]
fn test_route_action_dispatch_cron_source() {
    let router = make_router();
    let ctx = context::RouterContext::default();
    let dispatch = ActionDispatch {
        tool: "cron".into(),
        params: serde_json::json!({"action": "list"}),
        source: ActionSource::Cron {
            job_id: "job-1".into(),
        },
    };
    let decision = router.route("ignored", &ctx, Some(&dispatch));
    assert!(matches!(
        decision,
        RoutingDecision::DirectDispatch {
            source: DispatchSource::Cron,
            ..
        }
    ));
}

#[test]
fn test_route_directive_match() {
    let router = make_router();
    let mut ctx = context::RouterContext::default();
    ctx.set_active_tool(Some("rss".into()));
    ctx.install_directives(vec![context::ActionDirective {
        trigger: context::DirectiveTrigger::OneOf(vec!["yes".into(), "accept".into()]),
        tool: "rss".into(),
        params: serde_json::json!({"action": "accept", "article_ids": ["abc"]}),
        single_use: true,
        ttl_ms: 300_000,
        created_at_ms: now_ms(),
    }]);
    let decision = router.route("yes", &ctx, None);
    assert!(matches!(
        decision,
        RoutingDecision::DirectDispatch {
            source: DispatchSource::ActionDirective,
            ..
        }
    ));
}

#[test]
fn test_route_expired_directive_skipped() {
    let router = make_router();
    let mut ctx = context::RouterContext::default();
    ctx.set_active_tool(Some("rss".into()));
    ctx.install_directives(vec![context::ActionDirective {
        trigger: context::DirectiveTrigger::Exact("yes".into()),
        tool: "rss".into(),
        params: serde_json::json!({}),
        single_use: false,
        ttl_ms: 1,
        created_at_ms: now_ms() - 1000,
    }]);
    let decision = router.route("yes", &ctx, None);
    assert!(matches!(decision, RoutingDecision::FullLLM));
}

#[test]
fn test_route_config_command() {
    let router = make_router();
    let ctx = context::RouterContext::default();
    let decision = router.route("!weather portland", &ctx, None);
    match decision {
        RoutingDecision::DirectDispatch {
            tool,
            params,
            source: DispatchSource::ConfigRule,
            ..
        } => {
            assert_eq!(tool, "weather");
            assert_eq!(params["location"], "portland");
        }
        _ => panic!("expected DirectDispatch ConfigRule"),
    }
}

#[test]
fn test_route_config_command_case_insensitive() {
    let router = make_router();
    let ctx = context::RouterContext::default();
    let decision = router.route("!Weather Portland", &ctx, None);
    match decision {
        RoutingDecision::DirectDispatch {
            tool,
            params,
            source: DispatchSource::ConfigRule,
            ..
        } => {
            assert_eq!(tool, "weather");
            assert_eq!(params["location"], "Portland");
        }
        _ => panic!("expected DirectDispatch ConfigRule"),
    }
}

#[test]
fn test_route_router_replay_command() {
    let router = make_router();
    let ctx = context::RouterContext::default();
    let decision = router.route("!router_replay 2", &ctx, None);
    match decision {
        RoutingDecision::DirectDispatch {
            tool,
            params,
            source: DispatchSource::Command,
            ..
        } => {
            assert_eq!(tool, "_router_replay");
            assert_eq!(params["index"], 2);
        }
        _ => panic!("expected DirectDispatch Command"),
    }
}

#[test]
fn test_route_static_rule_with_context() {
    let router = make_router();
    let mut ctx = context::RouterContext::default();
    ctx.set_active_tool(Some("rss".into()));
    let decision = router.route("next", &ctx, None);
    assert!(matches!(
        decision,
        RoutingDecision::DirectDispatch {
            source: DispatchSource::StaticRule,
            ..
        }
    ));
}

#[test]
fn test_route_static_rule_wrong_context() {
    let router = make_router();
    let mut ctx = context::RouterContext::default();
    ctx.set_active_tool(Some("cron".into()));
    // "next" requires rss context, so it shouldn't match.
    // active_tool is "cron" with no live directives → stale context → FullLLM
    let decision = router.route("next", &ctx, None);
    assert!(matches!(decision, RoutingDecision::FullLLM));
}

#[test]
fn test_route_static_rule_no_context_required() {
    let router = make_router();
    let ctx = context::RouterContext::default();
    let decision = router.route("list jobs", &ctx, None);
    assert!(matches!(
        decision,
        RoutingDecision::DirectDispatch {
            source: DispatchSource::StaticRule,
            ..
        }
    ));
}

#[test]
fn test_route_guided_llm_active_context_with_directives() {
    let router = make_router();
    let mut ctx = context::RouterContext::default();
    ctx.set_active_tool(Some("rss".into()));
    // GuidedLLM only fires when there are live directives
    ctx.install_directives(vec![context::ActionDirective {
        trigger: context::DirectiveTrigger::Exact("yes".into()).normalized(),
        tool: "rss".into(),
        params: serde_json::json!({}),
        single_use: false,
        ttl_ms: 300_000,
        created_at_ms: now_ms(),
    }]);
    let decision = router.route("show me something interesting", &ctx, None);
    match decision {
        RoutingDecision::GuidedLLM { policy } => {
            assert!(policy.allowed_tools.contains(&"rss".to_string()));
            assert_eq!(policy.reason, "active_tool_with_live_directives");
            assert!(policy.context_hint.is_some());
        }
        _ => panic!("expected GuidedLLM"),
    }
}

#[test]
fn test_route_stale_active_tool_falls_to_full_llm() {
    let router = make_router();
    let mut ctx = context::RouterContext::default();
    ctx.set_active_tool(Some("rss".into()));
    // No directives — stale context
    let decision = router.route("show me something interesting", &ctx, None);
    assert!(matches!(decision, RoutingDecision::FullLLM));
}

#[test]
fn test_route_full_llm_no_context() {
    let router = make_router();
    let ctx = context::RouterContext::default();
    let decision = router.route("hello how are you", &ctx, None);
    assert!(matches!(decision, RoutingDecision::FullLLM));
}

#[test]
fn test_route_remember_fast_path() {
    let router = MessageRouter::with_remember_checker(
        vec![],
        vec![],
        "!".into(),
        Some(Box::new(|msg: &str| {
            msg.to_lowercase().contains("remember that")
        })),
    );
    let ctx = context::RouterContext::default();
    let decision = router.route("remember that my favorite color is blue", &ctx, None);
    assert!(matches!(
        decision,
        RoutingDecision::DirectDispatch {
            source: DispatchSource::RememberFastPath,
            ..
        }
    ));
}

#[test]
fn test_route_empty_message() {
    let router = make_router();
    let ctx = context::RouterContext::default();
    let decision = router.route("", &ctx, None);
    assert!(matches!(decision, RoutingDecision::FullLLM));
}

#[test]
fn test_route_directive_returns_index() {
    let router = make_router();
    let mut ctx = context::RouterContext::default();
    ctx.set_active_tool(Some("rss".into()));
    ctx.install_directives(vec![
        context::ActionDirective {
            trigger: context::DirectiveTrigger::Exact("no".into()),
            tool: "rss".into(),
            params: serde_json::json!({}),
            single_use: true,
            ttl_ms: 300_000,
            created_at_ms: now_ms(),
        },
        context::ActionDirective {
            trigger: context::DirectiveTrigger::Exact("yes".into()),
            tool: "rss".into(),
            params: serde_json::json!({}),
            single_use: true,
            ttl_ms: 300_000,
            created_at_ms: now_ms(),
        },
    ]);
    match router.route("yes", &ctx, None) {
        RoutingDecision::DirectDispatch {
            directive_index, ..
        } => assert_eq!(directive_index, Some(1)),
        _ => panic!("expected DirectDispatch"),
    }
    match router.route("maybe", &ctx, None) {
        RoutingDecision::GuidedLLM { .. } => {}
        _ => panic!("expected GuidedLLM"),
    }
}
