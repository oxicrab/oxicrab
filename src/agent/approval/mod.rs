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
}

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

        // Validate source channel (empty operator_channel = self-approval, accept any source)
        if !entry.operator_channel.is_empty() && source_channel != entry.operator_channel {
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
