pub mod context;
pub mod metrics;
pub mod rules;
pub mod semantic;

use std::collections::HashMap;

use tracing::{debug, info};

use crate::dispatch::{ActionDispatch, ActionSource};
use context::RouterContext;

/// The routing decision produced by `MessageRouter::route()`.
#[derive(Debug)]
pub enum RoutingDecision {
    /// Bypass the LLM entirely — call this tool with these params.
    DirectDispatch {
        tool: String,
        params: serde_json::Value,
        source: DispatchSource,
        directive_index: Option<usize>,
    },
    /// Send to LLM, but constrain available tools and prepend a context hint.
    GuidedLLM { policy: RoutingPolicy },
    /// LLM interprets with semantically filtered tools.
    /// NOTE: Not yet wired — router returns `FullLLM`; agent loop may construct this
    /// in the future for embedding-based tool selection.
    SemanticFilter { policy: RoutingPolicy },
    /// Full unconstrained LLM turn.
    FullLLM,
}

/// Policy payload for constrained LLM turns.
#[derive(Debug, Clone)]
pub struct RoutingPolicy {
    /// Exact tool allow-list for this turn.
    pub allowed_tools: Vec<String>,
    /// Explicit block-list for this turn (for observability and strict policy).
    pub blocked_tools: Vec<String>,
    /// Optional prompt hint to inject into the system prompt.
    pub context_hint: Option<String>,
    /// Human-readable route reason for logs and analytics.
    pub reason: &'static str,
}

/// Identifies how a `DirectDispatch` decision was produced.
#[derive(Debug)]
pub enum DispatchSource {
    Button,
    ActionDirective,
    StaticRule,
    ConfigRule,
    RememberFastPath,
    Webhook,
    Cron,
    Command,
    ToolChain,
}

impl DispatchSource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Button => "button",
            Self::ActionDirective => "directive",
            Self::StaticRule => "rule",
            Self::ConfigRule => "command",
            Self::RememberFastPath => "remember",
            Self::Webhook => "webhook",
            Self::Cron => "cron",
            Self::Command => "command_dispatch",
            Self::ToolChain => "chain",
        }
    }
}

/// Priority-ordered message router.
pub struct MessageRouter {
    static_rules: Vec<rules::StaticRule>,
    config_rules: HashMap<String, rules::ConfigRule>,
    prefix: String,
    static_literal_to_index: HashMap<String, usize>,
    static_pattern_indices: Vec<usize>,
}

impl MessageRouter {
    pub fn new(
        static_rules: Vec<rules::StaticRule>,
        config_rules: HashMap<String, rules::ConfigRule>,
        prefix: String,
    ) -> Self {
        let static_rules: Vec<rules::StaticRule> = static_rules
            .into_iter()
            .map(|mut r| {
                r.trigger = r.trigger.normalized();
                r
            })
            .collect();
        let mut static_literal_to_index = HashMap::new();
        let mut static_pattern_indices = Vec::new();
        for (idx, rule) in static_rules.iter().enumerate() {
            match &rule.trigger {
                context::DirectiveTrigger::Exact(s) => {
                    if static_literal_to_index.contains_key(s) {
                        tracing::warn!(
                            "router: static rule literal conflict for '{}' (keeping first)",
                            s
                        );
                    } else {
                        static_literal_to_index.insert(s.clone(), idx);
                    }
                }
                context::DirectiveTrigger::OneOf(options) => {
                    for opt in options {
                        if static_literal_to_index.contains_key(opt) {
                            tracing::warn!(
                                "router: static rule literal conflict for '{}' (keeping first)",
                                opt
                            );
                        } else {
                            static_literal_to_index.insert(opt.clone(), idx);
                        }
                    }
                }
                context::DirectiveTrigger::Pattern(_) => static_pattern_indices.push(idx),
            }
        }
        // Lowercase config rule keys so lookup is case-insensitive
        let config_rules = config_rules
            .into_iter()
            .map(|(k, v)| (k.to_lowercase(), v))
            .collect();
        Self {
            static_rules,
            config_rules,
            prefix,
            static_literal_to_index,
            static_pattern_indices,
        }
    }

