pub mod context;
pub mod rules;

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
    GuidedLLM {
        tool_subset: Vec<String>,
        context_hint: String,
    },
    /// LLM interprets with semantically filtered tools.
    /// NOTE: Not yet wired — router returns `FullLLM`; agent loop may construct this
    /// in the future for embedding-based tool selection.
    SemanticFilter { tool_subset: Vec<String> },
    /// Full unconstrained LLM turn.
    FullLLM,
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
        }
    }
}

/// Priority-ordered message router.
pub struct MessageRouter {
    static_rules: Vec<rules::StaticRule>,
    config_rules: HashMap<String, rules::ConfigRule>,
    prefix: String,
}

impl MessageRouter {
    pub fn new(
        static_rules: Vec<rules::StaticRule>,
        config_rules: HashMap<String, rules::ConfigRule>,
        prefix: String,
    ) -> Self {
        let static_rules = static_rules
            .into_iter()
            .map(|mut r| {
                r.trigger = r.trigger.normalized();
                r
            })
            .collect();
        // Lowercase config rule keys so lookup is case-insensitive
        let config_rules = config_rules
            .into_iter()
            .map(|(k, v)| (k.to_lowercase(), v))
            .collect();
        Self {
            static_rules,
            config_rules,
            prefix,
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
            return RoutingDecision::FullLLM;
        }

        // Pre-lowercase once for all directive and rule matching
        let normalized = message.trim().to_lowercase();
        let now = now_ms();

        // 3. ActionDirective match (skip expired).
        for (i, directive) in ctx.action_directives.iter().enumerate() {
            if directive.is_expired(now) {
                continue;
            }
            if directive.trigger.matches_normalized(&normalized) {
                info!(
                    "router: decision=DirectDispatch tool={} source=ActionDirective",
                    directive.tool
                );
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
                return RoutingDecision::DirectDispatch {
                    tool: rule.tool.clone(),
                    params,
                    source: DispatchSource::ConfigRule,
                    directive_index: None,
                };
            }
        }

        // 5. StaticRule match.
        let active_tool = ctx.active_tool.as_deref();
        for rule in &self.static_rules {
            if rule.matches_normalized(&normalized, active_tool) {
                info!(
                    "router: decision=DirectDispatch tool={} source=StaticRule",
                    rule.tool
                );
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
            return RoutingDecision::DirectDispatch {
                tool: "_remember".into(),
                params: serde_json::json!({"content": message}),
                source: DispatchSource::RememberFastPath,
                directive_index: None,
            };
        }

        // 7. Active tool context → GuidedLLM.
        if let Some(tool) = &ctx.active_tool {
            let context_hint = build_context_hint(ctx);
            info!("router: decision=GuidedLLM tool_subset=[{tool}]");
            return RoutingDecision::GuidedLLM {
                tool_subset: vec![tool.clone()],
                context_hint,
            };
        }

        // 8. Full LLM.
        debug!("router: decision=FullLLM");
        RoutingDecision::FullLLM
    }
}

fn action_source_to_dispatch_source(source: &ActionSource) -> DispatchSource {
    match source {
        ActionSource::Button { .. } => DispatchSource::Button,
        ActionSource::Webhook { .. } => DispatchSource::Webhook,
        // NOTE: Cron, Command, and ToolChain are collapsed to ActionDirective
        // because DispatchSource has no dedicated variants for them yet.
        // This is acceptable because handle_direct_dispatch uses
        // ActionSource::label() (not DispatchSource::label()) for session
        // history and logging, preserving the correct source identity.
        ActionSource::Cron { .. }
        | ActionSource::Command { .. }
        | ActionSource::ToolChain { .. } => DispatchSource::ActionDirective,
    }
}

fn build_context_hint(ctx: &RouterContext) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(tool) = &ctx.active_tool {
        parts.push(format!("Active tool: {tool}"));
    }

    let now = now_ms();
    let keywords: Vec<String> = ctx
        .action_directives
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
    fn test_route_directive_match() {
        let router = make_router();
        let mut ctx = context::RouterContext {
            active_tool: Some("rss".into()),
            ..Default::default()
        };
        ctx.action_directives.push(context::ActionDirective {
            trigger: context::DirectiveTrigger::OneOf(vec!["yes".into(), "accept".into()]),
            tool: "rss".into(),
            params: serde_json::json!({"action": "accept", "article_ids": ["abc"]}),
            single_use: true,
            ttl_ms: 300_000,
            created_at_ms: now_ms(),
        });
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
        ctx.action_directives.push(context::ActionDirective {
            trigger: context::DirectiveTrigger::Exact("yes".into()),
            tool: "rss".into(),
            params: serde_json::json!({}),
            single_use: false,
            ttl_ms: 1,
            created_at_ms: now_ms() - 1000,
        });
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
        let ctx = context::RouterContext {
            active_tool: Some("rss".into()),
            ..Default::default()
        };
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
        let ctx = context::RouterContext {
            active_tool: Some("cron".into()),
            ..Default::default()
        };
        // "next" requires rss context, so it shouldn't match.
        // But "list jobs" is context-free, so it also shouldn't match "next".
        let decision = router.route("next", &ctx, None);
        // Should fall through to GuidedLLM (active_tool is set to cron)
        assert!(matches!(decision, RoutingDecision::GuidedLLM { .. }));
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
    fn test_route_guided_llm_active_context_no_match() {
        let router = make_router();
        let ctx = context::RouterContext {
            active_tool: Some("rss".into()),
            ..Default::default()
        };
        let decision = router.route("show me something interesting", &ctx, None);
        assert!(matches!(decision, RoutingDecision::GuidedLLM { .. }));
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
        ctx.action_directives.push(context::ActionDirective {
            trigger: context::DirectiveTrigger::Exact("no".into()),
            tool: "rss".into(),
            params: serde_json::json!({}),
            single_use: true,
            ttl_ms: 300_000,
            created_at_ms: now_ms(),
        });
        ctx.action_directives.push(context::ActionDirective {
            trigger: context::DirectiveTrigger::Exact("yes".into()),
            tool: "rss".into(),
            params: serde_json::json!({}),
            single_use: true,
            ttl_ms: 300_000,
            created_at_ms: now_ms(),
        });
        match router.route("yes", &ctx, None) {
            RoutingDecision::DirectDispatch {
                directive_index, ..
            } => assert_eq!(directive_index, Some(1)),
            _ => panic!("expected DirectDispatch"),
        }
        match router.route("maybe", &ctx, None) {
            RoutingDecision::FullLLM => {}
            _ => panic!("expected FullLLM"),
        }
    }
}
