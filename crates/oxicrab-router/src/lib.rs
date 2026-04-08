pub mod context;
pub mod metrics;
pub mod rules;
pub mod semantic;

use std::collections::HashMap;

use tracing::{debug, info, trace};

use context::RouterContext;
use oxicrab_core::dispatch::{ActionDispatch, ActionSource};

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
            Self::ConfigRule => "config_rule",
            Self::RememberFastPath => "remember",
            Self::Webhook => "webhook",
            Self::Cron => "cron",
            Self::Command => "command_dispatch",
            Self::ToolChain => "chain",
        }
    }
}

/// Callback type for detecting "remember" fast-path messages.
type RememberChecker = Box<dyn Fn(&str) -> bool + Send + Sync>;

/// Priority-ordered message router.
pub struct MessageRouter {
    static_rules: Vec<rules::StaticRule>,
    config_rules: HashMap<String, rules::ConfigRule>,
    prefix: String,
    static_literal_to_index: HashMap<String, usize>,
    static_pattern_indices: Vec<usize>,
    remember_checker: Option<RememberChecker>,
}

impl MessageRouter {
    pub fn new(
        static_rules: Vec<rules::StaticRule>,
        config_rules: Vec<rules::ConfigRule>,
        prefix: String,
    ) -> Self {
        Self::with_remember_checker(static_rules, config_rules, prefix, None)
    }

