mod common;

use common::{
    MockLLMProvider, TestAgentOverrides, create_test_agent_with, text_response, tool_call,
    tool_response,
};
use serde_json::json;
use tempfile::TempDir;

#[tokio::test]
async fn test_write_then_read_across_turns() {
    let tmp = TempDir::new().expect("create temp dir");
    let target = tmp.path().join("cross_turn.txt");
    // Create placeholder so canonicalize works for write_file
    std::fs::write(&target, "").expect("write placeholder file");

    let provider = MockLLMProvider::with_responses(vec![
        // Turn 1: write file
        tool_response(vec![tool_call(
            "tc1",
            "write_file",
            json!({"path": target.to_str().unwrap(), "content": "turn 1 data"}),
        )]),
        text_response("File written."),
        // Turn 2: read the same file
        tool_response(vec![tool_call(
            "tc2",
            "read_file",
            json!({"path": target.to_str().unwrap()}),
        )]),
        text_response("File contains turn 1 data."),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;

    // Turn 1
    let resp1 = agent
        .process_direct("Write cross_turn.txt", "test:mt1", "telegram", "mt1")
        .await
        .expect("process message");
    assert_eq!(resp1, "File written.");

    // Turn 2 (same session)
    let resp2 = agent
        .process_direct("Read cross_turn.txt", "test:mt1", "telegram", "mt1")
        .await
        .expect("process message");
    assert_eq!(resp2, "File contains turn 1 data.");

    // Turn 2 should have history from turn 1
    let recorded = calls.lock().expect("lock recorded calls");
    // 4 calls total: write + text, read + text
    assert_eq!(recorded.len(), 4);
    // The 3rd call (start of turn 2) should have more messages than just system + user
    let turn2_msgs = &recorded[2].messages;
    assert!(
        turn2_msgs.len() >= 3,
        "Turn 2 should include history from turn 1, got {} messages",
        turn2_msgs.len()
    );
}

#[tokio::test]
async fn test_cross_channel_isolation() {
    let tmp = TempDir::new().expect("create temp dir");

    let provider = MockLLMProvider::with_responses(vec![
        text_response("Telegram response"),
        text_response("Discord response"),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;

    let resp_tg = agent
        .process_direct("Hello from TG", "telegram:chatA", "telegram", "chatA")
        .await
        .expect("process message");
    assert_eq!(resp_tg, "Telegram response");

    let resp_dc = agent
        .process_direct("Hello from DC", "discord:chatB", "discord", "chatB")
        .await
        .expect("process message");
    assert_eq!(resp_dc, "Discord response");

    // Discord call should NOT see Telegram's history
    let recorded = calls.lock().expect("lock recorded calls");
    let discord_msgs = &recorded[1].messages;
    let has_tg = discord_msgs
        .iter()
        .any(|m| m.content.contains("Hello from TG"));
    assert!(!has_tg, "Discord session should not see Telegram messages");
}

#[tokio::test]
async fn test_same_channel_different_chats() {
    let tmp = TempDir::new().expect("create temp dir");

    let provider = MockLLMProvider::with_responses(vec![
        text_response("Chat A response"),
        text_response("Chat B response"),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;

    agent
        .process_direct("I'm in chat A", "telegram:chatA", "telegram", "chatA")
        .await
        .expect("process message");

    agent
        .process_direct("I'm in chat B", "telegram:chatB", "telegram", "chatB")
        .await
        .expect("process message");

    // Chat B should not see Chat A's messages
    let recorded = calls.lock().expect("lock recorded calls");
    let chat_b_msgs = &recorded[1].messages;
    let has_chat_a = chat_b_msgs
        .iter()
        .any(|m| m.content.contains("I'm in chat A"));
    assert!(!has_chat_a, "Chat B should not see Chat A's messages");
}

#[tokio::test]
async fn test_session_persists_across_agent_restarts() {
    let tmp = TempDir::new().expect("create temp dir");

    // First agent instance
    {
        let provider = MockLLMProvider::with_responses(vec![text_response("First agent response")]);
        let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;
        agent
            .process_direct("Remember this", "test:restart", "telegram", "restart")
            .await
            .expect("process message");
    }

    // Second agent instance (simulating restart) on same workspace
    {
        let provider =
            MockLLMProvider::with_responses(vec![text_response("Second agent response")]);
        let calls = provider.calls.clone();
        let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;
        agent
            .process_direct("What did I say?", "test:restart", "telegram", "restart")
            .await
            .expect("process message");

        // Second agent should have history from the first (persisted to disk)
        let recorded = calls.lock().expect("lock recorded calls");
        let msgs = &recorded[0].messages;
        assert!(
            msgs.len() >= 3,
            "Should have history from previous agent, got {} messages",
            msgs.len()
        );
    }
}

#[tokio::test]
async fn test_long_conversation_with_tool_use() {
    let tmp = TempDir::new().expect("create temp dir");

    let file_txt = tmp.path().join("file.txt");
    // Create placeholder so canonicalize works for write_file
    std::fs::write(&file_txt, "").expect("write placeholder file");

    let provider = MockLLMProvider::with_responses(vec![
        // Turn 1: text only
        text_response("Hello! How can I help?"),
        // Turn 2: tool use
        tool_response(vec![tool_call(
            "tc1",
            "list_dir",
            json!({"path": tmp.path().to_str().unwrap()}),
        )]),
        text_response("The directory is empty."),
        // Turn 3: write file
        tool_response(vec![tool_call(
            "tc2",
            "write_file",
            json!({"path": file_txt.to_str().unwrap(), "content": "data"}),
        )]),
        text_response("Created file.txt."),
        // Turn 4: text
        text_response("Anything else?"),
        // Turn 5: read file
        tool_response(vec![tool_call(
            "tc3",
            "read_file",
            json!({"path": file_txt.to_str().unwrap()}),
        )]),
        text_response("File contains: data"),
    ]);
    let calls = provider.calls.clone();

    let agent = create_test_agent_with(provider, &tmp, TestAgentOverrides::default()).await;

    let session = "test:long";
    agent
        .process_direct("Hi", session, "telegram", "long")
        .await
        .expect("process message");
    agent
        .process_direct("List directory", session, "telegram", "long")
        .await
        .expect("process message");
    agent
        .process_direct("Create a file", session, "telegram", "long")
        .await
        .expect("process message");
    agent
        .process_direct("Thanks", session, "telegram", "long")
        .await
        .expect("process message");
    let resp = agent
        .process_direct("Read the file", session, "telegram", "long")
        .await
        .expect("process message");

    assert_eq!(resp, "File contains: data");

    // Verify history accumulates
    let recorded = calls.lock().expect("lock recorded calls");
    // Last call should have substantial history
    let last_call = recorded.last().unwrap();
    let msg_count = last_call.messages.len();
    assert!(
        msg_count >= 5,
        "Last turn should have accumulated history, got {} messages",
        msg_count
    );
}
