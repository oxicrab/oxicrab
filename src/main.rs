use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,whatsapp_rust=warn".parse().unwrap());
    tracing_subscriber::fmt().with_env_filter(filter).init();

    nanobot::cli::run().await
}
