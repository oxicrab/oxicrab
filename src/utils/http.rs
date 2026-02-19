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
mod tests {
    use super::*;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn test_default_http_client_builds() {
        let _client = default_http_client();
    }

    #[test]
    fn test_default_max_body_bytes() {
        assert_eq!(DEFAULT_MAX_BODY_BYTES, 10 * 1024 * 1024);
    }

    async fn get_response(server: &MockServer) -> Response {
        Client::new().get(server.uri()).send().await.unwrap()
    }

    #[tokio::test]
    async fn test_limited_body_under_limit() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"hello world"))
            .mount(&server)
            .await;
        let resp = get_response(&server).await;
        let (result, truncated) = limited_body(resp, 1024).await.unwrap();
        assert_eq!(result, b"hello world");
        assert!(!truncated);
    }

    #[tokio::test]
    async fn test_limited_body_exact_limit() {
        let body = vec![b'x'; 100];
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
            .mount(&server)
            .await;
        let resp = get_response(&server).await;
        let (result, truncated) = limited_body(resp, 100).await.unwrap();
        assert_eq!(result, body);
        assert!(!truncated);
    }

    #[tokio::test]
    async fn test_limited_body_exceeds_limit_truncates() {
        // Use raw TCP with chunked encoding (no Content-Length) to test
        // the streaming truncation path without triggering the CL pre-check.
        use std::io::Write;
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = tokio::task::spawn_blocking(move || {
            let (mut stream, _) = listener.accept().unwrap();
            // Read request
            let mut buf = [0u8; 1024];
            let _ = std::io::Read::read(&mut stream, &mut buf);
            // Send chunked response without Content-Length
            let body = vec![b'x'; 200];
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n"
            )
            .unwrap();
            // Send as single chunk
            write!(stream, "{:x}\r\n", body.len()).unwrap();
            stream.write_all(&body).unwrap();
            write!(stream, "\r\n0\r\n\r\n").unwrap();
        });

        let resp = Client::new()
            .get(format!("http://{addr}"))
            .send()
            .await
            .unwrap();
        assert!(resp.content_length().is_none());
        let (result, truncated) = limited_body(resp, 50).await.unwrap();
        assert!(truncated);
        // No marker appended — raw bytes only
        assert_eq!(result.len(), 50);
        assert!(result.iter().all(|&b| b == b'x'));
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_limited_body_content_length_over_limit_rejects() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; 10000]))
            .mount(&server)
            .await;
        let resp = get_response(&server).await;
        // The server will set Content-Length: 10000 automatically
        let result = limited_body(resp, 100).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("too large"));
    }

    #[tokio::test]
    async fn test_limited_text_returns_string() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("hello text"))
            .mount(&server)
            .await;
        let resp = get_response(&server).await;
        let result = limited_text(resp, 1024).await.unwrap();
        assert_eq!(result, "hello text");
    }

    #[tokio::test]
    async fn test_limited_text_handles_invalid_utf8() {
        let body = vec![0xFF, 0xFE, b'o', b'k'];
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .mount(&server)
            .await;
        let resp = get_response(&server).await;
        let result = limited_text(resp, 1024).await.unwrap();
        assert!(result.contains("ok"));
    }

    #[tokio::test]
    async fn test_limited_body_empty_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let resp = get_response(&server).await;
        let (result, truncated) = limited_body(resp, 1024).await.unwrap();
        assert!(result.is_empty());
        assert!(!truncated);
    }
}
