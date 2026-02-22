use super::*;

#[test]
fn test_claude_code_headers_has_required_keys() {
    let headers = claude_code_headers();
    let keys: Vec<&str> = headers.iter().map(|(k, _)| *k).collect();
    assert!(keys.contains(&"anthropic-version"));
    assert!(keys.contains(&"content-type"));
    assert!(keys.contains(&"user-agent"));
    assert!(keys.contains(&"x-app"));
}

#[test]
fn test_claude_code_headers_anthropic_version() {
    let headers = claude_code_headers();
    let version = headers
        .iter()
        .find(|(k, _)| *k == "anthropic-version")
        .map(|(_, v)| *v);
    assert_eq!(version, Some("2023-06-01"));
}

#[test]
fn test_new_default_model() {
    let provider =
        AnthropicOAuthProvider::new("access".to_string(), "refresh".to_string(), 0, None, None)
            .unwrap();
    assert_eq!(provider.default_model, "claude-opus-4-6");
}

#[test]
fn test_new_strips_anthropic_prefix() {
    let provider = AnthropicOAuthProvider::new(
        "access".to_string(),
        "refresh".to_string(),
        0,
        Some("anthropic/claude-sonnet-4-5-20250929".to_string()),
        None,
    )
    .unwrap();
    assert_eq!(provider.default_model, "claude-sonnet-4-5-20250929");
}

#[test]
fn test_new_no_prefix_preserved() {
    let provider = AnthropicOAuthProvider::new(
        "access".to_string(),
        "refresh".to_string(),
        0,
        Some("claude-opus-4-6".to_string()),
        None,
    )
    .unwrap();
    assert_eq!(provider.default_model, "claude-opus-4-6");
}

#[test]
fn test_from_credentials_file_missing() {
    let result = AnthropicOAuthProvider::from_credentials_file(
        std::path::Path::new("/nonexistent/path.json"),
        None,
    )
    .unwrap();
    assert!(result.is_none());
}

#[test]
fn test_from_credentials_file_valid() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("creds.json");
    std::fs::write(
        &path,
        r#"{"access_token":"tok123","refresh_token":"ref456","expires_at":9999999999999}"#,
    )
    .unwrap();

    let result =
        AnthropicOAuthProvider::from_credentials_file(&path, Some("my-model".to_string())).unwrap();
    assert!(result.is_some());
    let provider = result.unwrap();
    assert_eq!(provider.default_model, "my-model");
}

#[test]
fn test_from_credentials_file_missing_access_token() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad_creds.json");
    std::fs::write(&path, r#"{"refresh_token":"ref456"}"#).unwrap();

    let result = AnthropicOAuthProvider::from_credentials_file(&path, None);
    assert!(result.is_err());
}

#[test]
fn test_from_credentials_file_invalid_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corrupt.json");
    std::fs::write(&path, "not json at all").unwrap();

    let result = AnthropicOAuthProvider::from_credentials_file(&path, None);
    assert!(result.is_err());
}

#[test]
fn test_load_cached_tokens_fresher_tokens_applied() {
    let dir = tempfile::tempdir().unwrap();
    let cache_path = dir.path().join("oauth_cache.json");
    std::fs::write(
        &cache_path,
        r#"{"access_token":"new_access","refresh_token":"new_refresh","expires_at":9999999999999}"#,
    )
    .unwrap();

    let provider = AnthropicOAuthProvider::new(
        "old_access".to_string(),
        "old_refresh".to_string(),
        1000, // very old
        None,
        Some(cache_path),
    )
    .unwrap();

    // Cached tokens should have been loaded since they have a later expires_at
    let token = provider.access_token.try_lock().unwrap().clone();
    assert_eq!(token, "new_access");
}

#[test]
fn test_load_cached_tokens_stale_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let cache_path = dir.path().join("oauth_cache.json");
    std::fs::write(
        &cache_path,
        r#"{"access_token":"stale_access","refresh_token":"stale_refresh","expires_at":500}"#,
    )
    .unwrap();

    let provider = AnthropicOAuthProvider::new(
        "current_access".to_string(),
        "current_refresh".to_string(),
        1000, // newer than cache
        None,
        Some(cache_path),
    )
    .unwrap();

    // Original tokens should be preserved
    let token = provider.access_token.try_lock().unwrap().clone();
    assert_eq!(token, "current_access");
}

#[test]
fn test_from_claude_cli_missing_file() {
    // Won't find credentials at a non-existent home dir
    let result = AnthropicOAuthProvider::from_claude_cli(None);
    // Should return Ok(None) or Ok(Some(...)) depending on whether ~/.claude exists
    assert!(result.is_ok());
}

#[test]
fn test_default_model_trait() {
    let provider = AnthropicOAuthProvider::new(
        "tok".to_string(),
        "ref".to_string(),
        0,
        Some("custom-model".to_string()),
        None,
    )
    .unwrap();
    assert_eq!(provider.default_model(), "custom-model");
}
