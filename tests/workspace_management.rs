mod common;

use common::{
    MockLLMProvider, TestAgentOverrides, create_test_agent_with, text_response, tool_call,
    tool_response,
};
use serde_json::json;
use tempfile::TempDir;

#[tokio::test]
async fn test_workspace_tool_list_through_agent() {
    let tmp = TempDir::new().unwrap();

    // Pre-create a file in a category directory
    let file_path = tmp.path().join("code/2026-02-27/hello.py");
    std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    std::fs::write(&file_path, "print('hello')").unwrap();

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "workspace",
            json!({"action": "list", "category": "code"}),
        )]),
        text_response("Here are the workspace files."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;

    // The workspace tool should be available and work
    let response = agent
        .process_direct("List workspace files", "test:ws1", "telegram", "ws1")
        .await
        .expect("process message");

    assert_eq!(response, "Here are the workspace files.");

    // Verify the workspace tool was called and succeeded
    let recorded = calls.lock().unwrap();
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(
        !tool_msg.is_error,
        "workspace list should succeed: {}",
        tool_msg.content
    );
}

#[tokio::test]
async fn test_workspace_tool_tree_through_agent() {
    let tmp = TempDir::new().unwrap();

    // Create some category directories with files
    std::fs::create_dir_all(tmp.path().join("code/2026-02-27")).unwrap();
    std::fs::write(tmp.path().join("code/2026-02-27/app.rs"), "fn main(){}").unwrap();
    std::fs::create_dir_all(tmp.path().join("data/2026-02-27")).unwrap();
    std::fs::write(tmp.path().join("data/2026-02-27/out.csv"), "a,b").unwrap();

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "workspace",
            json!({"action": "tree"}),
        )]),
        text_response("Done."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;
    let _response = agent
        .process_direct("Show workspace tree", "test:ws2", "telegram", "ws2")
        .await
        .unwrap();

    let recorded = calls.lock().unwrap();
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(!tool_msg.is_error);
    assert!(
        tool_msg.content.contains("code"),
        "tree should contain 'code' dir"
    );
    assert!(
        tool_msg.content.contains("data"),
        "tree should contain 'data' dir"
    );
}

#[tokio::test]
async fn test_write_file_auto_registers_in_workspace() {
    let tmp = TempDir::new().unwrap();

    // First call: write a file to a category dir
    // Second call: list workspace files to see it registered
    let target = tmp.path().join("code/2026-02-27/new_script.py");
    // Create parent so canonicalize works
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    // Create placeholder file (canonicalize requires existing path)
    std::fs::write(&target, "").unwrap();

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "write_file",
            json!({"path": target.to_str().unwrap(), "content": "print('new')"}),
        )]),
        tool_response(vec![tool_call(
            "tc2",
            "workspace",
            json!({"action": "list", "category": "code"}),
        )]),
        text_response("Listed."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;
    let _response = agent
        .process_direct("Write and list", "test:ws3", "telegram", "ws3")
        .await
        .unwrap();

    // The list result should show the file we wrote
    let recorded = calls.lock().unwrap();
    let third_msgs = &recorded[2].messages;
    let tool_msg = third_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(!tool_msg.is_error);
    assert!(
        tool_msg.content.contains("new_script.py"),
        "workspace list should contain auto-registered file: {}",
        tool_msg.content
    );
}
