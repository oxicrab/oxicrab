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