    pub fn with_remember_checker(
        static_rules: Vec<rules::StaticRule>,
        config_rules: Vec<rules::ConfigRule>,
        prefix: String,
        remember_checker: Option<RememberChecker>,
    ) -> Self {
        if prefix.is_empty() {
            tracing::warn!(
                "router: empty prefix configured — all messages will match as prefix commands, \
                 falling back to '!'"
            );
        }
        let prefix = if prefix.is_empty() {
            "!".to_string()
        } else {
            prefix
        };
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
        // Lowercase config rule keys so lookup is case-insensitive.
        // Detect conflicts at startup and keep the first definition.
        let mut config_rules_map = HashMap::new();
        for rule in config_rules {
            let key = rule.trigger.to_lowercase();
            if config_rules_map.contains_key(&key) {
                tracing::warn!(
                    "router: config rule conflict for trigger '{}' (keeping first)",
                    key
                );
                continue;
            }
            config_rules_map.insert(key, rule);
        }
        Self {
            static_rules,
            config_rules: config_rules_map,
            prefix,
            static_literal_to_index,
            static_pattern_indices,
            remember_checker,
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
        self.route_with_semantic(message, ctx, action, None)
    }

    /// Route a message with optional semantic tool candidates.
    pub fn route_with_semantic(
        &self,
        message: &str,
        ctx: &RouterContext,
        action: Option<&ActionDispatch>,
        semantic_allowed_tools: Option<Vec<String>>,
    ) -> RoutingDecision {
        trace!(
            message_len = message.len(),
            has_action = action.is_some(),
            active_tool = ctx.active_tool().unwrap_or(""),
            directive_count = ctx.directives().len(),
            semantic_candidate_count = semantic_allowed_tools.as_ref().map_or(0, Vec::len),
            "router: begin route evaluation"
        );
        // 1. Explicit action dispatch (button / webhook / cron).
        trace!(
            matched = action.is_some(),
            "router: priority=1 explicit action dispatch"
        );
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
        trace!(
            is_empty = message.is_empty(),
            "router: priority=2 empty message"
        );
        if message.is_empty() {
            debug!("router: decision=FullLLM");
            metrics::record_full_llm();
            return RoutingDecision::FullLLM;
        }

        // Pre-lowercase once for all directive and rule matching
        let normalized = message.trim().to_lowercase();
        let now = now_ms();

        // 3. ActionDirective match (skip expired).
        trace!("router: priority=3 action directive match");
        if let Some(i) = ctx.match_directive_index(&normalized, now)
            && let Some(directive) = ctx.directives().get(i)
        {
            trace!(
                directive_index = i,
                tool = directive.tool.as_str(),
                "router: matched live action directive"
            );
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

        // 4. Prefix command → ConfigRule.
        trace!(
            starts_with_prefix = message.trim().starts_with(&self.prefix),
            prefix = self.prefix.as_str(),
            "router: priority=4 prefix command"
        );
        if message.trim().starts_with(&self.prefix) {
            let (cmd, args) = rules::parse_prefixed_command(message, &self.prefix);
            let cmd_lower = cmd.to_lowercase();
            trace!(
                command = cmd_lower.as_str(),
                argc = args.len(),
                "router: parsed prefixed command"
            );
            if cmd_lower == "router_replay" || cmd_lower == "route_replay" {
                let index = args.first().and_then(|raw| raw.parse::<i64>().ok());
                info!("router: decision=DirectDispatch tool=_router_replay source=Command");
                metrics::record_direct_dispatch();
                return RoutingDecision::DirectDispatch {
                    tool: "_router_replay".into(),
                    params: serde_json::json!({ "index": index }),
                    source: DispatchSource::Command,
                    directive_index: None,
                };
            }
            if !cmd_lower.is_empty()
                && let Some(rule) = self.config_rules.get(&cmd_lower)
            {
                trace!(
                    command = cmd_lower.as_str(),
                    tool = rule.tool.as_str(),
                    "router: matched config rule"
                );
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
        trace!(
            has_exact_candidate = self.static_literal_to_index.contains_key(&normalized),
            pattern_rule_count = self.static_pattern_indices.len(),
            "router: priority=5 static rule match"
        );
        if let Some(idx) = self.static_literal_to_index.get(&normalized)
            && self
                .static_rules
                .get(*idx)
                .is_some_and(|rule| rule.matches_normalized(&normalized, active_tool))
            && let Some(rule) = self.static_rules.get(*idx)
        {
            trace!(
                rule_index = *idx,
                tool = rule.tool.as_str(),
                "router: matched exact static rule"
            );
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
        for idx in &self.static_pattern_indices {
            let Some(rule) = self.static_rules.get(*idx) else {
                continue;
            };
            if rule.matches_normalized(&normalized, active_tool) {
                trace!(
                    rule_index = *idx,
                    tool = rule.tool.as_str(),
                    "router: matched pattern static rule"
                );
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
        trace!(
            enabled = self.remember_checker.is_some(),
            "router: priority=6 remember fast path"
        );
        if let Some(ref checker) = self.remember_checker
            && checker(message)
        {
            trace!("router: remember fast path matched");
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
        trace!("router: priority=7 active tool context");
        match ctx.state(now) {
            context::RouterState::Focused { tool } => {
                trace!(tool = tool, "router: active tool context is focused");
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
                    trace!(
                        tool = tool,
                        "router: active tool present but context is stale"
                    );
                    // Stale context — all directives expired. Fall through to FullLLM.
                    debug!(
                        "router: active_tool={tool} but no live directives, falling through to FullLLM"
                    );
                }
            }
        }

        // 8. Full LLM.
        trace!(
            semantic_candidate_count = semantic_allowed_tools.as_ref().map_or(0, Vec::len),
            "router: priority=8 semantic filter / full llm"
        );
        if let Some(mut tools) = semantic_allowed_tools {
            tools.sort();
            tools.dedup();
            if tools.len() >= 2 {
                trace!(
                    tool_count = tools.len(),
                    "router: semantic filter selected enough tools"
                );
                info!(
                    "router: decision=SemanticFilter tool_subset=[{}]",
                    tools.join(",")
                );
                metrics::record_semantic_filter();
                return RoutingDecision::SemanticFilter {
                    policy: RoutingPolicy {
                        allowed_tools: tools,
                        blocked_tools: Vec::new(),
                        context_hint: None,
                        reason: "semantic_filter",
                    },
                };
            }
        }
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

pub use oxicrab_core::time::now_ms;
pub use oxicrab_core::tools::base::routing_types::{DirectiveTrigger, StaticRule};

#[cfg(test)]
mod tests;
