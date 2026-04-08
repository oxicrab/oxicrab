use super::super::MemoryDB;

#[test]
fn test_add_and_check_paired_sender() {
    let db = MemoryDB::new(":memory:").unwrap();

    assert!(!db.is_sender_paired("telegram", "user1").unwrap());
    assert!(db.add_paired_sender("telegram", "user1").unwrap());
    assert!(db.is_sender_paired("telegram", "user1").unwrap());

    // Duplicate insert returns false
    assert!(!db.add_paired_sender("telegram", "user1").unwrap());
}

#[test]
fn test_remove_paired_sender() {
    let db = MemoryDB::new(":memory:").unwrap();

    db.add_paired_sender("telegram", "user1").unwrap();
    assert!(db.remove_paired_sender("telegram", "user1").unwrap());
    assert!(!db.is_sender_paired("telegram", "user1").unwrap());

    // Removing non-existent returns false
    assert!(!db.remove_paired_sender("telegram", "user1").unwrap());
}

#[test]
fn test_list_paired_senders() {
    let db = MemoryDB::new(":memory:").unwrap();

    db.add_paired_sender("telegram", "alice").unwrap();
    db.add_paired_sender("telegram", "bob").unwrap();
    db.add_paired_sender("discord", "charlie").unwrap();

    let tg = db.list_paired_senders("telegram").unwrap();
    assert_eq!(tg.len(), 2);
    assert!(tg.contains(&"alice".to_string()));
    assert!(tg.contains(&"bob".to_string()));

    let dc = db.list_paired_senders("discord").unwrap();
    assert_eq!(dc.len(), 1);
}

#[test]
fn test_count_paired_senders() {
    let db = MemoryDB::new(":memory:").unwrap();

    assert_eq!(db.count_paired_senders().unwrap(), 0);
    db.add_paired_sender("telegram", "a").unwrap();
    db.add_paired_sender("discord", "b").unwrap();
    assert_eq!(db.count_paired_senders().unwrap(), 2);
}

#[test]
fn test_list_all_paired_channels() {
    let db = MemoryDB::new(":memory:").unwrap();

    db.add_paired_sender("discord", "user1").unwrap();
    db.add_paired_sender("discord", "user2").unwrap();
    db.add_paired_sender("telegram", "user3").unwrap();

    let all = db.list_all_paired_channels().unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].0, "discord");
    assert_eq!(all[0].1.len(), 2);
    assert_eq!(all[1].0, "telegram");
    assert_eq!(all[1].1.len(), 1);
}

#[test]
fn test_pending_request_lifecycle() {
    let db = MemoryDB::new(":memory:").unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    db.add_pending_request("telegram", "user1", "ABCD1234", now)
        .unwrap();

    let pending = db.get_pending_for_sender("telegram", "user1", 900).unwrap();
    assert!(pending.is_some());
    let p = pending.unwrap();
    assert_eq!(p.code, "ABCD1234");

    assert_eq!(db.count_pending_for_channel("telegram", 900).unwrap(), 1);

    assert!(db.remove_pending("ABCD1234").unwrap());
    assert!(!db.remove_pending("ABCD1234").unwrap());

    assert_eq!(db.count_pending_for_channel("telegram", 900).unwrap(), 0);
}

#[test]
fn test_get_all_pending() {
    let db = MemoryDB::new(":memory:").unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    db.add_pending_request("telegram", "u1", "CODE1111", now)
        .unwrap();
    db.add_pending_request("discord", "u2", "CODE2222", now)
        .unwrap();
    // Expired request
    db.add_pending_request("slack", "u3", "CODE3333", now.saturating_sub(1000))
        .unwrap();

    let all = db.get_all_pending(900).unwrap();
    assert_eq!(all.len(), 2);
}

#[test]
fn test_cleanup_expired_pending() {
    let db = MemoryDB::new(":memory:").unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    db.add_pending_request("telegram", "u1", "FRESH111", now)
        .unwrap();
    db.add_pending_request("telegram", "u2", "OLD22222", now.saturating_sub(1000))
        .unwrap();

    let cleaned = db.cleanup_expired_pending(900).unwrap();
    assert_eq!(cleaned, 1);

    let all = db.get_all_pending(900).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].code, "FRESH111");
}

#[test]
fn test_failed_attempts() {
    let db = MemoryDB::new(":memory:").unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    db.record_failed_attempt("admin1", now).unwrap();
    db.record_failed_attempt("admin1", now).unwrap();
    db.record_failed_attempt("admin2", now).unwrap();

    assert_eq!(db.count_recent_failed_attempts("admin1", 300).unwrap(), 2);
    assert_eq!(db.count_recent_failed_attempts("admin2", 300).unwrap(), 1);
    assert_eq!(db.count_recent_failed_attempts("admin3", 300).unwrap(), 0);
}

#[test]
fn test_cleanup_old_failed_attempts() {
    let db = MemoryDB::new(":memory:").unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    db.record_failed_attempt("admin1", now).unwrap();
    db.record_failed_attempt("admin1", now.saturating_sub(600))
        .unwrap();

    let cleaned = db.cleanup_old_failed_attempts(300).unwrap();
    assert_eq!(cleaned, 1);
    assert_eq!(db.count_recent_failed_attempts("admin1", 300).unwrap(), 1);
}

#[test]
fn test_evict_oldest_lockout_client() {
    let db = MemoryDB::new(":memory:").unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Add 3 clients with different timestamps
    db.record_failed_attempt("oldest", now.saturating_sub(100))
        .unwrap();
    db.record_failed_attempt("middle", now.saturating_sub(50))
        .unwrap();
    db.record_failed_attempt("newest", now).unwrap();

    // Evict if more than 2 clients
    db.evict_oldest_lockout_client(2).unwrap();

    // "oldest" should be evicted
    assert_eq!(db.count_recent_failed_attempts("oldest", 300).unwrap(), 0);
    assert_eq!(db.count_recent_failed_attempts("middle", 300).unwrap(), 1);
    assert_eq!(db.count_recent_failed_attempts("newest", 300).unwrap(), 1);
}
