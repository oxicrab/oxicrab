//! `query_activity` — natural-language search over the activity
//! journal. Reads the NDJSON file written by `ActivityJournal` and
//! returns records within a time window centred on the resolved anchor.

use crate::agent::activity_journal::{
    ActivityJournal, ActivityWindow, parse_time_expression, query_window, render_records,
};
use async_trait::async_trait;
use chrono::Utc;
use oxicrab_core::actions;
use oxicrab_core::tools::base::{
    ExecutionContext, Tool, ToolCapabilities, ToolCategory, ToolConcurrency, ToolResult,
};
use serde_json::{Value, json};
use std::sync::Arc;

#[cfg(test)]
mod tests;

pub struct QueryActivityTool {
    journal: Arc<ActivityJournal>,
    default_window: u32,
    max_window: u32,
}

impl QueryActivityTool {
    pub fn new(journal: Arc<ActivityJournal>, default_window: u32, max_window: u32) -> Self {
        Self {
            journal,
            default_window: default_window.max(1),
            max_window: max_window.max(1),
        }
    }
}

#[async_trait]
impl Tool for QueryActivityTool {
    fn name(&self) -> &'static str {
        "query_activity"
    }

    fn description(&self) -> &'static str {
        "Search the activity journal for past conversation turns. Accepts a free-form time \
         expression like '2 hours ago', 'this morning', '3pm yesterday', 'a week ago'. \
         Returns records within a window centred on the resolved time. Use this when the \
         user asks 'what did I/we ...' about something not in the current session."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "time_expression": {
                    "type": "string",
                    "description": "Natural-language time anchor (e.g. '30 minutes ago', 'yesterday afternoon')."
                },
                "window_minutes": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Half-window width in minutes; the search covers anchor ± window. Capped server-side."
                },
                "session_only": {
                    "type": "boolean",
                    "description": "When true, restrict to the current session_key (set by the harness)."
                }
            },
            "required": ["time_expression"]
        })
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            actions: actions![query: ro],
            category: ToolCategory::Productivity,
            concurrency: ToolConcurrency::ReadOnly,
            ..Default::default()
        }
    }

    async fn execute(&self, params: Value, ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
        let expr = match params.get("time_expression").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s,
            _ => {
                return Ok(ToolResult::error(
                    "query_activity: missing 'time_expression'".to_string(),
                ));
            }
        };
        let window = params
            .get("window_minutes")
            .and_then(serde_json::Value::as_u64)
            .map_or(self.default_window, |n| {
                (n as u32).clamp(1, self.max_window)
            });
        let session_only = params
            .get("session_only")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        let resolved = match parse_time_expression(expr, Utc::now()) {
            Some(r) => r,
            None => {
                return Ok(ToolResult::error(format!(
                    "query_activity: could not parse time expression '{expr}'. \
                     Try '30 minutes ago', '2pm yesterday', 'this morning', or '3 days ago'."
                )));
            }
        };

        let records = match self.journal.read_all() {
            Ok(r) => r,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "query_activity: failed to read journal: {e}"
                )));
            }
        };

        let session_filter = if session_only {
            ctx.metadata
                .get("session_key")
                .and_then(serde_json::Value::as_str)
        } else {
            None
        };

        let win = ActivityWindow {
            anchor: resolved.anchor,
            half_window_minutes: window,
        };
        let hits = query_window(&records, &win, session_filter);
        let body = render_records(&hits);
        Ok(ToolResult::new(format!(
            "Resolved '{expr}' to {} (± {} min).\n\n{body}",
            resolved.resolution, window
        )))
    }
}
