mod common;

use common::{MockLLMProvider, TestAgentOverrides, create_test_agent_with, text_response};
use oxicrab::agent::AgentRunOverrides;
use oxicrab::dispatch::{ActionDispatch, ActionSource};
use oxicrab::router::context::{ActionDirective, DirectiveTrigger, RouterContext};
use oxicrab::router::rules::{ConfigRule, StaticRule};
use oxicrab::router::{MessageRouter, RoutingDecision};
use serde_json::json;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// MessageRouter — priority ordering (tests the assembled router, not individual
// components which are covered by unit tests in src/router/)
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
            ..
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
    // "next" requires rss context, cron context → no static rule match.
    // active_tool is set but no live directives → stale context → FullLLM
    let decision = router.route("next", &ctx, None);
    assert!(matches!(decision, RoutingDecision::FullLLM));
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
fn test_router_guided_llm_with_active_tool_and_directives() {
    let router = make_router();
    let now = oxicrab::router::now_ms();
    let mut ctx = RouterContext {
        active_tool: Some("rss".into()),
        ..Default::default()
    };
    // GuidedLLM only fires when active_tool has live directives
    ctx.action_directives.push(ActionDirective {
        trigger: DirectiveTrigger::Exact("yes".into()).normalized(),
        tool: "rss".into(),
        params: json!({}),
        single_use: false,
        ttl_ms: 300_000,
        created_at_ms: now,
    });
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
fn test_route_directive_returns_correct_index() {
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
    match router.route("yes", &ctx, None) {
        RoutingDecision::DirectDispatch {
            directive_index, ..
        } => assert_eq!(directive_index, Some(1)),
        other => panic!("expected DirectDispatch, got {other:?}"),
    }
    match router.route("no", &ctx, None) {
        RoutingDecision::DirectDispatch {
            directive_index, ..
        } => assert_eq!(directive_index, Some(0)),
        other => panic!("expected DirectDispatch, got {other:?}"),
    }
    match router.route("maybe", &ctx, None) {
        RoutingDecision::FullLLM => {}
        other => panic!("expected FullLLM, got {other:?}"),
    }
}

#[test]
fn test_route_expired_directive_returns_no_index() {
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
    // Expired directive should not match — falls through to FullLLM
    match router.route("yes", &ctx, None) {
        RoutingDecision::FullLLM => {}
        other => panic!("expected FullLLM, got {other:?}"),
    }
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
