use super::*;
use oxicrab_core::config::schema::{DenyByDefaultList, DmPolicy};

/// Serialize tests that mutate the `OXICRAB_HOME` env var to prevent races.
static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// RAII guard that removes an env var on drop, ensuring cleanup even on panic.
struct EnvVarGuard(&'static str);

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        // SAFETY: serialized by ENV_MUTEX
        unsafe { std::env::remove_var(self.0) };
    }
}

fn list(entries: &[&str]) -> DenyByDefaultList {
    DenyByDefaultList::new(entries.iter().map(ToString::to_string).collect())
}

// --- DenyByDefaultList unit tests ---

#[test]
fn test_deny_by_default_empty_denies_all() {
    let l = DenyByDefaultList::default();
    assert!(!l.allows("anyone"));
    assert!(!l.allows_normalized("anyone"));
}

#[test]
fn test_deny_by_default_wildcard_allows_all() {
    let l = list(&["*"]);
    assert!(l.allows("anyone"));
    assert!(l.allows("other_user"));
}

#[test]
fn test_deny_by_default_exact_match() {
    let l = list(&["alice", "bob"]);
    assert!(l.allows("alice"));
    assert!(l.allows("bob"));
    assert!(!l.allows("eve"));
}

#[test]
fn test_deny_by_default_normalized_plus_prefix() {
    let l = list(&["1234567890"]);
    assert!(l.allows_normalized("+1234567890"));
}

#[test]
fn test_deny_by_default_normalized_allow_with_plus() {
    let l = list(&["+1234567890"]);
    assert!(l.allows_normalized("1234567890"));
}

#[test]
fn test_deny_by_default_normalized_control_chars() {
    let l = list(&["username"]);
    assert!(l.allows_normalized("user\x00name"));
    assert!(l.allows_normalized("user\nname"));
}

#[test]
fn test_deny_by_default_is_empty() {
    assert!(DenyByDefaultList::default().is_empty());
    assert!(!list(&["x"]).is_empty());
}

#[test]
fn test_deny_by_default_entries() {
    let l = list(&["a", "b"]);
    assert_eq!(l.entries(), &["a".to_string(), "b".to_string()]);
}

// --- check_allowed_sender tests (with pairing fallback) ---

#[test]
fn test_empty_allow_list_denies_all() {
    // No pairing DB exists in test env, so this is pure deny
    assert!(!check_allowed_sender(
        "anyone",
        &DenyByDefaultList::default(),
        "test"
    ));
}

#[test]
fn test_wildcard_allows_all() {
    let l = list(&["*"]);
    assert!(check_allowed_sender("anyone", &l, "test"));
    assert!(check_allowed_sender("other_user", &l, "test"));
}

#[test]
fn test_exact_match_allowed() {
    let l = list(&["alice", "bob"]);
    assert!(check_allowed_sender("alice", &l, "test"));
    assert!(check_allowed_sender("bob", &l, "test"));
}

#[test]
fn test_non_matching_sender_rejected() {
    let l = list(&["alice"]);
    assert!(!check_allowed_sender("eve", &l, "test"));
}

#[test]
fn test_phone_number_normalization() {
    let l = list(&["1234567890"]);
    // With leading + should still match
    assert!(check_allowed_sender("+1234567890", &l, "test"));
}

#[test]
fn test_allow_list_with_plus_prefix() {
    let l = list(&["+1234567890"]);
    // Without leading + should still match
    assert!(check_allowed_sender("1234567890", &l, "test"));
}

#[test]
fn test_no_substring_match() {
    let l = list(&["alice"]);
    assert!(!check_allowed_sender("alice123", &l, "test"));
    assert!(!check_allowed_sender("xalice", &l, "test"));
}

