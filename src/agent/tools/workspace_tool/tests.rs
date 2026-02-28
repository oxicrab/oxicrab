use super::*;
use crate::agent::memory::memory_db::MemoryDB;
use crate::agent::tools::Tool;
use crate::agent::tools::base::ExecutionContext;
use crate::agent::workspace::WorkspaceManager;
use crate::config::schema::WorkspaceTtlConfig;
use std::sync::Arc;

fn test_tool() -> (tempfile::TempDir, WorkspaceTool) {
    let tmp = tempfile::tempdir().unwrap();
    let db = Arc::new(MemoryDB::new(tmp.path().join("memory/memory.sqlite3")).unwrap());
    let mgr = Arc::new(WorkspaceManager::new(tmp.path().to_path_buf(), Some(db)));
    let tool = WorkspaceTool::new(mgr, WorkspaceTtlConfig::default());
    (tmp, tool)
}

fn test_ctx() -> ExecutionContext {
    ExecutionContext::default()
}

#[test]
fn test_workspace_tool_name_and_capabilities() {
    let (_tmp, tool) = test_tool();

    assert_eq!(tool.name(), "workspace");
    assert_eq!(tool.version().major, 0);
    assert_eq!(tool.version().minor, 1);
    assert_eq!(tool.version().patch, 0);

    let caps = tool.capabilities();
    assert!(caps.built_in);
    assert!(!caps.network_outbound);
    assert_eq!(caps.subagent_access, SubagentAccess::ReadOnly);
    assert_eq!(caps.actions.len(), 8);

    // Verify read-only flags
    let action_map: std::collections::HashMap<&str, bool> =
        caps.actions.iter().map(|a| (a.name, a.read_only)).collect();

    assert_eq!(action_map.get("list"), Some(&true));
    assert_eq!(action_map.get("search"), Some(&true));
    assert_eq!(action_map.get("info"), Some(&true));
    assert_eq!(action_map.get("tree"), Some(&true));
    assert_eq!(action_map.get("move"), Some(&false));
    assert_eq!(action_map.get("delete"), Some(&false));
    assert_eq!(action_map.get("tag"), Some(&false));
    assert_eq!(action_map.get("cleanup"), Some(&false));
}

#[tokio::test]
async fn test_workspace_tool_list_action() {
    let (tmp, tool) = test_tool();
    let ctx = test_ctx();

    // Create and register a file
    let code_dir = tmp.path().join("code/2026-02-27");
    std::fs::create_dir_all(&code_dir).unwrap();
    let file = code_dir.join("script.py");
    std::fs::write(&file, "print('hello')").unwrap();
    tool.manager
        .register_file(&file, Some("test"), None)
        .unwrap();

    let params = serde_json::json!({ "action": "list" });
    let result = tool.execute(params, &ctx).await.unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("script.py"));
    assert!(result.content.contains("code"));
    assert!(result.content.contains("1 file(s)"));
}

#[tokio::test]
async fn test_workspace_tool_search_action() {
    let (tmp, tool) = test_tool();
    let ctx = test_ctx();

    // Create and register a file
    let data_dir = tmp.path().join("data/2026-02-27");
    std::fs::create_dir_all(&data_dir).unwrap();
    let file = data_dir.join("users.csv");
    std::fs::write(&file, "id,name\n1,alice").unwrap();
    tool.manager.register_file(&file, None, None).unwrap();

    let params = serde_json::json!({ "action": "search", "query": "users" });
    let result = tool.execute(params, &ctx).await.unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("users.csv"));
}

#[tokio::test]
async fn test_workspace_tool_tree_action() {
    let (tmp, tool) = test_tool();
    let ctx = test_ctx();

    // Create category dirs with files
    let code_dir = tmp.path().join("code/2026-02-27");
    std::fs::create_dir_all(&code_dir).unwrap();
    std::fs::write(code_dir.join("a.py"), "pass").unwrap();
    std::fs::write(code_dir.join("b.py"), "pass").unwrap();

    let data_dir = tmp.path().join("data/2026-02-27");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::write(data_dir.join("c.csv"), "x").unwrap();

    // Create an empty temp dir
    std::fs::create_dir_all(tmp.path().join("temp")).unwrap();

    let params = serde_json::json!({ "action": "tree" });
    let result = tool.execute(params, &ctx).await.unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("workspace/"));
    assert!(result.content.contains("code/"));
    assert!(result.content.contains("data/"));
    assert!(result.content.contains("temp/"));
    assert!(result.content.contains("2 files"));
    assert!(result.content.contains("1 file)"));
}

