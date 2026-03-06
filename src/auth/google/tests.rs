use super::*;

fn make_creds(expiry: Option<u64>) -> GoogleCredentials {
    GoogleCredentials {
        token: "tok_test".to_string(),
        refresh_token: Some("rt_test".to_string()),
        token_uri: "https://oauth2.googleapis.com/token".to_string(),
        client_id: "cid".to_string(),
        client_secret: "csec".to_string(),
        scopes: DEFAULT_SCOPES.iter().map(ToString::to_string).collect(),
        expiry,
    }
}

// -- extract_param_from_request ----

#[test]
fn test_extract_param_basic() {
    let req = "GET /?code=abc123&state=xyz HTTP/1.1\r\nHost: localhost\r\n";
    assert_eq!(
        extract_param_from_request(req, "code"),
        Some("abc123".to_string())
    );
    assert_eq!(
        extract_param_from_request(req, "state"),
        Some("xyz".to_string())
    );
}

#[test]
fn test_extract_param_missing() {
    let req = "GET /?code=abc123 HTTP/1.1\r\n";
    assert_eq!(extract_param_from_request(req, "state"), None);
}

#[test]
fn test_extract_param_empty_request() {
    assert_eq!(extract_param_from_request("", "code"), None);
}

#[test]
fn test_extract_param_no_query_string() {
    let req = "GET / HTTP/1.1\r\n";
    assert_eq!(extract_param_from_request(req, "code"), None);
}

#[test]
fn test_extract_param_url_encoded_value() {
    let req = "GET /?code=4%2F0Atest%26more HTTP/1.1\r\n";
    assert_eq!(
        extract_param_from_request(req, "code"),
        Some("4/0Atest&more".to_string())
    );
}

// -- extract_code_from_request -----

#[test]
fn test_extract_code_basic() {
    let req = "GET /?code=AUTH_CODE_HERE&scope=email HTTP/1.1\r\nHost: localhost\r\n";
    assert_eq!(extract_code_from_request(req).unwrap(), "AUTH_CODE_HERE");
}

#[test]
fn test_extract_code_missing() {
    let req = "GET /?state=csrf_token HTTP/1.1\r\n";
    assert!(extract_code_from_request(req).is_err());
}

#[test]
fn test_extract_code_url_encoded() {
    let req = "GET /?code=4%2F0AfJohXl HTTP/1.1\r\n";
    assert_eq!(extract_code_from_request(req).unwrap(), "4/0AfJohXl");
}

#[test]
fn test_extract_code_empty_request() {
    assert!(extract_code_from_request("").is_err());
}

// -- GoogleCredentials::is_valid ---

#[test]
fn test_is_valid_future_expiry() {
    let future = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    let creds = make_creds(Some(future));
    assert!(creds.is_valid());
}

#[test]
fn test_is_valid_past_expiry() {
    let creds = make_creds(Some(1000));
    assert!(!creds.is_valid());
}

#[test]
fn test_is_valid_no_expiry() {
    let creds = make_creds(None);
    assert!(!creds.is_valid());
}

// -- load / save credentials round-trip (file-based) ----

#[test]
fn test_save_and_load_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tokens.json");
    let creds = make_creds(Some(9_999_999_999));

    save_credentials(&creds, &path, None).unwrap();
    let loaded = load_credentials(
        &path,
        &["https://www.googleapis.com/auth/gmail.modify"],
        None,
    )
    .unwrap();
    let loaded = loaded.expect("should load credentials");
    assert_eq!(loaded.token, "tok_test");
    assert_eq!(loaded.refresh_token, Some("rt_test".to_string()));
}

#[test]
fn test_load_missing_file_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nonexistent.json");
    let loaded = load_credentials(&path, &["scope"], None).unwrap();
    assert!(loaded.is_none());
}

#[test]
fn test_load_scope_mismatch_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tokens.json");
    let creds = make_creds(Some(9_999_999_999));
    save_credentials(&creds, &path, None).unwrap();

    // Request a scope the saved credentials don't have
    let loaded = load_credentials(
        &path,
        &["https://www.googleapis.com/auth/drive.readonly"],
        None,
    )
    .unwrap();
    assert!(loaded.is_none());
}

#[cfg(unix)]
#[test]
fn test_save_sets_restricted_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tokens.json");
    let creds = make_creds(Some(9_999_999_999));
    save_credentials(&creds, &path, None).unwrap();

    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

// -- has_valid_credentials -----

#[test]
fn test_has_valid_with_valid_token() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tokens.json");
    let future = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    let creds = make_creds(Some(future));
    save_credentials(&creds, &path, None).unwrap();

    assert!(has_valid_credentials("cid", "csec", None, Some(&path)));
}

#[test]
fn test_has_valid_with_expired_but_refresh_token() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tokens.json");
    let creds = make_creds(Some(1000)); // expired
    save_credentials(&creds, &path, None).unwrap();

    // Should return true because refresh_token is present
    assert!(has_valid_credentials("cid", "csec", None, Some(&path)));
}

#[test]
fn test_has_valid_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nope.json");
    assert!(!has_valid_credentials("cid", "csec", None, Some(&path)));
}

// -- DB-based load / save round-trip ----

#[test]
fn test_save_and_load_db_round_trip() {
    let db = Arc::new(MemoryDB::new(":memory:").unwrap());
    let creds = make_creds(Some(9_999_999_999));
    let dummy_path = std::path::Path::new("/nonexistent");

    save_credentials(&creds, dummy_path, Some(&db)).unwrap();
    let loaded = load_credentials(
        dummy_path,
        &["https://www.googleapis.com/auth/gmail.modify"],
        Some(&db),
    )
    .unwrap();
    let loaded = loaded.expect("should load credentials from DB");
    assert_eq!(loaded.token, "tok_test");
    assert_eq!(loaded.refresh_token, Some("rt_test".to_string()));
    assert_eq!(loaded.client_id, "cid");
    assert_eq!(loaded.client_secret, "csec");
    assert_eq!(loaded.expiry, Some(9_999_999_999));
}

#[test]
fn test_db_scope_mismatch_returns_none() {
    let db = Arc::new(MemoryDB::new(":memory:").unwrap());
    let creds = make_creds(Some(9_999_999_999));
    let dummy_path = std::path::Path::new("/nonexistent");

    save_credentials(&creds, dummy_path, Some(&db)).unwrap();
    let loaded = load_credentials(
        dummy_path,
        &["https://www.googleapis.com/auth/drive.readonly"],
        Some(&db),
    )
    .unwrap();
    assert!(loaded.is_none());
}

#[test]
fn test_db_empty_returns_none() {
    let db = Arc::new(MemoryDB::new(":memory:").unwrap());
    let dummy_path = std::path::Path::new("/nonexistent");
    let loaded = load_credentials(dummy_path, &["scope"], Some(&db)).unwrap();
    assert!(loaded.is_none());
}
