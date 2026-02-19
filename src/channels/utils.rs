//! Utility functions for channel implementations

/// Check if a sender is allowed based on an allow list OR the pairing store.
///
/// Access is granted if any of these conditions are met:
/// 1. `allow_list` contains `"*"` (wildcard — allow everyone)
/// 2. `sender` matches an entry in `allow_list` (after normalization)
/// 3. `sender` is in the pairing store's allowlist for `channel`
///
/// If none match, access is denied (default-deny).
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-slack",
    feature = "channel-whatsapp",
    feature = "channel-twilio",
))]
pub fn check_allowed_sender(sender: &str, allow_list: &[String], channel: &str) -> bool {
    // Explicit wildcard allows all senders
    if allow_list.iter().any(|a| a == "*") {
        return true;
    }

    // Check config allowlist
    let normalized_sender = normalize_sender_id(sender);
    if allow_list
        .iter()
        .any(|allowed| normalized_sender == normalize_sender_id(allowed))
    {
        return true;
    }

    // Fallback: check pairing store
    is_sender_paired(channel, sender)
}

/// Check if a sender appears in the pairing store's per-channel allowlist file.
///
/// Reads `~/.oxicrab/pairing/{channel}-allowlist.json` directly from disk so that
/// CLI `oxicrab pairing approve` takes effect without restarting the gateway.
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-slack",
    feature = "channel-whatsapp",
    feature = "channel-twilio",
))]
fn is_sender_paired(channel: &str, sender: &str) -> bool {
    #[derive(serde::Deserialize)]
    struct AllowlistData {
        senders: Vec<String>,
    }

    let Ok(home) = crate::utils::get_oxicrab_home() else {
        return false;
    };
    let path = home
        .join("pairing")
        .join(format!("{}-allowlist.json", channel));
    let Ok(content) = std::fs::read_to_string(&path) else {
        return false;
    };
    let Ok(data) = serde_json::from_str::<AllowlistData>(&content) else {
        return false;
    };
    let normalized = normalize_sender_id(sender);
    data.senders
        .iter()
        .any(|s| normalize_sender_id(s) == normalized)
}

/// Normalize a sender ID by removing common prefixes and formatting
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-slack",
    feature = "channel-whatsapp",
    feature = "channel-twilio",
))]
pub fn normalize_sender_id(sender: &str) -> String {
    sender.trim_start_matches('+').to_string()
}

/// Result of a DM access check.
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-slack",
    feature = "channel-whatsapp",
    feature = "channel-twilio",
))]
pub enum DmCheckResult {
    Allowed,
    Denied,
    PairingRequired { code: String },
}

/// Check DM access based on the channel's `dmPolicy`.
///
/// - `"open"` — allow all senders unconditionally
/// - `"allowlist"` — check config allowFrom + pairing store; silently deny unknown
/// - `"pairing"` — check config allowFrom + pairing store; issue a pairing code for unknown
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-slack",
    feature = "channel-whatsapp",
    feature = "channel-twilio",
))]
pub fn check_dm_access(
    sender: &str,
    allow_list: &[String],
    channel: &str,
    dm_policy: &crate::config::DmPolicy,
) -> DmCheckResult {
    if *dm_policy == crate::config::DmPolicy::Open {
        return DmCheckResult::Allowed;
    }

    if check_allowed_sender(sender, allow_list, channel) {
        return DmCheckResult::Allowed;
    }

    if *dm_policy == crate::config::DmPolicy::Pairing {
        match crate::pairing::PairingStore::new() {
            Ok(mut store) => match store.request_pairing(channel, sender) {
                Ok(Some(code)) => return DmCheckResult::PairingRequired { code },
                Ok(None) => {
                    tracing::debug!("pairing request rate-limited for {} on {}", sender, channel);
                }
                Err(e) => {
                    tracing::warn!("failed to create pairing request: {}", e);
                }
            },
            Err(e) => {
                tracing::warn!("failed to open pairing store: {}", e);
            }
        }
    }

    DmCheckResult::Denied
}

/// Format a pairing reply message for an unrecognized sender.
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-slack",
    feature = "channel-whatsapp",
    feature = "channel-twilio",
))]
pub fn format_pairing_reply(channel: &str, sender_id: &str, code: &str) -> String {
    format!(
        "Access not configured. To use this bot, ask the owner to approve:\n\
         Your ID: {}\n\
         Pairing code: {}\n\
         Approve with: oxicrab pairing approve {} {}",
        sender_id, code, channel, code
    )
}

/// Calculate exponential backoff delay for reconnection attempts
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-slack",
    feature = "channel-whatsapp",
    feature = "channel-twilio",
))]
pub fn exponential_backoff_delay(attempt: u32, base_delay_secs: u64, max_delay_secs: u64) -> u64 {
    let delay = (base_delay_secs as f64 * 2.0_f64.powi(attempt as i32)) as u64;
    let capped = delay.min(max_delay_secs);
    // Add up to 25% jitter to avoid thundering herd
    let jitter = (capped as f64 * 0.25 * fastrand::f64()) as u64;
    capped + jitter
}

#[cfg(test)]
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-slack",
    feature = "channel-whatsapp",
    feature = "channel-twilio",
))]
mod tests {
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
}
