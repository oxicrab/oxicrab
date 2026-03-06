use super::*;
use crate::agent::memory::memory_db::MemoryDB;

fn test_db() -> Arc<MemoryDB> {
    Arc::new(MemoryDB::new(":memory:").expect("test db"))
}

#[test]
fn test_generate_code_format() {
    let code = PairingStore::generate_code();
    assert_eq!(code.len(), CODE_LENGTH);
    for c in code.chars() {
        assert!(
            CODE_ALPHABET.contains(&(c as u8)),
            "invalid char in code: {c}"
        );
    }
}

#[test]
fn test_request_and_approve() {
    let store = PairingStore::new(test_db());

    assert!(!store.is_paired("telegram", "user123"));

    let code = store
        .request_pairing("telegram", "user123")
        .unwrap()
        .unwrap();
    assert_eq!(code.len(), CODE_LENGTH);

    // Approve with the code
    let result = store.approve(&code).unwrap();
    assert!(result.is_some());
    let (channel, sender) = result.unwrap();
    assert_eq!(channel, "telegram");
    assert_eq!(sender, "user123");

    // Now paired
    assert!(store.is_paired("telegram", "user123"));
}

#[test]
fn test_approve_invalid_code() {
    let store = PairingStore::new(test_db());

    store
        .request_pairing("telegram", "user123")
        .unwrap()
        .unwrap();
    let result = store.approve("BADCODE1").unwrap();
    assert!(result.is_none());
}

#[test]
fn test_revoke() {
    let store = PairingStore::new(test_db());

    let code = store
        .request_pairing("discord", "user456")
        .unwrap()
        .unwrap();
    store.approve(&code).unwrap();
    assert!(store.is_paired("discord", "user456"));

    let revoked = store.revoke("discord", "user456").unwrap();
    assert!(revoked);
    assert!(!store.is_paired("discord", "user456"));
}

#[test]
fn test_duplicate_request_returns_same_code() {
    let store = PairingStore::new(test_db());

    let code1 = store
        .request_pairing("telegram", "user123")
        .unwrap()
        .unwrap();
    let code2 = store
        .request_pairing("telegram", "user123")
        .unwrap()
        .unwrap();
    assert_eq!(code1, code2);
}

#[test]
fn test_persistence_via_db() {
    let db = test_db();

    // Create store, add a pairing
    {
        let store = PairingStore::new(db.clone());
        let code = store.request_pairing("slack", "user789").unwrap().unwrap();
        store.approve(&code).unwrap();
    }

    // New store with same DB — should still be paired
    {
        let store = PairingStore::new(db);
        assert!(store.is_paired("slack", "user789"));
    }
}

#[test]
fn test_list_pending() {
    let store = PairingStore::new(test_db());

    store.request_pairing("telegram", "user1").unwrap();
    store.request_pairing("telegram", "user2").unwrap();

    let pending = store.list_pending();
    assert_eq!(pending.len(), 2);
}

#[test]
fn test_max_pending_per_channel() {
    let store = PairingStore::new(test_db());

    for i in 0..MAX_PENDING_PER_CHANNEL {
        let result = store
            .request_pairing("telegram", &format!("user{i}"))
            .unwrap();
        assert!(result.is_some());
    }

    // Next request should be rate-limited
    let result = store.request_pairing("telegram", "overflow_user").unwrap();
    assert!(result.is_none());

    // But a different channel should still work
    let result = store.request_pairing("discord", "overflow_user").unwrap();
    assert!(result.is_some());
}

#[test]
fn test_approve_with_client_success() {
    let store = PairingStore::new(test_db());

    let code = store
        .request_pairing("telegram", "user123")
        .unwrap()
        .unwrap();
    let result = store.approve_with_client(&code, "admin1").unwrap();
    assert!(result.is_some());
    let (channel, sender) = result.unwrap();
    assert_eq!(channel, "telegram");
    assert_eq!(sender, "user123");
    assert!(store.is_paired("telegram", "user123"));
}

#[test]
fn test_approve_with_client_records_failed_attempts() {
    let db = test_db();
    let store = PairingStore::new(db.clone());

    store
        .request_pairing("telegram", "user123")
        .unwrap()
        .unwrap();

    // Try a bad code
    let result = store.approve_with_client("BADCODE1", "admin1").unwrap();
    assert!(result.is_none());

    // Should have recorded one failed attempt
    let count = db
        .count_recent_failed_attempts("admin1", FAILED_ATTEMPT_WINDOW_SECS)
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_approve_with_client_locks_out_after_max_failures() {
    let store = PairingStore::new(test_db());

    store
        .request_pairing("telegram", "user123")
        .unwrap()
        .unwrap();

    // Exhaust the failure budget
    for _ in 0..MAX_FAILED_ATTEMPTS {
        let _ = store.approve_with_client("BADCODE1", "admin1");
    }

    // Next attempt should bail
    let result = store.approve_with_client("BADCODE1", "admin1");
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("too many failed approval attempts")
    );
}

#[test]
fn test_approve_with_client_separate_limits_per_client() {
    let store = PairingStore::new(test_db());

    let code = store
        .request_pairing("telegram", "user123")
        .unwrap()
        .unwrap();

    // Exhaust admin1's budget
    for _ in 0..MAX_FAILED_ATTEMPTS {
        let _ = store.approve_with_client("BADCODE1", "admin1");
    }

    // admin1 is locked out
    assert!(store.approve_with_client("BADCODE1", "admin1").is_err());

    // admin2 can still approve successfully
    let result = store.approve_with_client(&code, "admin2").unwrap();
    assert!(result.is_some());
}

#[test]
fn test_approve_default_uses_default_client() {
    let db = test_db();
    let store = PairingStore::new(db.clone());

    store
        .request_pairing("telegram", "user123")
        .unwrap()
        .unwrap();

    // approve() delegates to approve_with_client("default")
    let _ = store.approve("BADCODE1");
    let count = db
        .count_recent_failed_attempts("default", FAILED_ATTEMPT_WINDOW_SECS)
        .unwrap();
    assert!(count > 0);
}

#[test]
fn test_case_insensitive_code_matching() {
    let store = PairingStore::new(test_db());

    let code = store
        .request_pairing("telegram", "user123")
        .unwrap()
        .unwrap();

    // Approve with lowercase version of the code
    let lower = code.to_lowercase();
    let result = store.approve(&lower).unwrap();
    assert!(result.is_some());
}
