use reqwest::Client;
use std::time::Duration;

/// Build a `reqwest::Client` with standard timeouts (10 s connect, 30 s overall).
///
/// Falls back to the default client if the builder fails.
pub fn default_http_client() -> Client {
    Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| Client::new())
}
