use super::*;
use oxicrab_core::time::now_ms;

#[test]
fn test_router_context_default() {
    let ctx = RouterContext::default();
    assert!(ctx.active_tool().is_none());
    assert!(ctx.directives().is_empty());
}

#[test]
fn test_router_context_state_machine_focus_and_idle() {
    let mut ctx = RouterContext::default();
    ctx.set_active_tool(Some("rss".into()));
    assert!(matches!(ctx.state(now_ms()), RouterState::Idle));
    ctx.install_directives(vec![ActionDirective {
        trigger: DirectiveTrigger::Exact("next".into()),
        tool: "rss".into(),
        params: serde_json::json!({}),
        single_use: false,
        ttl_ms: 300_000,
        created_at_ms: now_ms(),
    }]);
    assert!(matches!(ctx.state(now_ms()), RouterState::Focused { .. }));
    ctx.set_idle();
    assert!(matches!(ctx.state(now_ms()), RouterState::Idle));
}

#[test]
fn test_match_directive_index_uses_compiled_literal_map() {
    let mut ctx = RouterContext::default();
    ctx.set_active_tool(Some("rss".into()));
    ctx.install_directives(vec![ActionDirective {
        trigger: DirectiveTrigger::OneOf(vec!["yes".into(), "accept".into()]),
        tool: "rss".into(),
        params: serde_json::json!({}),
        single_use: false,
        ttl_ms: 300_000,
        created_at_ms: now_ms(),
    }]);
    assert_eq!(ctx.match_directive_index("yes", now_ms()), Some(0));
    assert_eq!(ctx.match_directive_index("accept", now_ms()), Some(0));
    assert_eq!(ctx.match_directive_index("no", now_ms()), None);
}

#[test]
fn test_prune_expired_transitions_to_idle() {
    let mut ctx = RouterContext::default();
    ctx.set_active_tool(Some("rss".into()));
    ctx.install_directives(vec![ActionDirective {
        trigger: DirectiveTrigger::Exact("old".into()),
        tool: "rss".into(),
        params: serde_json::json!({}),
        single_use: false,
        ttl_ms: 1,
        created_at_ms: now_ms() - 1000,
    }]);
    ctx.prune_expired(now_ms());
    assert!(matches!(ctx.state(now_ms()), RouterState::Idle));
    assert!(ctx.directives().is_empty());
}

#[test]
fn test_context_serde_roundtrip() {
    let mut ctx = RouterContext::default();
    ctx.set_active_tool(Some("rss".into()));
    ctx.install_directives(vec![ActionDirective {
        trigger: DirectiveTrigger::Exact("next".into()),
        tool: "rss".into(),
        params: serde_json::json!({"action": "next"}),
        single_use: false,
        ttl_ms: 300_000,
        created_at_ms: now_ms(),
    }]);
    ctx.updated_at_ms = now_ms();

    let json = serde_json::to_string(&ctx).unwrap();
    let restored: RouterContext = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.active_tool(), Some("rss"));
    assert_eq!(restored.directives().len(), 1);
}

#[test]
fn test_from_session_metadata_rebuilds_matcher() {
    let mut ctx = RouterContext::default();
    ctx.set_active_tool(Some("rss".into()));
    let ts = now_ms();
    ctx.install_directives(vec![ActionDirective {
        trigger: DirectiveTrigger::Exact("next".into()),
        tool: "rss".into(),
        params: serde_json::json!({"action": "next"}),
        single_use: false,
        ttl_ms: 300_000,
        created_at_ms: ts,
    }]);

    // Serialize to session metadata and restore — matcher must be rebuilt.
    let mut metadata = std::collections::HashMap::new();
    ctx.to_session_metadata(&mut metadata);
    let restored = RouterContext::from_session_metadata(&metadata);
    assert_eq!(
        restored.match_directive_index("next", ts),
        Some(0),
        "from_session_metadata must rebuild the matcher so directives are matchable"
    );
}

#[test]
fn test_raw_deserialized_matcher_is_empty() {
    let mut ctx = RouterContext::default();
    ctx.set_active_tool(Some("rss".into()));
    ctx.install_directives(vec![ActionDirective {
        trigger: DirectiveTrigger::Exact("next".into()),
        tool: "rss".into(),
        params: serde_json::json!({"action": "next"}),
        single_use: false,
        ttl_ms: 300_000,
        created_at_ms: now_ms(),
    }]);

    // Raw serde deserialization skips the matcher — match_directive_index
    // returns None. This is why from_session_metadata() must be used.
    let json = serde_json::to_string(&ctx).unwrap();
    let raw: RouterContext = serde_json::from_str(&json).unwrap();
    assert!(
        raw.matcher.literal_to_index.is_empty(),
        "raw deserialization should leave matcher empty"
    );
}
