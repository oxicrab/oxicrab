use anyhow::{Result, bail};
use reqwest::{Client, Response};
use std::time::Duration;

use super::url_security::ResolvedUrl;

/// Default maximum body size for streaming downloads (10 MB).
pub const DEFAULT_MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

/// Build a reqwest Client pinned to resolved DNS addresses.
/// Disables redirects (SSRF prevention) and prefers IPv4 when available.
pub fn build_pinned_client(
    resolved: &ResolvedUrl,
    timeout: Duration,
    user_agent: Option<&str>,
) -> Result<Client> {
    let mut builder = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none());

    if let Some(ua) = user_agent {
        builder = builder.user_agent(ua);
    }

    // Prefer IPv4 when available (some hosts have broken IPv6)
    let has_ipv4 = resolved.addrs.iter().any(std::net::SocketAddr::is_ipv4);
    for addr in &resolved.addrs {
        if has_ipv4 && addr.is_ipv6() {
            continue;
        }
        builder = builder.resolve(&resolved.host, *addr);
    }

    builder
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build pinned client: {e}"))
}

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

/// Download a response body as bytes with a size limit.
///
/// Returns `(bytes, was_truncated)`.
pub async fn limited_body(resp: Response, max_bytes: usize) -> Result<(Vec<u8>, bool)> {
    if let Some(cl) = resp.content_length()
        && cl as usize > max_bytes
    {
        bail!("response body too large: Content-Length {cl} exceeds limit {max_bytes}");
    }

    let mut buf = Vec::new();
    let mut stream = resp;
    while let Some(chunk) = stream.chunk().await? {
        if buf.len() + chunk.len() > max_bytes {
            let remaining = max_bytes.saturating_sub(buf.len());
            buf.extend_from_slice(&chunk[..remaining]);
            return Ok((buf, true));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok((buf, false))
}

/// Download a response body as a UTF-8 string with a size limit.
pub async fn limited_text(resp: Response, max_bytes: usize) -> Result<String> {
    let (bytes, truncated) = limited_body(resp, max_bytes).await?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        text.push_str("\n[truncated]");
    }
    Ok(text)
}
