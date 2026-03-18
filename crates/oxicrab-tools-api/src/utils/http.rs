//! HTTP client utilities.

use reqwest::Client;
use std::time::Duration;

/// Build a `reqwest::Client` with standard timeouts (10s connect, 30s overall).
pub fn default_http_client() -> Client {
    Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| Client::new())
}
