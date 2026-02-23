#![warn(clippy::pedantic)]
// Noisy doc/signature lints — would require annotating hundreds of pub functions
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::must_use_candidate)]
// Style preference — keeping format!("{}", x) over format!("{x}") for readability with complex exprs
#![allow(clippy::uninlined_format_args)]
// Intentional casts throughout LLM/API integration code (token counts, timestamps, sizes)
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_wrap)]
// Complex tool/loop functions are naturally long; splitting would be artificial
#![allow(clippy::too_many_lines)]
// Module structure — our tool module has foo::FooTool pattern by design
#![allow(clippy::module_name_repetitions)]

pub mod agent;
pub(crate) mod auth;
pub mod bus;
pub(crate) mod channels;
pub mod cli;
pub mod config;
pub mod cron;
pub(crate) mod errors;
pub mod gateway;
pub(crate) mod heartbeat;
pub mod pairing;
pub mod providers;
pub mod safety;
pub mod session;
pub(crate) mod utils;

/// Re-exports for fuzz targets. Not part of the public API.
#[doc(hidden)]
pub mod fuzz_api {
    pub use crate::utils::url_security::validate_and_resolve;

    /// Wrapper around `gateway::validate_webhook_signature` for fuzz targets.
    pub fn validate_webhook_signature(secret: &str, signature: &str, body: &[u8]) -> bool {
        crate::gateway::validate_webhook_signature(secret, signature, body)
    }
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const LOGO: &str = "🤖";
