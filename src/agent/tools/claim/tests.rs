use super::*;
use std::collections::HashMap;
use tempfile::tempdir;

fn ctx() -> ExecutionContext {
    ExecutionContext {
        channel: "test".to_string(),
        chat_id: "x".to_string(),
        context_summary: None,
        metadata: HashMap::new(),
    }
}

fn tool() -> (ClaimTool, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let db = Arc::new(MemoryDB::new(dir.path().join("t.db")).unwrap());
    (ClaimTool::new(db), dir)
}

#[tokio::test]
async fn add_then_list() {
    let (tool, _g) = tool();
    let r = tool
        .execute(
            json!({
                "action": "add",
                "text": "User prefers Rust",
                "confidence": 0.85
            }),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(!r.is_error);
    assert!(r.content.contains("#1"));
    assert!(r.content.contains("0.85"));

    let l = tool
        .execute(json!({"action": "list"}), &ctx())
        .await
        .unwrap();
    assert!(!l.is_error);
    assert!(l.content.contains("User prefers Rust"));
    assert!(l.content.contains("[open/agent_inferred]"));
}

#[tokio::test]
async fn add_with_evidence() {
    let (tool, _g) = tool();
    let r = tool
        .execute(
            json!({
                "action": "add",
                "text": "Project deadline is May 15",
                "confidence": 0.9,
                "evidence": [
                    {"kind": "message", "value": "telegram:msg-9999"},
                    {"kind": "file", "value": "ROADMAP.md"}
                ]
            }),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(r.content.contains("2 evidence pointer(s)"));

    let g = tool
        .execute(json!({"action": "get", "id": 1}), &ctx())
        .await
        .unwrap();
    assert!(g.content.contains("telegram:msg-9999"));
    assert!(g.content.contains("ROADMAP.md"));
}

#[tokio::test]
async fn lint_reports_contradictions() {
    let (tool, _g) = tool();
    tool.execute(
        json!({"action": "add", "text": "User prefers Rust"}),
        &ctx(),
    )
    .await
    .unwrap();
    tool.execute(json!({"action": "add", "text": "User prefers Go"}), &ctx())
        .await
        .unwrap();
    let lint = tool
        .execute(json!({"action": "lint"}), &ctx())
        .await
        .unwrap();
    assert!(!lint.is_error);
    assert!(lint.content.contains("# claim_lint report"));
    assert!(lint.content.contains("Potential contradictions (1"));
}

#[tokio::test]
async fn update_status_lifecycle() {
    let (tool, _g) = tool();
    tool.execute(json!({"action": "add", "text": "test claim"}), &ctx())
        .await
        .unwrap();
    let r = tool
        .execute(
            json!({"action": "update_status", "id": 1, "status": "accepted"}),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(r.content.contains("accepted"));
    let r = tool
        .execute(
            json!({"action": "update_status", "id": 1, "status": "retracted"}),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(r.content.contains("retracted"));
}

#[tokio::test]
async fn missing_action_errors() {
    let (tool, _g) = tool();
    let r = tool.execute(json!({}), &ctx()).await.unwrap();
    assert!(r.is_error);
}

#[tokio::test]
async fn unknown_status_errors() {
    let (tool, _g) = tool();
    tool.execute(json!({"action": "add", "text": "x"}), &ctx())
        .await
        .unwrap();
    let r = tool
        .execute(
            json!({"action": "update_status", "id": 1, "status": "bogus"}),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(r.is_error);
}

#[tokio::test]
async fn confidence_clamps() {
    let (tool, _g) = tool();
    let r = tool
        .execute(
            json!({"action": "add", "text": "over", "confidence": 5.0}),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(!r.is_error);
    let g = tool
        .execute(json!({"action": "get", "id": 1}), &ctx())
        .await
        .unwrap();
    assert!(g.content.contains("Confidence: 1.00"));
}

#[tokio::test]
async fn add_agent_inferred_then_accept_is_blocked() {
    let (tool, _g) = tool();
    let a = tool
        .execute(
            json!({
                "action": "add",
                "text": "User is probably a night owl"
            }),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(!a.is_error);
    assert!(a.content.contains("#1"));

    let r = tool
        .execute(
            json!({"action": "update_status", "id": 1, "status": "accepted"}),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(r.is_error);
    let lower = r.content.to_lowercase();
    assert!(
        lower.contains("operator") && lower.contains("yourself"),
        "block message must route promotion through an operator, not self-confirm: {}",
        r.content
    );
    assert!(
        !lower.contains("retry"),
        "block message must not tell the agent to confirm then retry: {}",
        r.content
    );

    let g = tool
        .execute(json!({"action": "get", "id": 1}), &ctx())
        .await
        .unwrap();
    assert!(!g.is_error);
    assert!(
        g.content.contains("Status: open"),
        "blocked accept must leave status open: {}",
        g.content
    );
}

#[tokio::test]
async fn confirm_unblocks_accept() {
    let (tool, _g) = tool();
    tool.execute(
        json!({
            "action": "add",
            "text": "User is probably a night owl"
        }),
        &ctx(),
    )
    .await
    .unwrap();

    let c = tool
        .execute(json!({"action": "confirm", "id": 1}), &ctx())
        .await
        .unwrap();
    assert!(!c.is_error);

    let r = tool
        .execute(
            json!({"action": "update_status", "id": 1, "status": "accepted"}),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(!r.is_error);

    let g = tool
        .execute(json!({"action": "get", "id": 1}), &ctx())
        .await
        .unwrap();
    assert!(
        g.content.contains("Status: accepted"),
        "confirmed claim must promote to accepted: {}",
        g.content
    );
}

#[tokio::test]
async fn agent_cannot_create_observed_claim() {
    let (tool, _g) = tool();
    // The agent tries to smuggle an `observed` provenance via params. The tool
    // must ignore it and store the claim as agent_inferred, keeping the
    // promotion gate unforgeable.
    let a = tool
        .execute(
            json!({
                "action": "add",
                "text": "Project builds with cargo",
                "provenance": "observed"
            }),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(!a.is_error);

    let g = tool
        .execute(json!({"action": "get", "id": 1}), &ctx())
        .await
        .unwrap();
    assert!(
        g.content.contains("agent_inferred"),
        "provenance param must be ignored; claim must be agent_inferred: {}",
        g.content
    );

    let r = tool
        .execute(
            json!({"action": "update_status", "id": 1, "status": "accepted"}),
            &ctx(),
        )
        .await
        .unwrap();
    assert!(
        r.is_error,
        "smuggled-observed claim must still be blocked from direct accept: {}",
        r.content
    );
}

#[tokio::test]
async fn get_shows_provenance() {
    let (tool, _g) = tool();
    tool.execute(
        json!({"action": "add", "text": "Default provenance claim"}),
        &ctx(),
    )
    .await
    .unwrap();
    let g = tool
        .execute(json!({"action": "get", "id": 1}), &ctx())
        .await
        .unwrap();
    assert!(!g.is_error);
    assert!(g.content.contains("Provenance:"));
    assert!(g.content.contains("agent_inferred"));
}

#[tokio::test]
async fn confirm_requires_operator_approval() {
    let (tool, _g) = tool();
    // The real gate: the agent loop refuses to run `confirm` without operator
    // approval, so add(agent_inferred) -> confirm -> accept is not
    // self-serviceable by the model.
    assert!(
        tool.requires_approval_for_action("confirm"),
        "confirm must require operator approval to close the self-bypass"
    );
    assert!(
        !tool.requires_approval_for_action("add"),
        "add must not require approval"
    );
    assert!(
        !tool.requires_approval_for_action("update_status"),
        "update_status must not require approval"
    );
}
