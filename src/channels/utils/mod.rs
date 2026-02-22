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
    let pairing_dir = home.join("pairing");
    let path = pairing_dir.join(format!("{}-allowlist.json", channel));

    // Acquire shared lock for consistent reads (writers hold exclusive lock).
    // The lock is held via _lock until the end of this function scope.
    let _lock = (|| -> Option<std::fs::File> {
        let lock_path = pairing_dir.join(".lock");
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .ok()?;
        fs2::FileExt::lock_shared(&lock_file).ok()?;
        Some(lock_file)
    })();

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
    let trimmed = sender.trim_start_matches('+');
    // Strip control characters and null bytes to prevent injection
    trimmed.chars().filter(|c| !c.is_control()).collect()
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
mod tests;
