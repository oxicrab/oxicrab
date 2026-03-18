use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,wa_rs=warn".parse().unwrap())
        .add_directive("selectors=off".parse().unwrap())
        .add_directive("html5ever=off".parse().unwrap())
        .add_directive("hyper_util=warn".parse().unwrap());
    tracing_subscriber::fmt().with_env_filter(filter).init();

    oxicrab::cli::run().await
}
