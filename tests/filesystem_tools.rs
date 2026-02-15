mod common;

use common::{
    create_test_agent_with, text_response, tool_call, tool_response, MockLLMProvider,
    TestAgentOverrides,
};
use serde_json::json;
use tempfile::TempDir;

#[tokio::test]
async fn test_write_file_through_agent_loop() {
    let tmp = TempDir::new().unwrap();
    // Create a placeholder so canonicalize resolves, then write_file overwrites it
    let target = tmp.path().join("output.txt");
    std::fs::write(&target, "").unwrap();

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "write_file",
            json!({"path": target.to_str().unwrap(), "content": "hello world"}),
        )]),
        text_response("File written."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;
    let response = agent
        .process_direct("Write a file", "test:fs1", "telegram", "fs1")
        .await
        .unwrap();

    assert_eq!(response, "File written.");

    // Verify the tool succeeded
    let recorded = calls.lock().unwrap();
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(
        !tool_msg.is_error,
        "write_file should succeed: {}",
        tool_msg.content
    );

    assert_eq!(std::fs::read_to_string(&target).unwrap(), "hello world");
}

#[tokio::test]
async fn test_read_file_through_agent_loop() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("data.txt");
    std::fs::write(&target, "file contents here").unwrap();

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "read_file",
            json!({"path": target.to_str().unwrap()}),
        )]),
        text_response("Got it."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;
    let response = agent
        .process_direct("Read data.txt", "test:fs2", "telegram", "fs2")
        .await
        .unwrap();

    assert_eq!(response, "Got it.");

    // Verify the tool result contains the file contents
    let recorded = calls.lock().unwrap();
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(tool_msg.content.contains("file contents here"));
    assert!(!tool_msg.is_error);
}

#[tokio::test]
async fn test_edit_file_through_agent_loop() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("edit_me.txt");
    std::fs::write(&target, "old text in the file").unwrap();

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "edit_file",
            json!({
                "path": target.to_str().unwrap(),
                "old_text": "old text",
                "new_text": "new text"
            }),
        )]),
        text_response("Edited."),
    ]);

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;
    let response = agent
        .process_direct("Edit the file", "test:fs3", "telegram", "fs3")
        .await
        .unwrap();

    assert_eq!(response, "Edited.");
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "new text in the file"
    );
}

#[tokio::test]
async fn test_list_dir_with_files() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("alpha.txt"), "a").unwrap();
    std::fs::write(tmp.path().join("beta.txt"), "b").unwrap();
    std::fs::create_dir(tmp.path().join("subdir")).unwrap();

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "list_dir",
            json!({"path": tmp.path().to_str().unwrap()}),
        )]),
        text_response("Listed."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;
    agent
        .process_direct("List dir", "test:fs4", "telegram", "fs4")
        .await
        .unwrap();

    let recorded = calls.lock().unwrap();
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(tool_msg.content.contains("alpha.txt"));
    assert!(tool_msg.content.contains("beta.txt"));
    assert!(tool_msg.content.contains("subdir/"));
}

#[tokio::test]
async fn test_write_then_read_multi_tool_sequence() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("roundtrip.txt");
    // Create placeholder so canonicalize works on first write
    std::fs::write(&target, "").unwrap();

    let provider = MockLLMProvider::with_responses(vec![
        // First: write
        tool_response(vec![tool_call(
            "tc1",
            "write_file",
            json!({"path": target.to_str().unwrap(), "content": "roundtrip data"}),
        )]),
        // Second: read same file
        tool_response(vec![tool_call(
            "tc2",
            "read_file",
            json!({"path": target.to_str().unwrap()}),
        )]),
        text_response("Done."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;
    let response = agent
        .process_direct("Write then read", "test:fs5", "telegram", "fs5")
        .await
        .unwrap();

    assert_eq!(response, "Done.");

    // The third call should have the read_file result with the written content
    let recorded = calls.lock().unwrap();
    assert_eq!(recorded.len(), 3);
    let third_msgs = &recorded[2].messages;
    let tool_msg = third_msgs.iter().rfind(|m| m.role == "tool").unwrap();
    assert!(tool_msg.content.contains("roundtrip data"));
}

#[tokio::test]
async fn test_write_file_outside_workspace_blocked() {
    let tmp = TempDir::new().unwrap();

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "write_file",
            json!({"path": "/tmp/nanobot_test_escape.txt", "content": "evil"}),
        )]),
        text_response("Blocked."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(
        provider,
        &tmp,
        TestAgentOverrides {
            restrict_to_workspace: Some(true),
            ..Default::default()
        },
    )
    .await;

    agent
        .process_direct("Write outside", "test:fs6", "telegram", "fs6")
        .await
        .unwrap();

    // Tool result should be an error
    let recorded = calls.lock().unwrap();
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(tool_msg.is_error);
    assert!(
        tool_msg.content.contains("outside")
            || tool_msg.content.contains("Error")
            || tool_msg.content.contains("Cannot resolve")
    );
}

#[tokio::test]
async fn test_read_file_nonexistent_returns_error() {
    let tmp = TempDir::new().unwrap();
    let missing = tmp.path().join("does_not_exist.txt");

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![tool_call(
            "tc1",
            "read_file",
            json!({"path": missing.to_str().unwrap()}),
        )]),
        text_response("File not found."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;
    agent
        .process_direct("Read missing", "test:fs7", "telegram", "fs7")
        .await
        .unwrap();

    let recorded = calls.lock().unwrap();
    let second_msgs = &recorded[1].messages;
    let tool_msg = second_msgs.iter().find(|m| m.role == "tool").unwrap();
    assert!(tool_msg.is_error);
    assert!(tool_msg.content.contains("not found") || tool_msg.content.contains("Error"));
}

#[tokio::test]
async fn test_parallel_file_reads() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("a.txt"), "content A").unwrap();
    std::fs::write(tmp.path().join("b.txt"), "content B").unwrap();

    let provider = MockLLMProvider::with_responses(vec![
        tool_response(vec![
            tool_call(
                "tc1",
                "read_file",
                json!({"path": tmp.path().join("a.txt").to_str().unwrap()}),
            ),
            tool_call(
                "tc2",
                "read_file",
                json!({"path": tmp.path().join("b.txt").to_str().unwrap()}),
            ),
        ]),
        text_response("Read both."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;
    let response = agent
        .process_direct("Read both files", "test:fs8", "telegram", "fs8")
        .await
        .unwrap();

    assert_eq!(response, "Read both.");

    // Both tool results should be present
    let recorded = calls.lock().unwrap();
    let second_msgs = &recorded[1].messages;
    let tool_msgs: Vec<_> = second_msgs.iter().filter(|m| m.role == "tool").collect();
    assert_eq!(tool_msgs.len(), 2);
    let combined: String = tool_msgs.iter().map(|m| m.content.as_str()).collect();
    assert!(combined.contains("content A"));
    assert!(combined.contains("content B"));
}
