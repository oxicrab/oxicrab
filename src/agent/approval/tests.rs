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
fn test_resolve_wrong_channel_preserves_entry_for_retry() {
    // Security-sensitive: a wrong-channel resolve attempt must NOT consume
    // the pending entry — the operator on the correct channel still has
    // to be able to approve. Earlier `pending.remove()` then re-`insert()`
    // logic would silently drop the entry if the re-insert path were ever
    // skipped. This test guards against that regression.
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

    // Wrong-channel attempt is rejected.
    let bad = store.resolve("appr-abc123", "slack:CWRONG", ApprovalDecision::Approved);
    assert!(bad.is_err());
    // Receiver MUST still be pending (no decision sent).
    assert!(
        matches!(rx.try_recv(), Err(oneshot::error::TryRecvError::Empty)),
        "no decision should have been sent"
    );
    assert_eq!(
        store.pending_ids(),
        vec!["appr-abc123".to_string()],
        "entry should still be pending after wrong-channel attempt"
    );

    // Correct-channel retry then succeeds.
    let ok = store.resolve("appr-abc123", "slack:C123", ApprovalDecision::Approved);
    assert!(ok.is_ok());
    assert!(rx.try_recv().is_ok(), "decision should arrive on retry");
    assert!(
        store.pending_ids().is_empty(),
        "entry consumed after successful resolve"
    );
}

#[test]
fn test_self_approval_wrong_channel_preserves_entry_for_retry() {
    // Same regression guard for the self-approval (operator_channel empty)
    // code path — must use source_channel as the expected channel.
    let store = ApprovalStore::new();
    let (tx, mut rx) = oneshot::channel();
    let entry = ApprovalEntry {
        sender: tx,
        tool_name: "shell".into(),
        action: "execute".into(),
        requested_by: "user1".into(),
        operator_channel: String::new(),
        source_channel: "slack:D_USER".into(),
    };
    store.register("appr-self", entry);

    let bad = store.resolve("appr-self", "discord:OTHER", ApprovalDecision::Approved);
    assert!(bad.is_err());
    assert!(matches!(
        rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert_eq!(store.pending_ids(), vec!["appr-self".to_string()]);

    // Original channel can still resolve.
    assert!(
        store
            .resolve("appr-self", "slack:D_USER", ApprovalDecision::Approved)
            .is_ok()
    );
    assert!(rx.try_recv().is_ok());
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
