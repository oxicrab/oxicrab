use super::*;

#[test]
fn test_empty_allow_list_denies_all() {
    // No pairing store file exists in test env, so this is pure deny
    assert!(!check_allowed_sender("anyone", &[], "test"));
}

#[test]
fn test_wildcard_allows_all() {
    let list = vec!["*".to_string()];
    assert!(check_allowed_sender("anyone", &list, "test"));
    assert!(check_allowed_sender("other_user", &list, "test"));
}

#[test]
fn test_exact_match_allowed() {
    let list = vec!["alice".to_string(), "bob".to_string()];
    assert!(check_allowed_sender("alice", &list, "test"));
    assert!(check_allowed_sender("bob", &list, "test"));
}

#[test]
fn test_non_matching_sender_rejected() {
    let list = vec!["alice".to_string()];
    assert!(!check_allowed_sender("eve", &list, "test"));
}

#[test]
fn test_phone_number_normalization() {
    let list = vec!["1234567890".to_string()];
    // With leading + should still match
    assert!(check_allowed_sender("+1234567890", &list, "test"));
}

#[test]
fn test_allow_list_with_plus_prefix() {
    let list = vec!["+1234567890".to_string()];
    // Without leading + should still match
    assert!(check_allowed_sender("1234567890", &list, "test"));
}

#[test]
fn test_no_substring_match() {
    let list = vec!["alice".to_string()];
    assert!(!check_allowed_sender("alice123", &list, "test"));
    assert!(!check_allowed_sender("xalice", &list, "test"));
}

#[test]
fn test_pairing_store_fallback() {
    // Set up a temp pairing store
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: test runs single-threaded; env var is restored before returning
    unsafe { std::env::set_var("OXICRAB_HOME", dir.path().as_os_str()) };
    let pairing_dir = dir.path().join("pairing");
    std::fs::create_dir_all(&pairing_dir).unwrap();
    std::fs::write(
        pairing_dir.join("telegram-allowlist.json"),
        r#"{"senders":["user789"]}"#,
    )
    .unwrap();

    // Empty allowFrom but paired → allowed
    assert!(check_allowed_sender("user789", &[], "telegram"));
    // Not in allowFrom and not paired → denied
    assert!(!check_allowed_sender("unknown", &[], "telegram"));

    // SAFETY: restoring env var after test
    unsafe { std::env::remove_var("OXICRAB_HOME") };
}

#[test]
fn test_normalize_strips_plus() {
    assert_eq!(normalize_sender_id("+1234"), "1234");
    assert_eq!(normalize_sender_id("1234"), "1234");
    assert_eq!(normalize_sender_id("+++abc"), "abc");
}

#[test]
fn test_dm_access_open_allows_all() {
    assert!(matches!(
        check_dm_access("anyone", &[], "test", &crate::config::DmPolicy::Open),
        DmCheckResult::Allowed
    ));
}

#[test]
fn test_dm_access_allowlist_denies_unknown() {
    assert!(matches!(
        check_dm_access("unknown", &[], "test", &crate::config::DmPolicy::Allowlist),
        DmCheckResult::Denied
    ));
}

#[test]
fn test_dm_access_allowlist_allows_known() {
    let list = vec!["alice".to_string()];
    assert!(matches!(
        check_dm_access("alice", &list, "test", &crate::config::DmPolicy::Allowlist),
        DmCheckResult::Allowed
    ));
}

#[test]
fn test_dm_access_pairing_returns_code() {
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: test runs single-threaded
    unsafe { std::env::set_var("OXICRAB_HOME", dir.path().as_os_str()) };
    std::fs::create_dir_all(dir.path().join("pairing")).unwrap();

    match check_dm_access(
        "newuser",
        &[],
        "telegram",
        &crate::config::DmPolicy::Pairing,
    ) {
        DmCheckResult::PairingRequired { code } => {
            assert_eq!(code.len(), 8);
        }
        other => panic!(
            "expected PairingRequired, got {:?}",
            match other {
                DmCheckResult::Allowed => "Allowed",
                DmCheckResult::Denied => "Denied",
                DmCheckResult::PairingRequired { .. } => unreachable!(),
            }
        ),
    }

    // SAFETY: restoring env var
    unsafe { std::env::remove_var("OXICRAB_HOME") };
}

#[test]
fn test_dm_access_pairing_allows_known() {
    let list = vec!["bob".to_string()];
    assert!(matches!(
        check_dm_access("bob", &list, "test", &crate::config::DmPolicy::Pairing),
        DmCheckResult::Allowed
    ));
}

#[test]
fn test_format_pairing_reply() {
    let reply = format_pairing_reply("telegram", "user123", "ABCD1234");
    assert!(reply.contains("user123"));
    assert!(reply.contains("ABCD1234"));
    assert!(reply.contains("oxicrab pairing approve telegram ABCD1234"));
}

#[test]
fn test_backoff_first_attempt() {
    let d = exponential_backoff_delay(0, 1, 60);
    assert!((1..=2).contains(&d), "expected 1..=2, got {d}");
}

#[test]
fn test_backoff_grows_exponentially() {
    // Each attempt should be >= base * 2^attempt (jitter adds up to 25%)
    let d0 = exponential_backoff_delay(0, 2, 120);
    assert!((2..=3).contains(&d0), "attempt 0: expected 2..=3, got {d0}");
    let d1 = exponential_backoff_delay(1, 2, 120);
    assert!((4..=5).contains(&d1), "attempt 1: expected 4..=5, got {d1}");
    let d2 = exponential_backoff_delay(2, 2, 120);
    assert!(
        (8..=10).contains(&d2),
        "attempt 2: expected 8..=10, got {d2}"
    );
    let d3 = exponential_backoff_delay(3, 2, 120);
    assert!(
        (16..=20).contains(&d3),
        "attempt 3: expected 16..=20, got {d3}"
    );
}

#[test]
fn test_backoff_capped_at_max() {
    let d = exponential_backoff_delay(10, 2, 60);
    assert!((60..=75).contains(&d), "expected 60..=75, got {d}");
    let d = exponential_backoff_delay(20, 2, 60);
    assert!((60..=75).contains(&d), "expected 60..=75, got {d}");
}

#[test]
fn test_backoff_large_attempt_no_overflow() {
    // 2^40 overflows u64 multiplication — should cap at max without panic
    let d = exponential_backoff_delay(40, 5, 300);
    assert!((300..=375).contains(&d), "expected 300..=375, got {d}");
}

#[test]
fn test_backoff_zero_base() {
    let d = exponential_backoff_delay(5, 0, 60);
    assert_eq!(d, 0);
}

#[test]
fn test_backoff_max_less_than_base() {
    // max < base: should still cap
    let d = exponential_backoff_delay(0, 10, 5);
    assert!((5..=7).contains(&d), "expected 5..=7, got {d}");
}

#[test]
fn test_normalize_empty_string() {
    assert_eq!(normalize_sender_id(""), "");
}

#[test]
fn test_normalize_no_plus() {
    assert_eq!(normalize_sender_id("user@example.com"), "user@example.com");
}

#[test]
fn test_format_pairing_reply_contains_approve_command() {
    let reply = format_pairing_reply("discord", "U123", "XYZW9876");
    assert!(reply.contains("oxicrab pairing approve discord XYZW9876"));
    assert!(reply.contains("U123"));
    assert!(reply.contains("XYZW9876"));
}
