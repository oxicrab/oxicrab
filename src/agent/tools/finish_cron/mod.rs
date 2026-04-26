//! `finish_cron` — explicit completion signal for cron-driven agent runs.
//!
//! ## Why
//!
//! Without an explicit terminal action, cron job execution ends when
//! the LLM stops calling tools — a *heuristic*. That fails the moment
//! a hallucination class drives the model to produce text claiming
//! success after a tool returned `is_error=true`: the trace completes
//! successfully and the operator never learns the work didn't happen.
//!
//! `finish_cron` flips the contract: the LLM signals completion by
//! calling this tool with a structured summary and a `success` flag.
//! The cron callback reads the call's metadata sideband and uses it
//! verbatim for the trace's `Completed` / `Failed` event.
//!
//! In non-cron contexts the tool is a harmless annotation — it just
//! returns the summary as the result content. Setting the metadata
//! anyway lets agent-loop callers (or future channels) consume the
//! same signal without code changes.

use async_trait::async_trait;
use oxicrab_core::actions;
use oxicrab_core::tools::base::{
    ExecutionContext, Tool, ToolCapabilities, ToolCategory, ToolConcurrency, ToolResult,
};
use serde_json::{Value, json};
use std::collections::HashMap;

/// Metadata key on `ToolResult` carrying the structured completion
/// payload `{summary, success, reason?}`. Read by the cron callback
/// in `gateway_setup.rs` to write the trace `Completed` event.
pub const FINISH_CRON_META: &str = "__cron_finish";

/// Metadata key on `OutboundMessage` indicating the agent run is the
/// terminal turn for a cron job. Currently set by `iteration.rs` when
/// it sees `FINISH_CRON_META` in `collected_tool_metadata` so callers
/// don't have to re-walk the metadata.
pub const FINISH_CRON_TERMINAL: &str = "__cron_terminal";

pub struct FinishCronTool;

impl FinishCronTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for FinishCronTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for FinishCronTool {
    fn name(&self) -> &'static str {
        "finish_cron"
    }

    fn description(&self) -> &'static str {
        "Signal that a cron job is finished. Call this when the scheduled task is complete \
         (or has failed unrecoverably). Pass a one-paragraph `summary` of what actually \
         happened, plus `success: false` when the work didn't complete. The cron trace will \
         use your summary verbatim. Do NOT call this tool in interactive conversations — \
         it's only meaningful for scheduled jobs."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "summary": {
                    "type": "string",
                    "description": "What happened in this cron run. Will be persisted to the trace as-is. Keep under 500 chars.",
                    "maxLength": 1000
                },
                "success": {
                    "type": "boolean",
                    "description": "True (default) when the work completed; false when something went wrong and a follow-up is needed."
                },
                "reason": {
                    "type": "string",
                    "description": "Optional short reason field — required when success=false. Surfaced in trace events for the operator."
                }
            },
            "required": ["summary"]
        })
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            actions: actions![finish: ro],
            category: ToolCategory::Scheduling,
            concurrency: ToolConcurrency::ReadOnly,
            ..Default::default()
        }
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
        let summary = params
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if summary.is_empty() {
            return Ok(ToolResult::error(
                "finish_cron: 'summary' is required and must be non-empty".to_string(),
            ));
        }
        let success = params
            .get("success")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        let reason = params
            .get("reason")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        if !success && reason.as_deref().is_none_or(str::is_empty) {
            return Ok(ToolResult::error(
                "finish_cron: 'reason' is required when success=false".to_string(),
            ));
        }

        // Cap summary at 1KB defensively — it lands in the trace as-is.
        let summary = if summary.chars().count() > 1_000 {
            let mut end = 1_000;
            while !summary.is_char_boundary(end) {
                end = end.saturating_sub(1);
            }
            format!("{}…", &summary[..end])
        } else {
            summary.to_string()
        };

        let mut metadata: HashMap<String, Value> = HashMap::new();
        metadata.insert(
            FINISH_CRON_META.to_string(),
            json!({
                "summary": summary,
                "success": success,
                "reason": reason,
            }),
        );

        let body = if success {
            format!("Marked complete: {summary}")
        } else {
            format!(
                "Marked failed: {} — reason: {}",
                summary,
                reason.as_deref().unwrap_or("(none)")
            )
        };
        Ok(ToolResult::new(body).with_metadata(metadata))
    }
}

#[cfg(test)]
mod tests;
