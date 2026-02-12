pub mod agent;
pub(crate) mod auth;
pub mod bus;
pub(crate) mod channels;
pub mod cli;
pub mod config;
pub mod cron;
pub(crate) mod errors;
pub(crate) mod heartbeat;
pub mod providers;
pub mod session;
pub(crate) mod utils;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const LOGO: &str = "🤖";
