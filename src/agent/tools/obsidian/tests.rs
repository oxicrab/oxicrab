use super::ObsidianTool;
use super::cache::ObsidianCache;
use super::client::ObsidianApiClient;
use crate::agent::tools::Tool;
use crate::agent::tools::base::ExecutionContext;
use std::sync::Arc;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn setup() -> (MockServer, tempfile::TempDir, Arc<ObsidianCache>) {
    let server = MockServer::start().await;
    let tmp = tempfile::TempDir::new().unwrap();
    let client = Arc::new(ObsidianApiClient::with_base_url(server.uri(), "test_key"));
    let cache = Arc::new(ObsidianCache::with_dir(client, tmp.path().to_path_buf()));
    (server, tmp, cache)
}

fn tool_with_cache(cache: Arc<ObsidianCache>) -> ObsidianTool {
    ObsidianTool { cache }
}

// --- Validation tests ---

#[tokio::test]
async fn test_missing_action() {
    let (_server, _tmp, cache) = setup().await;
    let tool = tool_with_cache(cache);
    let result = tool
        .execute(serde_json::json!({}), &ExecutionContext::default())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("unknown action"));
}

#[tokio::test]
async fn test_unknown_action() {
    let (_server, _tmp, cache) = setup().await;
    let tool = tool_with_cache(cache);
    let result = tool
        .execute(
            serde_json::json!({"action": "bogus"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("unknown action"));
}

#[tokio::test]
async fn test_read_missing_path() {
    let (_server, _tmp, cache) = setup().await;
    let tool = tool_with_cache(cache);
    let result = tool
        .execute(
            serde_json::json!({"action": "read"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("path"));
}

#[tokio::test]
async fn test_write_missing_path() {
    let (_server, _tmp, cache) = setup().await;
    let tool = tool_with_cache(cache);
    let result = tool
        .execute(
            serde_json::json!({"action": "write", "content": "hello"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("path"));
}

#[tokio::test]
async fn test_write_missing_content() {
    let (_server, _tmp, cache) = setup().await;
    let tool = tool_with_cache(cache);
    let result = tool
        .execute(
            serde_json::json!({"action": "write", "path": "test.md"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("content"));
}

#[tokio::test]
async fn test_append_missing_path() {
    let (_server, _tmp, cache) = setup().await;
    let tool = tool_with_cache(cache);
    let result = tool
        .execute(
            serde_json::json!({"action": "append", "content": "hello"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("path"));
}

#[tokio::test]
async fn test_append_missing_content() {
    let (_server, _tmp, cache) = setup().await;
    let tool = tool_with_cache(cache);
    let result = tool
        .execute(
            serde_json::json!({"action": "append", "path": "test.md"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("content"));
}

#[tokio::test]
async fn test_search_missing_query() {
    let (_server, _tmp, cache) = setup().await;
    let tool = tool_with_cache(cache);
    let result = tool
        .execute(
            serde_json::json!({"action": "search"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("query"));
}

// --- Read tests ---

#[tokio::test]
async fn test_read_not_in_cache() {
    let (_server, _tmp, cache) = setup().await;
    let tool = tool_with_cache(cache);
    let result = tool
        .execute(
            serde_json::json!({"action": "read", "path": "nonexistent.md"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("not found in cache"));
}

#[tokio::test]
async fn test_read_from_cache() {
    let (_server, tmp, cache) = setup().await;
    // Write directly to cache dir
    std::fs::write(tmp.path().join("note.md"), "# Hello\nWorld").unwrap();
    // Update state so list knows about it
    cache.write_file("note.md", "# Hello\nWorld").await.ok(); // will fail API write, but cache is populated

    let tool = tool_with_cache(cache);
    let result = tool
        .execute(
            serde_json::json!({"action": "read", "path": "note.md"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("# Hello"));
    assert!(result.content.contains("World"));
}

// --- Write tests ---

#[tokio::test]
async fn test_write_through_to_api() {
    let (server, _tmp, cache) = setup().await;

    // Mock reachability check
    Mock::given(method("GET"))
        .and(path("/vault/"))
        .and(header("Authorization", "Bearer test_key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"files": []})))
        .mount(&server)
        .await;

    // Mock write
    Mock::given(method("PUT"))
        .and(path("/vault/notes%2Fhello.md"))
        .and(header("Authorization", "Bearer test_key"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let tool = tool_with_cache(cache.clone());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "write",
                "path": "notes/hello.md",
                "content": "# Hello"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("Written"));

    // Verify it's in cache — new notes get frontmatter prepended
    let cached = cache.read_cached("notes/hello.md").unwrap();
    assert!(cached.contains("---\ncreate-date:"));
    assert!(cached.contains("type: note"));
    assert!(cached.contains("# Hello"));
}

#[tokio::test]
async fn test_write_queued_when_offline() {
    let (server, _tmp, cache) = setup().await;

    // Mock reachability check — return error (offline)
    Mock::given(method("GET"))
        .and(path("/vault/"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let tool = tool_with_cache(cache.clone());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "write",
                "path": "offline.md",
                "content": "queued content"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("queued"));

    // Verify it's in cache — new notes get frontmatter prepended
    let cached = cache.read_cached("offline.md").unwrap();
    assert!(cached.contains("---\ncreate-date:"));
    assert!(cached.contains("type: note"));
    assert!(cached.contains("queued content"));
}

// --- Append tests ---

#[tokio::test]
async fn test_append_through_to_api() {
    let (server, tmp, cache) = setup().await;

    // Pre-populate cache
    std::fs::write(tmp.path().join("existing.md"), "line1\n").unwrap();

    // Mock reachability check
    Mock::given(method("GET"))
        .and(path("/vault/"))
        .and(header("Authorization", "Bearer test_key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"files": []})))
        .mount(&server)
        .await;

    // Mock append
    Mock::given(method("POST"))
        .and(path("/vault/existing.md"))
        .and(header("Authorization", "Bearer test_key"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let tool = tool_with_cache(cache.clone());
    let result = tool
        .execute(
            serde_json::json!({
                "action": "append",
                "path": "existing.md",
                "content": "line2\n"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("Appended"));

    // Cache should have combined content
    let cached = cache.read_cached("existing.md").unwrap();
    assert!(cached.contains("line1"));
    assert!(cached.contains("line2"));
}

// --- Search tests ---

#[tokio::test]
async fn test_search_no_results() {
    let (_server, _tmp, cache) = setup().await;
    let tool = tool_with_cache(cache);
    let result = tool
        .execute(
            serde_json::json!({"action": "search", "query": "nonexistent"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("No results"));
}

#[tokio::test]
async fn test_search_finds_matches() {
    let (_server, tmp, cache) = setup().await;

    // Populate cache with files and state
    std::fs::write(tmp.path().join("a.md"), "# Rust programming\nHello world").unwrap();
    std::fs::write(tmp.path().join("b.md"), "# Python guide\nNo match here").unwrap();
    // Manually update state so search iterates these files
    {
        let mut state = cache.state.lock().await;
        state.files.insert(
            "a.md".to_string(),
            super::cache::CachedFileMeta {
                content_hash: "aaa".to_string(),
                last_synced_at: 0,
                size: 0,
            },
        );
        state.files.insert(
            "b.md".to_string(),
            super::cache::CachedFileMeta {
                content_hash: "bbb".to_string(),
                last_synced_at: 0,
                size: 0,
            },
        );
    }

    let tool = tool_with_cache(cache);
    let result = tool
        .execute(
            serde_json::json!({"action": "search", "query": "Rust"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("a.md"));
    assert!(result.content.contains("Rust programming"));
    assert!(!result.content.contains("b.md"));
}

#[tokio::test]
async fn test_search_case_insensitive() {
    let (_server, tmp, cache) = setup().await;

    std::fs::write(tmp.path().join("note.md"), "IMPORTANT: do this").unwrap();
    {
        let mut state = cache.state.lock().await;
        state.files.insert(
            "note.md".to_string(),
            super::cache::CachedFileMeta {
                content_hash: "x".to_string(),
                last_synced_at: 0,
                size: 0,
            },
        );
    }

    let tool = tool_with_cache(cache);
    let result = tool
        .execute(
            serde_json::json!({"action": "search", "query": "important"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("IMPORTANT"));
}

// --- List tests ---

#[tokio::test]
async fn test_list_empty_cache() {
    let (_server, _tmp, cache) = setup().await;
    let tool = tool_with_cache(cache);
    let result = tool
        .execute(
            serde_json::json!({"action": "list"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("No notes"));
}

#[tokio::test]
async fn test_list_all_files() {
    let (_server, _tmp, cache) = setup().await;
    {
        let mut state = cache.state.lock().await;
        for name in &["Daily/2025-01-01.md", "Daily/2025-01-02.md", "README.md"] {
            state.files.insert(
                name.to_string(),
                super::cache::CachedFileMeta {
                    content_hash: "x".to_string(),
                    last_synced_at: 0,
                    size: 0,
                },
            );
        }
    }

    let tool = tool_with_cache(cache);
    let result = tool
        .execute(
            serde_json::json!({"action": "list"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("3 notes"));
    assert!(result.content.contains("Daily/2025-01-01.md"));
    assert!(result.content.contains("README.md"));
}

#[tokio::test]
async fn test_list_with_folder_filter() {
    let (_server, _tmp, cache) = setup().await;
    {
        let mut state = cache.state.lock().await;
        for name in &["Daily/2025-01-01.md", "Daily/2025-01-02.md", "README.md"] {
            state.files.insert(
                name.to_string(),
                super::cache::CachedFileMeta {
                    content_hash: "x".to_string(),
                    last_synced_at: 0,
                    size: 0,
                },
            );
        }
    }

    let tool = tool_with_cache(cache);
    let result = tool
        .execute(
            serde_json::json!({"action": "list", "folder": "Daily/"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("2 notes"));
    assert!(result.content.contains("Daily/2025-01-01.md"));
    assert!(!result.content.contains("README.md"));
}

// --- Client tests ---

#[tokio::test]
async fn test_client_is_reachable_true() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/vault/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"files": []})))
        .mount(&server)
        .await;

    let client = ObsidianApiClient::with_base_url(server.uri(), "key");
    assert!(client.is_reachable().await);
}

#[tokio::test]
async fn test_client_is_reachable_false() {
    // No server running on this port
    let client = ObsidianApiClient::with_base_url("http://127.0.0.1:1".to_string(), "key");
    assert!(!client.is_reachable().await);
}

#[tokio::test]
async fn test_client_list_files_recursive() {
    let server = MockServer::start().await;

    // Root listing
    Mock::given(method("GET"))
        .and(path("/vault/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "files": ["root.md", "Notes/"]
        })))
        .mount(&server)
        .await;

    // Subdirectory listing
    Mock::given(method("GET"))
        .and(path("/vault/Notes%2F"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "files": ["deep.md", "Sub/"]
        })))
        .mount(&server)
        .await;

    // Nested subdirectory
    Mock::given(method("GET"))
        .and(path("/vault/Notes%2FSub%2F"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "files": ["nested.md"]
        })))
        .mount(&server)
        .await;

    let client = ObsidianApiClient::with_base_url(server.uri(), "key");
    let files = client.list_files().await.unwrap();

    assert!(files.contains(&"root.md".to_string()));
    assert!(files.contains(&"Notes/deep.md".to_string()));
    assert!(files.contains(&"Notes/Sub/nested.md".to_string()));
    assert_eq!(files.len(), 3);
}

#[tokio::test]
async fn test_client_read_file() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/vault/notes%2Fhello.md"))
        .and(header("Authorization", "Bearer key"))
        .respond_with(ResponseTemplate::new(200).set_body_string("# Hello\nWorld"))
        .mount(&server)
        .await;

    let client = ObsidianApiClient::with_base_url(server.uri(), "key");
    let content = client.read_file("notes/hello.md").await.unwrap();
    assert_eq!(content, "# Hello\nWorld");
}

#[tokio::test]
async fn test_client_read_file_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/vault/missing.md"))
        .respond_with(ResponseTemplate::new(404).set_body_string("Not found"))
        .mount(&server)
        .await;

    let client = ObsidianApiClient::with_base_url(server.uri(), "key");
    let result = client.read_file("missing.md").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_client_write_file() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/vault/new.md"))
        .and(header("Authorization", "Bearer key"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let client = ObsidianApiClient::with_base_url(server.uri(), "key");
    let result = client.write_file("new.md", "# New Note").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_client_append_file() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/vault/existing.md"))
        .and(header("Authorization", "Bearer key"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let client = ObsidianApiClient::with_base_url(server.uri(), "key");
    let result = client.append_file("existing.md", "\nAppended").await;
    assert!(result.is_ok());
}

// --- Sync tests ---

#[tokio::test]
async fn test_full_sync_downloads_files() {
    let (server, _tmp, cache) = setup().await;

    // Mock listing
    Mock::given(method("GET"))
        .and(path("/vault/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "files": ["note1.md", "note2.md"]
        })))
        .mount(&server)
        .await;

    // Mock reads
    Mock::given(method("GET"))
        .and(path("/vault/note1.md"))
        .respond_with(ResponseTemplate::new(200).set_body_string("# Note 1"))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/vault/note2.md"))
        .respond_with(ResponseTemplate::new(200).set_body_string("# Note 2"))
        .mount(&server)
        .await;

    cache.full_sync().await.unwrap();

    // Files should be in cache
    assert_eq!(cache.read_cached("note1.md"), Some("# Note 1".to_string()));
    assert_eq!(cache.read_cached("note2.md"), Some("# Note 2".to_string()));

    // State should track them
    let files = cache.list_cached(None).await;
    assert_eq!(files.len(), 2);
}

#[tokio::test]
async fn test_full_sync_removes_deleted_files() {
    let (server, tmp, cache) = setup().await;

    // Pre-populate cache with a file that no longer exists remotely
    std::fs::write(tmp.path().join("old.md"), "old content").unwrap();
    {
        let mut state = cache.state.lock().await;
        state.files.insert(
            "old.md".to_string(),
            super::cache::CachedFileMeta {
                content_hash: "x".to_string(),
                last_synced_at: 0,
                size: 0,
            },
        );
    }

    // Remote only has new.md
    Mock::given(method("GET"))
        .and(path("/vault/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "files": ["new.md"]
        })))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/vault/new.md"))
        .respond_with(ResponseTemplate::new(200).set_body_string("# New"))
        .mount(&server)
        .await;

    cache.full_sync().await.unwrap();

    // old.md should be removed
    assert!(cache.read_cached("old.md").is_none());
    assert!(!tmp.path().join("old.md").exists());

    // new.md should exist
    assert_eq!(cache.read_cached("new.md"), Some("# New".to_string()));
}

#[tokio::test]
async fn test_flush_write_queue() {
    let (server, _tmp, cache) = setup().await;

    // First, queue a write while "offline"
    Mock::given(method("GET"))
        .and(path("/vault/"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    cache.write_file("queued.md", "# Queued").await.ok();

    // Verify it's queued
    {
        let queue = cache.write_queue.lock().await;
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].path, "queued.md");
    }

    // Now "come online" — reset mock
    server.reset().await;

    Mock::given(method("GET"))
        .and(path("/vault/queued.md"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    Mock::given(method("PUT"))
        .and(path("/vault/queued.md"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    cache.flush_write_queue().await.unwrap();

    // Queue should be empty
    let queue = cache.write_queue.lock().await;
    assert!(queue.is_empty());
}

// --- Cache utility tests ---

#[test]
fn test_safe_vault_name() {
    assert_eq!(super::cache::safe_vault_name("MyVault"), "MyVault");
    assert_eq!(super::cache::safe_vault_name("My Vault!"), "My_Vault_");
    assert_eq!(
        super::cache::safe_vault_name("vault-name_123"),
        "vault-name_123"
    );
}

#[test]
fn test_content_hash_deterministic() {
    let h1 = super::cache::content_hash("hello world");
    let h2 = super::cache::content_hash("hello world");
    assert_eq!(h1, h2);
    assert_eq!(h1.len(), 64); // SHA-256 hex
}

#[test]
fn test_content_hash_different_inputs() {
    let h1 = super::cache::content_hash("hello");
    let h2 = super::cache::content_hash("world");
    assert_ne!(h1, h2);
}
