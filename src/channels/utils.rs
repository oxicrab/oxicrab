//! Utility functions for channel implementations

/// Check if a sender is allowed based on an allow list (exact match after normalization)
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-slack",
    feature = "channel-whatsapp",
    feature = "channel-twilio",
))]
pub fn check_allowed_sender(sender: &str, allow_list: &[String]) -> bool {
    // Default-deny: empty allowlist denies all senders
    if allow_list.is_empty() {
        return false;
    }

    // Explicit wildcard allows all senders
    if allow_list.iter().any(|a| a == "*") {
        return true;
    }

    let normalized_sender = normalize_sender_id(sender);
    allow_list
        .iter()
        .any(|allowed| normalized_sender == normalize_sender_id(allowed))
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
    delay.min(max_delay_secs)
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
        assert!(!check_allowed_sender("anyone", &[]));
    }

    #[test]
    fn test_wildcard_allows_all() {
        let list = vec!["*".to_string()];
        assert!(check_allowed_sender("anyone", &list));
        assert!(check_allowed_sender("other_user", &list));
    }

    #[test]
    fn test_exact_match_allowed() {
        let list = vec!["alice".to_string(), "bob".to_string()];
        assert!(check_allowed_sender("alice", &list));
        assert!(check_allowed_sender("bob", &list));
    }

    #[test]
    fn test_non_matching_sender_rejected() {
        let list = vec!["alice".to_string()];
        assert!(!check_allowed_sender("eve", &list));
    }

    #[test]
    fn test_phone_number_normalization() {
        let list = vec!["1234567890".to_string()];
        // With leading + should still match
        assert!(check_allowed_sender("+1234567890", &list));
    }

    #[test]
    fn test_allow_list_with_plus_prefix() {
        let list = vec!["+1234567890".to_string()];
        // Without leading + should still match
        assert!(check_allowed_sender("1234567890", &list));
    }

    #[test]
    fn test_no_substring_match() {
        let list = vec!["alice".to_string()];
        assert!(!check_allowed_sender("alice123", &list));
        assert!(!check_allowed_sender("xalice", &list));
    }

    #[test]
    fn test_normalize_strips_plus() {
        assert_eq!(normalize_sender_id("+1234"), "1234");
        assert_eq!(normalize_sender_id("1234"), "1234");
        assert_eq!(normalize_sender_id("+++abc"), "abc");
    }

    #[test]
    fn test_backoff_first_attempt() {
        assert_eq!(exponential_backoff_delay(0, 1, 60), 1);
    }

    #[test]
    fn test_backoff_grows_exponentially() {
        assert_eq!(exponential_backoff_delay(0, 2, 120), 2);
        assert_eq!(exponential_backoff_delay(1, 2, 120), 4);
        assert_eq!(exponential_backoff_delay(2, 2, 120), 8);
        assert_eq!(exponential_backoff_delay(3, 2, 120), 16);
    }

    #[test]
    fn test_backoff_capped_at_max() {
        assert_eq!(exponential_backoff_delay(10, 2, 60), 60);
        assert_eq!(exponential_backoff_delay(20, 2, 60), 60);
    }
}