#[test]
fn test_pairing_store_fallback() {
    let _lock = ENV_MUTEX
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: serialized by ENV_MUTEX; cleaned up by EnvVarGuard on drop
    unsafe { std::env::set_var("OXICRAB_HOME", dir.path().as_os_str()) };
    let _env = EnvVarGuard("OXICRAB_HOME");

    // Create the workspace/memory directory and populate the DB
    let db_dir = dir.path().join("workspace").join("memory");
    std::fs::create_dir_all(&db_dir).unwrap();
    let db_path = db_dir.join("memory.sqlite3");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS pairing_allowlist (
            channel TEXT NOT NULL,
            sender_id TEXT NOT NULL,
            PRIMARY KEY (channel, sender_id)
        )",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO pairing_allowlist (channel, sender_id) VALUES (?1, ?2)",
        rusqlite::params!["telegram", "user789"],
    )
    .unwrap();
    drop(conn);

    // Empty allowFrom but paired -> allowed
    assert!(check_allowed_sender(
        "user789",
        &DenyByDefaultList::default(),
        "telegram"
    ));
    // Not in allowFrom and not paired -> denied
    assert!(!check_allowed_sender(
        "unknown",
        &DenyByDefaultList::default(),
        "telegram"
    ));
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
        check_dm_access(
            "anyone",
            &DenyByDefaultList::default(),
            "test",
            &DmPolicy::Open
        ),
        DmCheckResult::Allowed
    ));
}

#[test]
fn test_dm_access_allowlist_denies_unknown() {
    assert!(matches!(
        check_dm_access(
            "unknown",
            &DenyByDefaultList::default(),
            "test",
            &DmPolicy::Allowlist
        ),
        DmCheckResult::Denied
    ));
}

#[test]
fn test_dm_access_allowlist_allows_known() {
    let l = list(&["alice"]);
    assert!(matches!(
        check_dm_access("alice", &l, "test", &DmPolicy::Allowlist),
        DmCheckResult::Allowed
    ));
}

#[test]
fn test_dm_access_pairing_allows_known() {
    let l = list(&["bob"]);
    assert!(matches!(
        check_dm_access("bob", &l, "test", &DmPolicy::Pairing),
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

// --- check_group_access tests ---

#[test]
fn test_group_access_empty_denies_all() {
    assert!(!check_group_access(
        "any_group",
        &DenyByDefaultList::default()
    ));
}

#[test]
fn test_group_access_wildcard_allows_all() {
    let groups = list(&["*"]);
    assert!(check_group_access("anything", &groups));
}

#[test]
fn test_group_access_explicit_match() {
    let groups = list(&["group1", "group2"]);
    assert!(check_group_access("group1", &groups));
    assert!(check_group_access("group2", &groups));
}

#[test]
fn test_group_access_no_match_denied() {
    let groups = list(&["group1"]);
    assert!(!check_group_access("group99", &groups));
}

// --- backoff bounds test over many iterations ---

#[test]
fn test_backoff_always_bounded() {
    // Run many attempts and verify bounds hold for every single one
    for attempt in 0..50 {
        let delay = exponential_backoff_delay(attempt, 5, 60);
        // Minimum: the smaller of base or max (since max caps before jitter)
        // For attempt 0: 5 * 2^0 = 5, capped at 60, jitter adds 0-25%
        // Maximum possible: 60 + 25% of 60 = 75
        assert!(
            delay <= 75,
            "attempt {attempt}: delay {delay} exceeds max + jitter"
        );
    }
}

// --- normalize_sender_id control character stripping ---

#[test]
fn test_normalize_strips_control_chars() {
    assert_eq!(normalize_sender_id("user\x00name"), "username");
    assert_eq!(normalize_sender_id("user\nname"), "username");
    assert_eq!(normalize_sender_id("user\rname"), "username");
    assert_eq!(normalize_sender_id("+\x01user"), "user");
}

// --- dm_access pairing denied without requester ---

#[test]
fn test_dm_access_pairing_denies_without_requester() {
    // No pairing requester set globally, so pairing policy falls back to denied
    assert!(matches!(
        check_dm_access(
            "unknown",
            &DenyByDefaultList::default(),
            "test",
            &DmPolicy::Pairing
        ),
        DmCheckResult::Denied
    ));
}
