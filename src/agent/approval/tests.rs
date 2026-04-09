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
        source_channel: "slack:D_USER".into(),
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
        source_channel: "slack:D_USER".into(),
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
        source_channel: "slack:D_USER".into(),
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
fn test_self_approval_same_channel() {
    let store = ApprovalStore::new();
    let (tx, mut rx) = oneshot::channel();
    let entry = ApprovalEntry {
        sender: tx,
        tool_name: "gmail".into(),
        action: "send".into(),
        requested_by: "user1".into(),
        operator_channel: String::new(), // self-approval
        source_channel: "slack:D_USER".into(),
    };
    store.register("appr-abc123", entry);
    // Self-approval must come from the same channel as the original request
    let result = store.resolve("appr-abc123", "slack:D_USER", ApprovalDecision::Approved);
    assert!(result.is_ok());
    assert!(rx.try_recv().is_ok());
}

#[test]
fn test_self_approval_rejects_other_channel() {
    let store = ApprovalStore::new();
    let (tx, _rx) = oneshot::channel();
    let entry = ApprovalEntry {
        sender: tx,
        tool_name: "gmail".into(),
        action: "send".into(),
        requested_by: "user1".into(),
        operator_channel: String::new(), // self-approval
        source_channel: "slack:D_USER".into(),
    };
    store.register("appr-abc123", entry);
    // Different channel should be rejected even in self-approval mode
    let result = store.resolve("appr-abc123", "discord:OTHER", ApprovalDecision::Approved);
    assert!(result.is_err());
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
        source_channel: "slack:D_USER".into(),
    };
    store.register("appr-abc123", entry);
    // Simulate timeout — drop the receiver
    drop(rx);
    // Resolve should fail because the receiver is gone
    let result = store.resolve("appr-abc123", "slack:C123", ApprovalDecision::Approved);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("timed out"));
}
