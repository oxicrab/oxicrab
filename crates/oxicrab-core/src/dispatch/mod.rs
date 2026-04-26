use serde::{Deserialize, Serialize};

/// Structured tool call that bypasses the LLM.
#[derive(Debug, Clone)]
pub struct ActionDispatch {
    pub tool: String,
    pub params: serde_json::Value,
    pub source: ActionSource,
}

#[derive(Debug, Clone)]
pub enum ActionSource {
    Button {
        action_id: String,
    },
    Webhook {
        webhook_name: String,
    },
    Cron {
        job_id: String,
    },
    Command {
        raw: String,
    },
    ToolChain {
        parent_tool: String,
    },
    /// Live `ActionDirective` from the router's `RouterContext` — a
    /// previous tool execution stashed a directive that matched the
    /// current message. Mirrors `DispatchSource::ActionDirective`
    /// in the router so direct-dispatch telemetry has a parallel
    /// label across both layers.
    Directive {
        directive_index: usize,
    },
}

impl ActionSource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Button { .. } => "button",
            Self::Webhook { .. } => "webhook",
            Self::Cron { .. } => "cron",
            Self::Command { .. } => "command",
            Self::ToolChain { .. } => "chain",
            Self::Directive { .. } => "directive",
        }
    }
}

/// Serialized payload in ButtonSpec.context and webhook dispatch configs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionDispatchPayload {
    pub tool: String,
    pub params: serde_json::Value,
}
