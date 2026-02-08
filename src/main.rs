mod agent;
mod auth;
mod bus;
mod channels;
mod cli;
mod config;
mod cron;
mod errors;
mod heartbeat;
mod providers;
mod session;
mod utils;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    cli::run().await
}
