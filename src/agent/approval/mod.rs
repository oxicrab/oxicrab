use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::oneshot;

#[derive(Debug)]
pub enum ApprovalDecision {
    Approved,
    Denied { reason: Option<String> },
}

pub(crate) struct ApprovalEntry {
    pub sender: oneshot::Sender<ApprovalDecision>,
    pub tool_name: String,
    pub action: String,
    pub requested_by: String,
    pub operator_channel: String,
    /// Channel the original request came from (for self-approval isolation).
    pub source_channel: String,
}

/// Ephemeral store of pending operator approval requests.
///
/// Maps approval IDs (`appr-{full-uuid-v4-hex}`) to oneshot senders that the
/// agent loop awaits while it asks the operator. `register()` stashes the
/// entry; `resolve()` validates that the response came from the same channel
/// that originated the request (self-approval isolation) and fires the
/// oneshot. Nothing persists — pending approvals are dropped on restart and
/// the agent loop sees a timeout.
///
/// **Check order in `execute_tool_call`:**
/// 1. MCP hard-block (untrusted servers)
/// 2. Interactive approval (when `approval.enabled` covers the tool/action)
/// 3. Legacy `requires_approval_for_action()` hard-block (only when
///    interactive approval is disabled — provides a fallback gate)
/// 4. Normal execution.
///
/// Interactive approval also runs on the direct-dispatch and action-dispatch
/// paths, not just LLM tool calls.
///
/// **Deadlock note:** the `__approval` synthetic dispatch target is handled
/// in `process_message()` *before* the per-session lock is taken, so an
/// operator approving from the same channel as the requester (self-approval)
/// doesn't block waiting on the lock that the original request still holds.
pub struct ApprovalStore {
    pending: Mutex<HashMap<String, ApprovalEntry>>,
}

impl Default for ApprovalStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ApprovalStore {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn register(&self, approval_id: &str, entry: ApprovalEntry) {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(approval_id.to_string(), entry);
    }

    /// Resolve a pending approval. Returns the tool name, action, and requester
    /// on success, or an error message if not found or unauthorized.
    pub fn resolve(
        &self,
        approval_id: &str,
        source_channel: &str,
        decision: ApprovalDecision,
    ) -> Result<(String, String, String), String> {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let Some(entry) = pending.remove(approval_id) else {
            return Err("this approval request has already been resolved or expired".into());
        };

        // Validate source channel:
        // - Non-empty operator_channel: must match exactly (dedicated operator)
        // - Empty operator_channel (self-approval): must match the original request channel
        let expected_channel = if entry.operator_channel.is_empty() {
            &entry.source_channel
        } else {
            &entry.operator_channel
        };
        if source_channel != expected_channel {
            // Put entry back — wrong channel, don't consume it
            let tool_name = entry.tool_name.clone();
            pending.insert(approval_id.to_string(), entry);
            return Err(format!(
                "approval response from unauthorized channel for {tool_name}"
            ));
        }

        let tool_name = entry.tool_name.clone();
        let action = entry.action.clone();
        let requested_by = entry.requested_by.clone();
        // If the receiver was dropped (timeout), send() returns Err — surface it
        entry
            .sender
            .send(decision)
            .map_err(|_| "approval request has already timed out or been cancelled".to_string())?;
        Ok((tool_name, action, requested_by))
    }

    /// Remove a pending approval entry (e.g., on timeout).
    pub fn remove(&self, approval_id: &str) {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(approval_id);
    }

    pub fn generate_id() -> String {
        format!("appr-{}", uuid::Uuid::new_v4().simple())
    }

    /// Return the IDs of all currently pending approvals.
    /// Useful for integration tests that need to find and resolve approvals.
    pub fn pending_ids(&self) -> Vec<String> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .keys()
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests;
