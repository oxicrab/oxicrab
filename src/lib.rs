pub mod agent;
pub mod auth;
pub mod bus;
pub mod channels;
pub mod cli;
pub mod config;
pub mod cron;
pub mod heartbeat;
pub mod providers;
pub mod session;
pub mod utils;

pub use auth::google::{get_credentials, has_valid_credentials, run_oauth_flow, GoogleCredentials};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const LOGO: &str = "🤖";
