use super::AgentLoop;
use super::config::AgentRunOverrides;
use anyhow::Result;
use serde_json::Value;

impl AgentLoop {
    pub(super) fn build_router_replay_metadata(
        content: &str,
        router_context: &crate::router::context::RouterContext,
        decision: &crate::router::RoutingDecision,
        overrides: &AgentRunOverrides,
    ) -> Value {
        let now = crate::router::now_ms();
        let (context_state, active_tool) = match router_context.state(now) {
            crate::router::context::RouterState::Idle => ("idle", None),
            crate::router::context::RouterState::Focused { tool } => ("tool_focused", Some(tool)),
        };
        let decision_kind = match decision {
            crate::router::RoutingDecision::DirectDispatch { .. } => "direct_dispatch",
            crate::router::RoutingDecision::GuidedLLM { .. } => "guided_llm",
            crate::router::RoutingDecision::SemanticFilter { .. } => "semantic_filter",
            crate::router::RoutingDecision::FullLLM => "full_llm",
        };
        serde_json::json!({
            "ts_ms": now,
            "message_normalized": content.trim().to_lowercase(),
            "decision": decision_kind,
            "context_state": context_state,
            "active_tool": active_tool,
            "live_directive_count": router_context.directives().iter().filter(|d| !d.is_expired(now)).count(),
            "policy_reason": overrides.routing_policy.as_ref().map(|p| p.reason),
            "policy_allowed_tools": overrides.routing_policy.as_ref().map(|p| p.allowed_tools.clone()).unwrap_or_default(),
            "policy_blocked_tools": overrides.routing_policy.as_ref().map(|p| p.blocked_tools.clone()).unwrap_or_default(),
        })
    }

    pub(super) async fn render_router_replay(
        &self,
        session_key: &str,
        index: Option<i64>,
    ) -> Result<String> {
        let session = self.sessions.get_or_create(session_key).await?;
        let entries: Vec<(usize, &crate::session::manager::MessageData, &Value)> = session
            .messages
            .iter()
            .enumerate()
            .filter_map(|(i, msg)| {
                msg.extra
                    .get("router_replay")
                    .map(|trace| (i, msg, trace))
                    .filter(|(_, msg, _)| msg.role == "user")
            })
            .collect();

        if entries.is_empty() {
            return Ok("No router replay traces are available in this session yet.".to_string());
        }

        let selected = if let Some(i) = index {
            if i < 0 {
                entries.last().copied()
            } else {
                entries.get(i as usize).copied()
            }
        } else {
            entries.last().copied()
        };
        let Some((message_idx, message, trace)) = selected else {
            return Ok(format!(
                "Router replay index out of range. Available traces: 0..{}.",
                entries.len().saturating_sub(1)
            ));
        };

        let trace_pos = entries
            .iter()
            .position(|(i, _, _)| *i == message_idx)
            .unwrap_or(0);
        let pretty_trace =
            serde_json::to_string_pretty(trace).unwrap_or_else(|_| trace.to_string());
        Ok(format!(
            "Router replay trace {trace_pos} of {} (session message #{message_idx}).\n\
             User message: {}\n\
             Trace:\n```json\n{}\n```",
            entries.len().saturating_sub(1),
            message.content,
            pretty_trace
        ))
    }
}
