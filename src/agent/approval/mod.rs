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
mod tests {
    use super::*;

    #[test]
    fn test_register_and_resolve() {
        let store = ApprovalStore::new();
        let (tx, mut rx) = oneshot::channel();
        let entry = ApprovalEntry {
            sender: tx,
            tool_name: "gmail".into(),
            action: "send".into(),
            requested_by: "user1".into(),
            operator_channel: "slack:C123".into(),
        };
        store.register("appr-abc123", entry);
        let result = store.resolve("appr-abc123", "slack:C123", ApprovalDecision::Approved);
        assert!(result.is_ok());
        // rx should have received the decision
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn test_resolve_unknown_id() {
        let store = ApprovalStore::new();
        let result = store.resolve("appr-unknown", "slack:C123", ApprovalDecision::Approved);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_wrong_channel() {
        let store = ApprovalStore::new();
        let (tx, _rx) = oneshot::channel();
        let entry = ApprovalEntry {
            sender: tx,
            tool_name: "gmail".into(),
            action: "send".into(),
            requested_by: "user1".into(),
            operator_channel: "slack:C123".into(),
        };
        store.register("appr-abc123", entry);
        let result = store.resolve("appr-abc123", "slack:CWRONG", ApprovalDecision::Approved);
        assert!(result.is_err());
    }

    #[test]
    fn test_double_resolve() {
        let store = ApprovalStore::new();
        let (tx, mut rx) = oneshot::channel();
        let entry = ApprovalEntry {
            sender: tx,
            tool_name: "gmail".into(),
            action: "send".into(),
            requested_by: "user1".into(),
            operator_channel: "slack:C123".into(),
        };
        store.register("appr-abc123", entry);
        assert!(
            store
                .resolve("appr-abc123", "slack:C123", ApprovalDecision::Approved)
                .is_ok()
        );
        assert!(rx.try_recv().is_ok());
        // Second resolve should fail — entry consumed
        assert!(
            store
                .resolve("appr-abc123", "slack:C123", ApprovalDecision::Approved)
                .is_err()
        );
    }

    #[test]
    fn test_self_approval_empty_channel() {
        let store = ApprovalStore::new();
        let (tx, mut rx) = oneshot::channel();
        let entry = ApprovalEntry {
            sender: tx,
            tool_name: "gmail".into(),
            action: "send".into(),
            requested_by: "user1".into(),
            operator_channel: String::new(), // self-approval
        };
        store.register("appr-abc123", entry);
        // Any source channel is accepted when operator_channel is empty
        let result = store.resolve("appr-abc123", "slack:U12345", ApprovalDecision::Approved);
        assert!(result.is_ok());
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn test_resolve_after_receiver_dropped_returns_error() {
        let store = ApprovalStore::new();
        let (tx, rx) = oneshot::channel();
        let entry = ApprovalEntry {
            sender: tx,
            tool_name: "gmail".into(),
            action: "send".into(),
            requested_by: "user1".into(),
            operator_channel: "slack:C123".into(),
        };
        store.register("appr-abc123", entry);
        // Simulate timeout — drop the receiver
        drop(rx);
        // Resolve should fail because the receiver is gone
        let result = store.resolve("appr-abc123", "slack:C123", ApprovalDecision::Approved);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("timed out"));
    }
}