#[tokio::test]
async fn test_workspace_tool_delete_action() {
    let (tmp, tool) = test_tool();
    let ctx = test_ctx();

    // Create and register a file
    let code_dir = tmp.path().join("code/2026-02-27");
    std::fs::create_dir_all(&code_dir).unwrap();
    let file = code_dir.join("delete_me.py");
    std::fs::write(&file, "pass").unwrap();
    tool.manager.register_file(&file, None, None).unwrap();

    assert!(file.exists());

    let rel_path = "code/2026-02-27/delete_me.py";
    let params = serde_json::json!({ "action": "delete", "path": rel_path });
    let result = tool.execute(params, &ctx).await.unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("Deleted"));
    assert!(!file.exists());
}

#[tokio::test]
async fn test_workspace_tool_tag_action() {
    let (tmp, tool) = test_tool();
    let ctx = test_ctx();

    // Create and register a file
    let code_dir = tmp.path().join("code/2026-02-27");
    std::fs::create_dir_all(&code_dir).unwrap();
    let file = code_dir.join("tag_me.py");
    std::fs::write(&file, "pass").unwrap();
    tool.manager.register_file(&file, None, None).unwrap();

    let rel_path = "code/2026-02-27/tag_me.py";
    let params =
        serde_json::json!({ "action": "tag", "path": rel_path, "tags": "important,review" });
    let result = tool.execute(params, &ctx).await.unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("Tagged"));
    assert!(result.content.contains("important,review"));

    // Verify tag filter works via list
    let list_params = serde_json::json!({ "action": "list", "tags": "important" });
    let list_result = tool.execute(list_params, &ctx).await.unwrap();
    assert!(!list_result.is_error);
    assert!(list_result.content.contains("tag_me.py"));
}

#[tokio::test]
async fn test_workspace_tool_move_action() {
    let (tmp, tool) = test_tool();
    let ctx = test_ctx();

    // Create and register a file in code/
    let code_dir = tmp.path().join("code/2026-02-27");
    std::fs::create_dir_all(&code_dir).unwrap();
    let file = code_dir.join("move_me.txt");
    std::fs::write(&file, "content").unwrap();
    tool.manager.register_file(&file, None, None).unwrap();

    let rel_path = "code/2026-02-27/move_me.txt";
    let params = serde_json::json!({ "action": "move", "path": rel_path, "category": "data" });
    let result = tool.execute(params, &ctx).await.unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("Moved to"));
    assert!(result.content.contains("data"));

    // Old file should be gone
    assert!(!file.exists());

    // New file should exist
    let new_file = tmp.path().join("data/2026-02-27/move_me.txt");
    assert!(new_file.exists());
}

#[tokio::test]
async fn test_workspace_tool_cleanup_action() {
    let (tmp, tool) = test_tool();
    let ctx = test_ctx();
    let db = tool.manager.db().unwrap();

    // Create a file on disk in temp/
    let tmp_dir = tmp.path().join("temp/2026-01-01");
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let file = tmp_dir.join("old_file.txt");
    std::fs::write(&file, "expired").unwrap();

    // Register with a backdated created_at (60 days ago)
    db.register_workspace_file_with_date(
        "temp/2026-01-01/old_file.txt",
        "temp",
        Some("old_file.txt"),
        7,
        None,
        None,
        "2025-12-01 00:00:00",
    )
    .unwrap();

    assert!(file.exists());

    let params = serde_json::json!({ "action": "cleanup" });
    let result = tool.execute(params, &ctx).await.unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("Cleanup complete"));
    assert!(result.content.contains("1 expired file(s) removed"));
    assert!(!file.exists());
}

#[tokio::test]
async fn test_workspace_tool_unknown_action() {
    let (_tmp, tool) = test_tool();
    let ctx = test_ctx();

    let params = serde_json::json!({ "action": "bogus" });
    let result = tool.execute(params, &ctx).await.unwrap();

    assert!(result.is_error);
    assert!(result.content.contains("unknown action"));
    assert!(result.content.contains("bogus"));
}
