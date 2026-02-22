use anyhow::{Result, bail};
use reqwest::{Client, Response};
use std::time::Duration;

/// Default maximum body size for streaming downloads (10 MB).
pub const DEFAULT_MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

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
/// - Checks the `Content-Length` header first; rejects immediately if over limit.
/// - Streams via `chunk()` with a running counter; truncates at the limit.
/// - Returns `(bytes, was_truncated)`. The bytes are raw with no marker appended,
///   so binary content (images, audio) is not corrupted on truncation.
pub async fn limited_body(resp: Response, max_bytes: usize) -> Result<(Vec<u8>, bool)> {
    // Pre-check Content-Length header
    if let Some(cl) = resp.content_length()
        && cl as usize > max_bytes
    {
        bail!(
            "response body too large: Content-Length {} exceeds limit {}",
            cl,
            max_bytes
        );
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
///
/// Same semantics as [`limited_body`] but converts the result to a `String`
/// and appends a `\n[truncated]` marker when the body exceeds the limit.
pub async fn limited_text(resp: Response, max_bytes: usize) -> Result<String> {
    let (bytes, truncated) = limited_body(resp, max_bytes).await?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        text.push_str("\n[truncated]");
    }
    Ok(text)
}

#[cfg(test)]
mod tests;
