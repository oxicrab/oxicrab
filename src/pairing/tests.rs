use super::*;

#[test]
fn test_generate_code_format() {
    let code = PairingStore::generate_code();
    assert_eq!(code.len(), CODE_LENGTH);
    for c in code.chars() {
        assert!(
            CODE_ALPHABET.contains(&(c as u8)),
            "invalid char in code: {}",
            c
        );
    }
}

#[test]
fn test_request_and_approve() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

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
    let dir = tempfile::tempdir().unwrap();
    let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

    store
        .request_pairing("telegram", "user123")
        .unwrap()
        .unwrap();
    let result = store.approve("BADCODE1").unwrap();
    assert!(result.is_none());
}

#[test]
fn test_revoke() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

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
    let dir = tempfile::tempdir().unwrap();
    let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

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
fn test_persistence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();

    // Create store, add a pairing
    {
        let mut store = PairingStore::with_dir(path.clone()).unwrap();
        let code = store.request_pairing("slack", "user789").unwrap().unwrap();
        store.approve(&code).unwrap();
    }

    // Reload and check
    {
        let store = PairingStore::with_dir(path).unwrap();
        assert!(store.is_paired("slack", "user789"));
    }
}

#[test]
fn test_list_pending() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

    store.request_pairing("telegram", "user1").unwrap();
    store.request_pairing("telegram", "user2").unwrap();

    let pending = store.list_pending();
    assert_eq!(pending.len(), 2);
}

#[test]
fn test_max_pending_per_channel() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

    for i in 0..MAX_PENDING_PER_CHANNEL {
        let result = store
            .request_pairing("telegram", &format!("user{}", i))
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
    let dir = tempfile::tempdir().unwrap();
    let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

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
    let dir = tempfile::tempdir().unwrap();
    let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

    store
        .request_pairing("telegram", "user123")
        .unwrap()
        .unwrap();

    // Try a bad code
    let result = store.approve_with_client("BADCODE1", "admin1").unwrap();
    assert!(result.is_none());

    // Should have recorded one failed attempt
    assert_eq!(store.failed_attempts["admin1"].len(), 1);
}

#[test]
fn test_approve_with_client_locks_out_after_max_failures() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

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
    let dir = tempfile::tempdir().unwrap();
    let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

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
    let dir = tempfile::tempdir().unwrap();
    let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

    store
        .request_pairing("telegram", "user123")
        .unwrap()
        .unwrap();

    // approve() delegates to approve_with_client("default")
    let _ = store.approve("BADCODE1");
    assert!(store.failed_attempts.contains_key("default"));
}

#[test]
fn test_case_insensitive_code_matching() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PairingStore::with_dir(dir.path().to_path_buf()).unwrap();

    let code = store
        .request_pairing("telegram", "user123")
        .unwrap()
        .unwrap();

    // Approve with lowercase version of the code
    let lower = code.to_lowercase();
    let result = store.approve(&lower).unwrap();
    assert!(result.is_some());
}
