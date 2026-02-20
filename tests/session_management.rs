use oxicrab::session::{Session, SessionManager};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

/// Create a SessionManager that uses an isolated temp directory.
fn create_test_session_manager() -> (SessionManager, TempDir) {
    let tmp = TempDir::new().expect("Failed to create temp dir");
    let mgr =
        SessionManager::new(tmp.path().to_path_buf()).expect("Failed to create SessionManager");
    (mgr, tmp)
}

#[tokio::test(flavor = "current_thread")]
async fn test_session_save_and_load() {
    let (mgr, tmp) = create_test_session_manager();

    // Create and populate session
    let mut session = mgr
        .get_or_create("test:chat1")
        .await
        .expect("get or create session");
    assert_eq!(session.key, "test:chat1");
    assert!(session.messages.is_empty());

    session.add_message("user".to_string(), "Hello".to_string(), HashMap::new());
    session.add_message(
        "assistant".to_string(),
        "Hi there!".to_string(),
        HashMap::new(),
    );
    mgr.save(&session).await.expect("save session");

    // Create a new manager pointing at the same directory to force load from disk
    let mgr2 =
        SessionManager::new(tmp.path().to_path_buf()).expect("Failed to create second manager");
    let loaded = mgr2
        .get_or_create("test:chat1")
        .await
        .expect("load session from disk");

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
    let mut session = mgr
        .get_or_create("test:old")
        .await
        .expect("get or create session");
    session.add_message("user".to_string(), "old msg".to_string(), HashMap::new());
    mgr.save(&session).await.expect("save session");

    // With TTL of 0 days, all sessions should be cleaned up
    let deleted = mgr.cleanup_old_sessions(0).expect("cleanup old sessions");
    assert_eq!(deleted, 1);

    // Create another fresh session
    let mut session2 = mgr
        .get_or_create("test:fresh")
        .await
        .expect("get or create session");
    session2.add_message("user".to_string(), "fresh msg".to_string(), HashMap::new());
    mgr.save(&session2).await.expect("save session");

    // With TTL of 365 days, nothing should be cleaned up
    let deleted = mgr.cleanup_old_sessions(365).expect("cleanup old sessions");
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

    let mut session_a = mgr
        .get_or_create("chan_a:chat_1")
        .await
        .expect("get or create session a");
    session_a.add_message("user".to_string(), "From A".to_string(), HashMap::new());
    mgr.save(&session_a).await.expect("save session a");

    let mut session_b = mgr
        .get_or_create("chan_b:chat_2")
        .await
        .expect("get or create session b");
    session_b.add_message("user".to_string(), "From B".to_string(), HashMap::new());
    mgr.save(&session_b).await.expect("save session b");

    // Reload and verify isolation
    let mgr2 =
        SessionManager::new(tmp.path().to_path_buf()).expect("Failed to create second manager");
    let loaded_a = mgr2
        .get_or_create("chan_a:chat_1")
        .await
        .expect("load session a from disk");
    let loaded_b = mgr2
        .get_or_create("chan_b:chat_2")
        .await
        .expect("load session b from disk");

    assert_eq!(loaded_a.messages.len(), 1);
    assert_eq!(loaded_a.messages[0].content, "From A");
    assert_eq!(loaded_b.messages.len(), 1);
    assert_eq!(loaded_b.messages[0].content, "From B");
}

#[tokio::test(flavor = "current_thread")]
async fn test_session_metadata_persists() {
    let (mgr, tmp) = create_test_session_manager();

    let mut session = mgr
        .get_or_create("test:meta")
        .await
        .expect("get or create session");
    session
        .metadata
        .insert("key1".to_string(), serde_json::json!("value1"));
    session
        .metadata
        .insert("key2".to_string(), serde_json::json!(42));
    session.add_message("user".to_string(), "Hello".to_string(), HashMap::new());
    mgr.save(&session).await.expect("save session");

    // Load from disk via new manager
    let mgr2 =
        SessionManager::new(tmp.path().to_path_buf()).expect("Failed to create second manager");
    let loaded = mgr2
        .get_or_create("test:meta")
        .await
        .expect("load session from disk");

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_concurrent_get_or_create_same_key() {
    let (mgr, _tmp) = create_test_session_manager();
    let mgr = Arc::new(mgr);

    let handles: Vec<_> = (0..10)
        .map(|_| {
            let mgr = Arc::clone(&mgr);
            tokio::spawn(async move { mgr.get_or_create("shared:key").await })
        })
        .collect();

    let results = futures_util::future::join_all(handles).await;
    for result in results {
        let session = result
            .expect("task should not panic")
            .expect("get or create should succeed");
        assert_eq!(session.key, "shared:key");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_concurrent_save_different_keys() {
    let (mgr, tmp) = create_test_session_manager();
    let mgr = Arc::new(mgr);

    let handles: Vec<_> = (0..10)
        .map(|i| {
            let mgr = Arc::clone(&mgr);
            tokio::spawn(async move {
                let key = format!("concurrent:{}", i);
                let mut session = mgr
                    .get_or_create(&key)
                    .await
                    .expect("get or create session");
                session.add_message("user", format!("msg from {}", i), HashMap::new());
                mgr.save(&session).await.expect("save session");
                key
            })
        })
        .collect();

    let keys = futures_util::future::join_all(handles).await;

    // Reload via a fresh manager and verify all 10 sessions persisted
    let mgr2 =
        SessionManager::new(tmp.path().to_path_buf()).expect("create second session manager");
    for result in keys {
        let key = result.expect("task should not panic");
        let loaded = mgr2
            .get_or_create(&key)
            .await
            .expect("load session from disk");
        assert_eq!(
            loaded.messages.len(),
            1,
            "session {} should have 1 message",
            key
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_concurrent_save_same_key_no_corruption() {
    let (mgr, tmp) = create_test_session_manager();
    let mgr = Arc::new(mgr);

    let handles: Vec<_> = (0..5)
        .map(|i| {
            let mgr = Arc::clone(&mgr);
            tokio::spawn(async move {
                let mut session = mgr
                    .get_or_create("race:key")
                    .await
                    .expect("get or create session");
                session.add_message("user", format!("writer {}", i), HashMap::new());
                mgr.save(&session).await.expect("save session");
            })
        })
        .collect();

    futures_util::future::join_all(handles)
        .await
        .into_iter()
        .for_each(|r| r.expect("task should not panic"));

    // Reload from disk — file should be valid JSON (not corrupted)
    let mgr2 =
        SessionManager::new(tmp.path().to_path_buf()).expect("create second session manager");
    let loaded = mgr2
        .get_or_create("race:key")
        .await
        .expect("load session from disk after concurrent writes");
    // At least one writer's message should be present (last writer wins)
    assert!(
        !loaded.messages.is_empty(),
        "session should have at least one message after concurrent writes"
    );
}
