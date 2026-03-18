//! Private utilities for Google tools.

use regex::Regex;
use reqwest::Client;
use std::sync::LazyLock;
use std::time::Duration;

/// Build a `reqwest::Client` with standard timeouts.
pub fn default_http_client() -> Client {
    Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| Client::new())
}

/// Regex for matching HTML tags.
pub fn html_tags_regex() -> &'static Regex {
    static RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"<[^>]+>").expect("Failed to compile HTML tags regex"));
    &RE
}
