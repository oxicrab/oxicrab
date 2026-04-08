use super::super::MemoryDB;

#[test]
fn test_save_and_load_oauth_token() {
    let db = MemoryDB::new(":memory:").unwrap();

    db.save_oauth_token("anthropic", "access_tok", Some("refresh_tok"), 99999, None)
        .unwrap();

    let row = db.load_oauth_token("anthropic").unwrap().unwrap();
    assert_eq!(row.provider, "anthropic");
    assert_eq!(row.access_token, "access_tok");
    assert_eq!(row.refresh_token, Some("refresh_tok".to_string()));
    assert_eq!(row.expires_at, 99999);
    assert!(row.extra_json.is_none());
}

#[test]
fn test_save_oauth_token_upsert() {
    let db = MemoryDB::new(":memory:").unwrap();

    db.save_oauth_token("anthropic", "old_tok", Some("old_ref"), 1000, None)
        .unwrap();
    db.save_oauth_token("anthropic", "new_tok", Some("new_ref"), 2000, None)
        .unwrap();

    let row = db.load_oauth_token("anthropic").unwrap().unwrap();
    assert_eq!(row.access_token, "new_tok");
    assert_eq!(row.refresh_token, Some("new_ref".to_string()));
    assert_eq!(row.expires_at, 2000);
}

#[test]
fn test_load_oauth_token_not_found() {
    let db = MemoryDB::new(":memory:").unwrap();
    assert!(db.load_oauth_token("nonexistent").unwrap().is_none());
}

#[test]
fn test_delete_oauth_token() {
    let db = MemoryDB::new(":memory:").unwrap();

    db.save_oauth_token("google", "tok", None, 5000, Some(r#"{"client_id":"cid"}"#))
        .unwrap();
    assert!(db.delete_oauth_token("google").unwrap());
    assert!(!db.delete_oauth_token("google").unwrap());
    assert!(db.load_oauth_token("google").unwrap().is_none());
}

#[test]
fn test_save_oauth_token_with_extra_json() {
    let db = MemoryDB::new(":memory:").unwrap();
    let extra = r#"{"client_id":"cid","client_secret":"csec","token_uri":"https://example.com","scopes":["a","b"]}"#;

    db.save_oauth_token("google", "tok", Some("ref"), 9000, Some(extra))
        .unwrap();

    let row = db.load_oauth_token("google").unwrap().unwrap();
    assert_eq!(row.extra_json, Some(extra.to_string()));
}

#[test]
fn test_save_oauth_token_no_refresh() {
    let db = MemoryDB::new(":memory:").unwrap();

    db.save_oauth_token("test", "access", None, 1234, None)
        .unwrap();

    let row = db.load_oauth_token("test").unwrap().unwrap();
    assert!(row.refresh_token.is_none());
}

#[test]
fn test_multiple_providers() {
    let db = MemoryDB::new(":memory:").unwrap();

    db.save_oauth_token("anthropic", "ant_tok", Some("ant_ref"), 1000, None)
        .unwrap();
    db.save_oauth_token("google", "goo_tok", Some("goo_ref"), 2000, None)
        .unwrap();

    let ant = db.load_oauth_token("anthropic").unwrap().unwrap();
    let goo = db.load_oauth_token("google").unwrap().unwrap();
    assert_eq!(ant.access_token, "ant_tok");
    assert_eq!(goo.access_token, "goo_tok");
}
