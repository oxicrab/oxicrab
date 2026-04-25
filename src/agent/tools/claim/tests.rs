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
    assert!(l.content.contains("[open]"));
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
