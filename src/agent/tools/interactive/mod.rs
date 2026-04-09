use crate::actions;
use crate::agent::tools::base::{
    ExecutionContext, ToolCapabilities, ToolCategory, ToolConcurrency,
};
use crate::agent::tools::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const REQUEST_ID_META_KEY: &str = "request_id";

/// A button specification for interactive messages.
#[derive(Debug, Clone)]
pub struct ButtonSpec {
    pub id: String,
    pub label: String,
    pub style: String,
    /// Optional context data returned when the button is clicked.
    /// On Slack, carried via the button `value` field (max 2000 chars).
    pub context: Option<String>,
}

/// Request-scoped pending buttons. The `add_buttons` tool writes here; the
/// agent loop reads and clears them after the matching run completes.
#[derive(Clone, Default)]
pub struct PendingButtons {
    inner: Arc<Mutex<HashMap<String, Vec<ButtonSpec>>>>,
}

impl PendingButtons {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn store(&self, request_id: &str, buttons: Vec<ButtonSpec>) {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(request_id.to_string(), buttons);
    }

    pub fn take(&self, request_id: &str) -> Option<Vec<ButtonSpec>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(request_id)
    }

    pub fn clear(&self, request_id: &str) {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(request_id);
    }
}

pub fn new_pending_buttons() -> PendingButtons {
    PendingButtons::new()
}

/// Tool that lets the LLM attach interactive buttons to its next response.
pub struct AddButtonsTool {
    pending: PendingButtons,
}

impl AddButtonsTool {
    pub fn new(pending: PendingButtons) -> Self {
        Self { pending }
    }
}

#[async_trait]
impl Tool for AddButtonsTool {
    fn name(&self) -> &'static str {
        "add_buttons"
    }

    fn description(&self) -> &'static str {
        "Attach interactive buttons to your next response message. Users can click these buttons \
         to trigger actions. Each button has an id (returned as [button:id] when clicked), \
         a label (displayed text), an optional style, and optional context (returned alongside \
         the id when clicked — use this to carry structured data like task IDs so you can take \
         action without needing to look them up again)."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "buttons": {
                    "type": "array",
                    "description": "Array of button specifications",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "description": "Unique identifier returned when clicked (e.g. 'confirm', 'cancel')"
                            },
                            "label": {
                                "type": "string",
                                "description": "Button display text"
                            },
                            "style": {
                                "type": "string",
                                "enum": ["primary", "danger", "success", "secondary"],
                                "description": "Button visual style (default: secondary)"
                            },
                            "context": {
                                "type": "string",
                                "description": "Opaque context data returned when the button is clicked. Use this to carry task IDs, action parameters, or any data needed to fulfill the button's action (max 2000 chars)."
                            }
                        },
                        "required": ["id", "label"]
                    }
                }
            },
            "required": ["buttons"]
        })
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            actions: actions![add: ro],
            category: ToolCategory::Core,
            concurrency: ToolConcurrency::ReadOnly,
            ..Default::default()
        }
    }

    async fn execute(&self, params: Value, ctx: &ExecutionContext) -> anyhow::Result<ToolResult> {
        let buttons_arr = params["buttons"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("buttons must be an array"))?;

        if buttons_arr.is_empty() {
            return Ok(ToolResult::error("buttons array must not be empty"));
        }
        if buttons_arr.len() > 5 {
            return Ok(ToolResult::error(
                "maximum 5 buttons per message (Slack/Discord limitation)",
            ));
        }

        let mut specs = Vec::with_capacity(buttons_arr.len());
        for b in buttons_arr {
            let id = b["id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("each button must have an 'id' string"))?;
            // Validate ID: must be non-empty, max 64 chars, alphanumeric/hyphen/underscore only.
            // IDs become [button:{id}] in inbound messages — unsafe chars could inject content.
            if id.is_empty() || id.len() > 64 {
                return Ok(ToolResult::error("button id must be 1-64 characters"));
            }
            if !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return Ok(ToolResult::error(
                    "button id must contain only alphanumeric characters, hyphens, or underscores",
                ));
            }
            let label = b["label"].as_str().unwrap_or(id);
            let style = b["style"].as_str().unwrap_or("secondary");
            let context = b["context"].as_str().map(|s| {
                if s.len() > 2000 {
                    s[..s.floor_char_boundary(2000)].to_string()
                } else {
                    s.to_string()
                }
            });
            specs.push(ButtonSpec {
                id: id.to_string(),
                label: label.to_string(),
                style: style.to_string(),
                context,
            });
        }

        if let Some(request_id) = ctx
            .metadata
            .get(REQUEST_ID_META_KEY)
            .and_then(Value::as_str)
        {
            self.pending.store(request_id, specs);
        }

        Ok(ToolResult::new(
            "Buttons will be attached to your next response message.",
        ))
    }
}

#[cfg(test)]
mod tests;
