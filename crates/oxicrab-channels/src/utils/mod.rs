//! Utility functions for channel implementations

/// Maximum size for downloaded image attachments (20 MB).
pub const MAX_IMAGE_DOWNLOAD: usize = 20 * 1024 * 1024;
/// Maximum size for downloaded audio attachments (50 MB).
pub const MAX_AUDIO_DOWNLOAD: usize = 50 * 1024 * 1024;

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

/// Check if a sender appears in the pairing store's allowlist in the `MemoryDB`.
///
/// Opens a lightweight read-only `SQLite` connection to the workspace database.
/// No schema initialization needed — the gateway or CLI will have already created it.
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-slack",
    feature = "channel-whatsapp",
    feature = "channel-twilio",
))]
fn is_sender_paired(channel: &str, sender: &str) -> bool {
    let Ok(db_path) = get_memory_db_path() else {
        return false;
    };

    let Ok(conn) = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return false;
    };

    // Check exact match first
    if conn
        .query_row(
            "SELECT 1 FROM pairing_allowlist WHERE channel = ?1 AND sender_id = ?2 LIMIT 1",
            rusqlite::params![channel, sender],
            |_| Ok(true),
        )
        .unwrap_or(false)
    {
        return true;
    }

    // Check with normalization: strip leading '+' and control chars
    let normalized = normalize_sender_id(sender);
    if normalized == sender {
        false
    } else {
        conn.query_row(
            "SELECT 1 FROM pairing_allowlist WHERE channel = ?1 AND sender_id = ?2 LIMIT 1",
            rusqlite::params![channel, normalized],
            |_| Ok(true),
        )
        .unwrap_or(false)
    }
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

/// Check if a group/channel ID is allowed based on the `allowGroups` config list.
/// Empty list means all groups are allowed (backward compatible).
/// Non-empty list restricts to only the listed group IDs.
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-slack",
    feature = "channel-whatsapp",
    feature = "channel-twilio",
))]
pub fn check_group_access(group_id: &str, allow_groups: &[String]) -> bool {
    if allow_groups.is_empty() {
        return true; // empty = all groups allowed
    }
    allow_groups.iter().any(|g| g == group_id || g == "*")
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
    dm_policy: &oxicrab_core::config::schema::DmPolicy,
) -> DmCheckResult {
    use oxicrab_core::config::schema::DmPolicy;

    if *dm_policy == DmPolicy::Open {
        return DmCheckResult::Allowed;
    }

    if check_allowed_sender(sender, allow_list, channel) {
        return DmCheckResult::Allowed;
    }

    if *dm_policy == DmPolicy::Pairing {
        if let Some(requester) = crate::get_pairing_requester() {
            if let Some(code) = requester.request_pairing(channel, sender) {
                return DmCheckResult::PairingRequired { code };
            }
            tracing::debug!("pairing request rate-limited for {} on {}", sender, channel);
        } else {
            tracing::debug!("no pairing requester configured, denying {}", sender);
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
         Your ID: {sender_id}\n\
         Pairing code: {code}\n\
         Approve with: oxicrab pairing approve {channel} {code}"
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
    let delay = (base_delay_secs as f64 * 2.0_f64.powi((attempt as i32).min(20))) as u64;
    let capped = delay.min(max_delay_secs);
    // Add up to 25% jitter to avoid thundering herd
    let jitter = (capped as f64 * 0.25 * fastrand::f64()) as u64;
    capped + jitter
}

/// Resolve the path to the shared `MemoryDB` file.
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-slack",
    feature = "channel-whatsapp",
    feature = "channel-twilio",
))]
fn get_memory_db_path() -> anyhow::Result<std::path::PathBuf> {
    Ok(get_oxicrab_home()?
        .join("workspace")
        .join("memory")
        .join("memory.sqlite3"))
}

/// Get the oxicrab home directory.
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-slack",
    feature = "channel-whatsapp",
    feature = "channel-twilio",
))]
fn get_oxicrab_home() -> anyhow::Result<std::path::PathBuf> {
    use anyhow::Context;
    if let Some(home) = std::env::var_os("OXICRAB_HOME") {
        return Ok(std::path::PathBuf::from(home));
    }
    Ok(dirs::home_dir()
        .context("Could not determine home directory")?
        .join(".oxicrab"))
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