    /// Route a message. Checks in priority order:
    ///
    /// 1. Explicit `ActionDispatch` (button / webhook / cron)
    /// 2. Empty message → `FullLLM`
    /// 3. Live `ActionDirective` match
    /// 4. Prefix command → `ConfigRule`
    /// 5. `StaticRule` match
    /// 6. Remember fast path
    /// 7. Active tool context → `GuidedLLM`
    /// 8. `FullLLM`
    pub fn route(
        &self,
        message: &str,
        ctx: &RouterContext,
        action: Option<&ActionDispatch>,
    ) -> RoutingDecision {
        // 1. Explicit action dispatch (button / webhook / cron).
        if let Some(dispatch) = action {
            let source = action_source_to_dispatch_source(&dispatch.source);
            let source_label = dispatch.source.label();
            info!(
                "router: decision=DirectDispatch tool={} source={source_label}",
                dispatch.tool
            );
            metrics::record_direct_dispatch();
            return RoutingDecision::DirectDispatch {
                tool: dispatch.tool.clone(),
                params: dispatch.params.clone(),
                source,
                directive_index: None,
            };
        }

        // 2. Empty message.
        if message.is_empty() {
            debug!("router: decision=FullLLM");
            metrics::record_full_llm();
            return RoutingDecision::FullLLM;
        }

        // Pre-lowercase once for all directive and rule matching
        let normalized = message.trim().to_lowercase();
        let now = now_ms();

        // 3. ActionDirective match (skip expired).
        if let Some(i) = ctx.match_directive_index(&normalized, now) {
            if let Some(directive) = ctx.directives().get(i) {
                info!(
                    "router: decision=DirectDispatch tool={} source=ActionDirective",
                    directive.tool
                );
                metrics::record_direct_dispatch();
                return RoutingDecision::DirectDispatch {
                    tool: directive.tool.clone(),
                    params: directive.params.clone(),
                    source: DispatchSource::ActionDirective,
                    directive_index: Some(i),
                };
            }
        }

        // 4. Prefix command → ConfigRule.
        if message.trim().starts_with(&self.prefix) {
            let (cmd, args) = rules::parse_prefixed_command(message, &self.prefix);
            let cmd_lower = cmd.to_lowercase();
            if !cmd_lower.is_empty()
                && let Some(rule) = self.config_rules.get(&cmd_lower)
            {
                let params = rule.substitute(&args);
                info!(
                    "router: decision=DirectDispatch tool={} source=ConfigRule",
                    rule.tool
                );
                metrics::record_direct_dispatch();
                return RoutingDecision::DirectDispatch {
                    tool: rule.tool.clone(),
                    params,
                    source: DispatchSource::ConfigRule,
                    directive_index: None,
                };
            }
        }

        // 5. StaticRule match.
        let active_tool = ctx.active_tool();
        if let Some(idx) = self.static_literal_to_index.get(&normalized)
            && self
                .static_rules
                .get(*idx)
                .is_some_and(|rule| rule.matches_normalized(&normalized, active_tool))
        {
            if let Some(rule) = self.static_rules.get(*idx) {
                info!(
                    "router: decision=DirectDispatch tool={} source=StaticRule",
                    rule.tool
                );
                metrics::record_direct_dispatch();
                return RoutingDecision::DirectDispatch {
                    tool: rule.tool.clone(),
                    params: rule.params.clone(),
                    source: DispatchSource::StaticRule,
                    directive_index: None,
                };
            }
        }
        for idx in &self.static_pattern_indices {
            let Some(rule) = self.static_rules.get(*idx) else {
                continue;
            };
            if rule.matches_normalized(&normalized, active_tool) {
                info!(
                    "router: decision=DirectDispatch tool={} source=StaticRule",
                    rule.tool
                );
                metrics::record_direct_dispatch();
                return RoutingDecision::DirectDispatch {
                    tool: rule.tool.clone(),
                    params: rule.params.clone(),
                    source: DispatchSource::StaticRule,
                    directive_index: None,
                };
            }
        }

        // 6. Remember fast path.
        if crate::agent::memory::remember::extract_remember_content(message).is_some() {
            info!("router: decision=DirectDispatch tool=_remember source=RememberFastPath");
            metrics::record_direct_dispatch();
            return RoutingDecision::DirectDispatch {
                tool: "_remember".into(),
                params: serde_json::json!({"content": message}),
                source: DispatchSource::RememberFastPath,
                directive_index: None,
            };
        }

        // 7. Active tool context → GuidedLLM.
        // Only route to GuidedLLM when context is fresh — active_tool with live
        // (non-expired) directives indicates an ongoing interaction. Stale context
        // (no live directives, or updated_at too old) falls through to FullLLM
        // to avoid biasing the LLM away from the tools the user actually needs.
        match ctx.state(now) {
            context::RouterState::Focused { tool } => {
                let context_hint = build_context_hint(ctx);
                info!("router: decision=GuidedLLM tool_subset=[{tool}]");
                metrics::record_guided_llm();
                return RoutingDecision::GuidedLLM {
                    policy: RoutingPolicy {
                        allowed_tools: vec![tool.to_string()],
                        blocked_tools: Vec::new(),
                        context_hint: Some(context_hint),
                        reason: "active_tool_with_live_directives",
                    },
                };
            }
            context::RouterState::Idle => {
                if let Some(tool) = ctx.active_tool() {
                    // Stale context — all directives expired. Fall through to FullLLM.
                    debug!(
                        "router: active_tool={tool} but no live directives, falling through to FullLLM"
                    );
                }
            }
        }

        // 8. Full LLM.
        debug!("router: decision=FullLLM");
        metrics::record_full_llm();
        RoutingDecision::FullLLM
    }
}

fn action_source_to_dispatch_source(source: &ActionSource) -> DispatchSource {
    match source {
        ActionSource::Button { .. } => DispatchSource::Button,
        ActionSource::Webhook { .. } => DispatchSource::Webhook,
        ActionSource::Cron { .. } => DispatchSource::Cron,
        ActionSource::Command { .. } => DispatchSource::Command,
        ActionSource::ToolChain { .. } => DispatchSource::ToolChain,
    }
}

fn build_context_hint(ctx: &RouterContext) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(tool) = ctx.active_tool() {
        parts.push(format!("Active tool: {tool}"));
    }

    let now = now_ms();
    let keywords: Vec<String> = ctx
        .directives()
        .iter()
        .filter(|d| !d.is_expired(now))
        .filter_map(|d| match &d.trigger {
            context::DirectiveTrigger::Exact(s) => Some(s.clone()),
            context::DirectiveTrigger::OneOf(opts) => Some(opts.join("|")),
            context::DirectiveTrigger::Pattern(_) => None,
        })
        .collect();

    if !keywords.is_empty() {
        parts.push(format!("Available commands: {}", keywords.join(", ")));
    }

    parts.join(". ")
}

pub use crate::utils::time::now_ms;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::{ActionDispatch, ActionSource};

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
        let mut config_rules = std::collections::HashMap::new();
        config_rules.insert(
            "weather".into(),
            rules::ConfigRule {
                trigger: "weather".into(),
                tool: "weather".into(),
                params: serde_json::json!({"location": "$1"}),
            },
        );
        MessageRouter::new(static_rules, config_rules, "!".into())
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
        let router = make_router();
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
}
