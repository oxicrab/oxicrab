use super::*;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// --- resolve_slack_redirect tests ---

#[test]
fn test_resolve_slack_redirect() {
    let cases = [
        (
            "https://myworkspace.slack.com/?redir=%2Ffiles-pri%2FT123-F456%2Fdownload%2Fimage.png",
            "https://myworkspace.slack.com/files-pri/T123-F456/download/image.png",
        ),
        (
            "https://cdn.slack.com/files/image.png",
            "https://cdn.slack.com/files/image.png",
        ),
        ("not-a-url", "not-a-url"),
        (
            "https://ws.slack.com/?redir=%2Ffiles-pri%2FT1-F2%2Fdownload%2Fscreenshot%202026%4016.45.png",
            "https://ws.slack.com/files-pri/T1-F2/download/screenshot 2026@16.45.png",
        ),
        (
            "https://ws.slack.com/?foo=bar&redir=%2Ffiles-pri%2FT1-F2%2Fdownload%2Fimg.png&baz=1",
            "https://ws.slack.com/files-pri/T1-F2/download/img.png",
        ),
    ];
    for (input, expected) in cases {
        let result = resolve_slack_redirect(input);
        assert_eq!(result, expected, "failed for input: {}", input);
    }
    // Also verify scheme preservation
    let http_result = resolve_slack_redirect(
        "http://ws.slack.com/?redir=%2Ffiles-pri%2FT1-F2%2Fdownload%2Fimg.png",
    );
    assert!(
        http_result.starts_with("http://"),
        "should preserve http scheme"
    );
}

// --- is_image_magic_bytes tests ---

// --- download_slack_file tests (wiremock) ---

#[tokio::test]
async fn test_download_slack_file_success() {
    let server = MockServer::start().await;
    let png_body: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

    Mock::given(method("GET"))
        .and(path("/files-pri/T1-F2/download/image.png"))
        .and(header("Authorization", "Bearer xoxb-test"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(png_body.clone())
                .insert_header("Content-Type", "image/png"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = reqwest::Client::new();
    let url = format!("{}/files-pri/T1-F2/download/image.png", server.uri());
    let result = download_slack_file(&client, "xoxb-test", &url).await;

    assert!(result.is_ok());
    let bytes = result.unwrap();
    assert_eq!(bytes, png_body);
}

#[tokio::test]
async fn test_download_slack_file_sends_auth_header() {
    let server = MockServer::start().await;

    // Only match requests with the correct auth header
    Mock::given(method("GET"))
        .and(path("/file.png"))
        .and(header("Authorization", "Bearer my-secret-token"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0x89, 0x50, 0x4E, 0x47]))
        .expect(1)
        .mount(&server)
        .await;

    let client = reqwest::Client::new();
    let url = format!("{}/file.png", server.uri());
    let result = download_slack_file(&client, "my-secret-token", &url).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_download_slack_file_error_status() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/file.png"))
        .respond_with(ResponseTemplate::new(403))
        .expect(1)
        .mount(&server)
        .await;

    let client = reqwest::Client::new();
    let url = format!("{}/file.png", server.uri());
    let result = download_slack_file(&client, "xoxb-test", &url).await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("403"));
}

#[tokio::test]
async fn test_download_slack_file_empty_body_is_error() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/file.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(Vec::<u8>::new()))
        .expect(1)
        .mount(&server)
        .await;

    let client = reqwest::Client::new();
    let url = format!("{}/file.png", server.uri());
    let result = download_slack_file(&client, "xoxb-test", &url).await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("empty body"));
}

