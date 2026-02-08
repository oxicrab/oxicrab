use nanobot::session::{Session, SessionManager};
use std::collections::HashMap;
use tempfile::TempDir;

/// Create a SessionManager that uses an isolated temp directory.
/// Sets NANOBOT_HOME so `get_nanobot_home()` returns the temp dir,
/// ensuring sessions are stored in `<tmp>/sessions/` instead of `~/.nanobot/sessions/`.
fn create_test_session_manager() -> (SessionManager, TempDir) {
    let tmp = TempDir::new().expect("Failed to create temp dir");
    // SessionManager::new() uses get_nanobot_home() for the sessions dir.
    // Setting NANOBOT_HOME makes it use our temp dir.
    std::env::set_var("NANOBOT_HOME", tmp.path());
    let mgr =
        SessionManager::new(tmp.path().to_path_buf()).expect("Failed to create SessionManager");
    (mgr, tmp)
}

#[tokio::test(flavor = "current_thread")]
async fn test_session_save_and_load() {
    let (mgr, tmp) = create_test_session_manager();

    // Create and populate session
    let mut session = mgr.get_or_create("test:chat1").await.unwrap();
    assert_eq!(session.key, "test:chat1");
    assert!(session.messages.is_empty());

    session.add_message("user".to_string(), "Hello".to_string(), HashMap::new());
    session.add_message(
        "assistant".to_string(),
        "Hi there!".to_string(),
        HashMap::new(),
    );
    mgr.save(&session).await.unwrap();

    // Create a new manager pointing at the same directory to force load from disk
    std::env::set_var("NANOBOT_HOME", tmp.path());
    let mgr2 =
        SessionManager::new(tmp.path().to_path_buf()).expect("Failed to create second manager");
    let loaded = mgr2.get_or_create("test:chat1").await.unwrap();

    assert_eq!(loaded.messages.len(), 2);
    assert_eq!(loaded.messages[0].role, "user");
    assert_eq!(loaded.messages[0].content, "Hello");
    assert_eq!(loaded.messages[1].role, "assistant");
    assert_eq!(loaded.messages[1].content, "Hi there!");
}

#[tokio::test(flavor = "current_thread")]
async fn test_session_pruning_at_capacity() {
    let (_mgr, _tmp) = create_test_session_manager();

    let mut session = Session::new("test:prune".to_string());

    // Add 250 messages (max is 200)
    for i in 0..250 {
        session.add_message("user".to_string(), format!("Message {}", i), HashMap::new());
    }

    // Should be capped at 200
    assert_eq!(session.messages.len(), 200);
    // First message should be the one at index 50 (0-49 pruned)
    assert_eq!(session.messages[0].content, "Message 50");
    // Last should be 249
    assert_eq!(session.messages[199].content, "Message 249");
}

#[tokio::test(flavor = "current_thread")]
async fn test_session_cleanup_removes_old_files() {
    let (mgr, _tmp) = create_test_session_manager();

    // Create and save a session
    let mut session = mgr.get_or_create("test:old").await.unwrap();
    session.add_message("user".to_string(), "old msg".to_string(), HashMap::new());
    mgr.save(&session).await.unwrap();

    // With TTL of 0 days, all sessions should be cleaned up
    let deleted = mgr.cleanup_old_sessions(0).unwrap();
    assert_eq!(deleted, 1);

    // Create another fresh session
    let mut session2 = mgr.get_or_create("test:fresh").await.unwrap();
    session2.add_message("user".to_string(), "fresh msg".to_string(), HashMap::new());
    mgr.save(&session2).await.unwrap();

    // With TTL of 365 days, nothing should be cleaned up
    let deleted = mgr.cleanup_old_sessions(365).unwrap();
    assert_eq!(deleted, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn test_session_history_limit() {
    let mut session = Session::new("test:history".to_string());

    for i in 0..10 {
        session.add_message("user".to_string(), format!("Msg {}", i), HashMap::new());
    }

    let history = session.get_history(3);
    assert_eq!(history.len(), 3);

    // Should return the last 3 messages (indices 7, 8, 9)
    assert_eq!(
        history[0]["content"],
        serde_json::Value::String("Msg 7".to_string())
    );
    assert_eq!(
        history[1]["content"],
        serde_json::Value::String("Msg 8".to_string())
    );
    assert_eq!(
        history[2]["content"],
        serde_json::Value::String("Msg 9".to_string())
    );
}

#[tokio::test(flavor = "current_thread")]
async fn test_session_full_history() {
    let mut session = Session::new("test:full".to_string());

    for i in 0..5 {
        session.add_message("user".to_string(), format!("Msg {}", i), HashMap::new());
    }

    let full = session.get_full_history();
    assert_eq!(full.len(), 5);
    for (i, entry) in full.iter().enumerate() {
        assert_eq!(
            entry["content"],
            serde_json::Value::String(format!("Msg {}", i))
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn test_sessions_are_isolated() {
    let (mgr, tmp) = create_test_session_manager();

    let mut session_a = mgr.get_or_create("chan_a:chat_1").await.unwrap();
    session_a.add_message("user".to_string(), "From A".to_string(), HashMap::new());
    mgr.save(&session_a).await.unwrap();

    let mut session_b = mgr.get_or_create("chan_b:chat_2").await.unwrap();
    session_b.add_message("user".to_string(), "From B".to_string(), HashMap::new());
    mgr.save(&session_b).await.unwrap();

    // Reload and verify isolation
    std::env::set_var("NANOBOT_HOME", tmp.path());
    let mgr2 =
        SessionManager::new(tmp.path().to_path_buf()).expect("Failed to create second manager");
    let loaded_a = mgr2.get_or_create("chan_a:chat_1").await.unwrap();
    let loaded_b = mgr2.get_or_create("chan_b:chat_2").await.unwrap();

    assert_eq!(loaded_a.messages.len(), 1);
    assert_eq!(loaded_a.messages[0].content, "From A");
    assert_eq!(loaded_b.messages.len(), 1);
    assert_eq!(loaded_b.messages[0].content, "From B");
}

#[tokio::test(flavor = "current_thread")]
async fn test_session_metadata_persists() {
    let (mgr, tmp) = create_test_session_manager();

    let mut session = mgr.get_or_create("test:meta").await.unwrap();
    session
        .metadata
        .insert("key1".to_string(), serde_json::json!("value1"));
    session
        .metadata
        .insert("key2".to_string(), serde_json::json!(42));
    session.add_message("user".to_string(), "Hello".to_string(), HashMap::new());
    mgr.save(&session).await.unwrap();

    // Load from disk via new manager
    std::env::set_var("NANOBOT_HOME", tmp.path());
    let mgr2 =
        SessionManager::new(tmp.path().to_path_buf()).expect("Failed to create second manager");
    let loaded = mgr2.get_or_create("test:meta").await.unwrap();

    assert_eq!(
        loaded.metadata.get("key1"),
        Some(&serde_json::json!("value1"))
    );
    assert_eq!(loaded.metadata.get("key2"), Some(&serde_json::json!(42)));
}

#[tokio::test(flavor = "current_thread")]
async fn test_new_session_has_timestamps() {
    let session = Session::new("test:timestamps".to_string());
    assert!(session.created_at <= chrono::Utc::now());
    assert!(session.updated_at <= chrono::Utc::now());
}
