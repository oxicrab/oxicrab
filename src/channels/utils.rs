//! Utility functions for channel implementations

/// Check if a sender is allowed based on an allow list
pub fn check_allowed_sender(sender: &str, allow_list: &[String]) -> bool {
    if allow_list.is_empty() {
        return true;
    }

    let normalized_sender = normalize_sender_id(sender);
    allow_list.iter().any(|allowed| {
        let normalized_allowed = normalize_sender_id(allowed);
        normalized_sender == normalized_allowed
            || normalized_sender.contains(&normalized_allowed)
            || normalized_allowed.contains(&normalized_sender)
    })
}

/// Normalize a sender ID by removing common prefixes and formatting
pub fn normalize_sender_id(sender: &str) -> String {
    sender.trim_start_matches('+').to_string()
}

/// Calculate exponential backoff delay for reconnection attempts
pub fn exponential_backoff_delay(attempt: u32, base_delay_secs: u64, max_delay_secs: u64) -> u64 {
    let delay = (base_delay_secs as f64 * 2.0_f64.powi(attempt as i32)) as u64;
    delay.min(max_delay_secs)
}