#[tokio::test]
async fn test_download_slack_file_follows_single_redirect() {
    let server = MockServer::start().await;
    let jpeg_body: Vec<u8> = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];

    // First request: redirect
    Mock::given(method("GET"))
        .and(path("/start"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", format!("{}/actual.jpg", server.uri())),
        )
        .expect(1)
        .mount(&server)
        .await;

    // Second request: file content
    Mock::given(method("GET"))
        .and(path("/actual.jpg"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(jpeg_body.clone())
                .insert_header("Content-Type", "image/jpeg"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = reqwest::Client::new();
    let url = format!("{}/start", server.uri());
    let result = download_slack_file(&client, "xoxb-test", &url).await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), jpeg_body);
}

#[tokio::test]
async fn test_download_slack_file_follows_multiple_redirects() {
    let server = MockServer::start().await;
    let png_body: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47];

    // Hop 0 -> Hop 1
    Mock::given(method("GET"))
        .and(path("/hop0"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("Location", format!("{}/hop1", server.uri())),
        )
        .expect(1)
        .mount(&server)
        .await;

    // Hop 1 -> Hop 2
    Mock::given(method("GET"))
        .and(path("/hop1"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("Location", format!("{}/hop2", server.uri())),
        )
        .expect(1)
        .mount(&server)
        .await;

    // Hop 2 -> final file
    Mock::given(method("GET"))
        .and(path("/hop2"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(png_body.clone()))
        .expect(1)
        .mount(&server)
        .await;

    let client = reqwest::Client::new();
    let url = format!("{}/hop0", server.uri());
    let result = download_slack_file(&client, "xoxb-test", &url).await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), png_body);
}

#[tokio::test]
async fn test_download_slack_file_redirect_preserves_auth_on_each_hop() {
    // Auth is always sent on the first hop (initial URL from Slack API).
    // On redirect hops, auth is only sent to Slack-owned domains; since
    // wiremock uses 127.0.0.1, the redirected hop should NOT receive auth.
    let server = MockServer::start().await;

    // First hop: requires auth (initial URL)
    Mock::given(method("GET"))
        .and(path("/hop0"))
        .and(header("Authorization", "Bearer xoxb-hop-test"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("Location", format!("{}/hop1", server.uri())),
        )
        .expect(1)
        .mount(&server)
        .await;

    // Second hop: no auth required (non-Slack redirect target)
    Mock::given(method("GET"))
        .and(path("/hop1"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0x89, 0x50, 0x4E, 0x47]))
        .expect(1)
        .mount(&server)
        .await;

    let client = reqwest::Client::new();
    let url = format!("{}/hop0", server.uri());
    let result = download_slack_file(&client, "xoxb-hop-test", &url).await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn test_download_slack_file_redirect_loop_detection() {
    let server = MockServer::start().await;

    // Always redirect to self
    Mock::given(method("GET"))
        .and(path("/loop"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("Location", format!("{}/loop", server.uri())),
        )
        .mount(&server)
        .await;

    let client = reqwest::Client::new();
    let url = format!("{}/loop", server.uri());
    let result = download_slack_file(&client, "xoxb-test", &url).await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("redirect loop"),
        "Expected redirect loop error, got: {}",
        err
    );
}

#[tokio::test]
async fn test_download_slack_file_redirect_loop_mentions_files_read() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/loop"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("Location", format!("{}/loop", server.uri())),
        )
        .mount(&server)
        .await;

    let client = reqwest::Client::new();
    let url = format!("{}/loop", server.uri());
    let result = download_slack_file(&client, "xoxb-test", &url).await;

    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("files:read"),
        "Error should mention missing files:read scope, got: {}",
        err
    );
}

#[tokio::test]
async fn test_download_slack_file_exceeds_max_redirects() {
    let server = MockServer::start().await;

    // Chain of unique redirects that exceeds max_redirects=5.
    // No .expect() — some hops may not be reached before the limit.
    for i in 0..6 {
        Mock::given(method("GET"))
            .and(path(format!("/hop{}", i)))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/hop{}", server.uri(), i + 1)),
            )
            .mount(&server)
            .await;
    }

    let client = reqwest::Client::new();
    let url = format!("{}/hop0", server.uri());
    let result = download_slack_file(&client, "xoxb-test", &url).await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("exceeded"));
}

#[tokio::test]
async fn test_download_slack_file_500_error() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/file.png"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;

    let client = reqwest::Client::new();
    let url = format!("{}/file.png", server.uri());
    let result = download_slack_file(&client, "xoxb-test", &url).await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("500"));
}

// --- format_for_slack tests ---

#[test]
fn test_format_for_slack_bold() {
    assert_eq!(SlackChannel::format_for_slack("**bold**"), "*bold*");
}

#[test]
fn test_format_for_slack_link() {
    assert_eq!(
        SlackChannel::format_for_slack("[text](https://example.com)"),
        "<https://example.com|text>"
    );
}

#[test]
fn test_format_for_slack_strikethrough() {
    assert_eq!(SlackChannel::format_for_slack("~~strike~~"), "~strike~");
}

#[test]
fn test_format_for_slack_empty() {
    assert_eq!(SlackChannel::format_for_slack(""), "");
}

#[test]
fn test_format_for_slack_plain_text() {
    assert_eq!(
        SlackChannel::format_for_slack("no formatting here"),
        "no formatting here"
    );
}

#[test]
fn test_format_for_slack_combined_formatting() {
    let result =
        SlackChannel::format_for_slack("**bold** and ~~strike~~ and [link](https://x.com)");
    assert_eq!(result, "*bold* and ~strike~ and <https://x.com|link>");
}

#[test]
fn test_format_for_slack_multiple_bold() {
    assert_eq!(
        SlackChannel::format_for_slack("**a** then **b**"),
        "*a* then *b*"
    );
}

#[test]
fn test_format_for_slack_multiple_links() {
    let input = "[one](https://one.com) and [two](https://two.com)";
    let expected = "<https://one.com|one> and <https://two.com|two>";
    assert_eq!(SlackChannel::format_for_slack(input), expected);
}

#[test]
fn test_format_for_slack_preserves_code_backticks() {
    // Slack also uses backticks for code — should pass through unchanged
    let input = "run `cargo test`";
    assert_eq!(SlackChannel::format_for_slack(input), "run `cargo test`");
}

#[test]
fn test_format_for_slack_already_slack_bold() {
    // Single asterisks should NOT be converted (they're already Slack bold)
    let input = "*already slack bold*";
    // Since regex matches **..** only, single * passes through
    assert_eq!(
        SlackChannel::format_for_slack(input),
        "*already slack bold*"
    );
}

#[test]
fn test_format_for_slack_newlines_preserved() {
    let input = "line 1\nline 2\n**bold line**";
    let expected = "line 1\nline 2\n*bold line*";
    assert_eq!(SlackChannel::format_for_slack(input), expected);
}
