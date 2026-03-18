#![warn(clippy::pedantic)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::module_name_repetitions)]

#[cfg(feature = "channel-discord")]
pub mod discord;
pub mod dispatch;
pub mod manager;
pub mod media_utils;
pub mod regex_utils;
#[cfg(feature = "channel-slack")]
pub mod slack;
#[cfg(feature = "channel-telegram")]
pub mod telegram;
#[cfg(feature = "channel-twilio")]
pub mod twilio;
pub mod utils;
#[cfg(feature = "channel-whatsapp")]
pub mod whatsapp;

/// Trait for requesting pairing codes from an external store.
/// Implemented by the main crate to wire in `PairingStore` without
/// pulling in the full `MemoryDB` dependency.
pub trait PairingRequester: Send + Sync {
    /// Request a pairing code for a sender on a channel.
    /// Returns `Some(code)` if a new code was issued, `None` if rate-limited or failed.
    fn request_pairing(&self, channel: &str, sender_id: &str) -> Option<String>;
}

/// Global pairing requester, set by the main crate at startup.
static PAIRING_REQUESTER: std::sync::OnceLock<Box<dyn PairingRequester>> =
    std::sync::OnceLock::new();

/// Register a pairing requester implementation.
/// Called once by the main crate at startup.
pub fn set_pairing_requester(requester: Box<dyn PairingRequester>) {
    let _ = PAIRING_REQUESTER.set(requester);
}

/// Get the registered pairing requester, if any.
pub fn get_pairing_requester() -> Option<&'static dyn PairingRequester> {
    PAIRING_REQUESTER.get().map(AsRef::as_ref)
}
