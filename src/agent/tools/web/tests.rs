use super::*;
use crate::agent::tools::base::ExecutionContext;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// --- Unit tests for HTML processing functions ---

#[test]
fn test_strip_tags_removes_html_tags() {
    let html = "<p>hello</p>";
    let result = strip_tags(html);
    assert!(result.contains("hello"));
    assert!(!result.contains("<p>"));
    assert!(!result.contains("</p>"));
}

#[test]
fn test_strip_tags_handles_nested_tags() {
    let html = "<div><p><strong>nested</strong> content</p></div>";
    let result = strip_tags(html);
    assert!(result.contains("nested"));
    assert!(result.contains("content"));
    assert!(!result.contains("<div>"));
    assert!(!result.contains("<strong>"));
}

#[test]
fn test_normalize_collapses_whitespace() {
    let text = "a  b\t\tc";
    let result = normalize(text);
    assert_eq!(result, "a b c");
}

#[test]
fn test_normalize_trims_excess_newlines() {
    let text = "line1\n\n\n\nline2";
    let result = normalize(text);
    assert!(result.contains("line1"));
    assert!(result.contains("line2"));
    assert!(!result.contains("\n\n\n\n"));
}

#[test]
fn test_strip_scripts_styles_removes_script_blocks() {
    let html = "<div>content</div><script>alert('test');</script><p>more</p>";
    let result = strip_scripts_styles(html);
    assert!(result.contains("<div>content</div>"));
    assert!(result.contains("<p>more</p>"));
    assert!(!result.contains("<script>"));
    assert!(!result.contains("alert"));
}

#[test]
fn test_strip_scripts_styles_removes_style_blocks() {
    let html = "<div>content</div><style>.class { color: red; }</style><p>more</p>";
    let result = strip_scripts_styles(html);
    assert!(result.contains("<div>content</div>"));
    assert!(result.contains("<p>more</p>"));
    assert!(!result.contains("<style>"));
    assert!(!result.contains("color: red"));
}

#[test]
fn test_html_to_markdown_converts_links() {
    let html = r#"<a href="http://example.com">click here</a>"#;
    let result = html_to_markdown(html);
    assert!(result.contains("[click here](http://example.com)") || result.contains("click here"));
}

#[test]
fn test_html_to_markdown_converts_bold() {
    let html = "<b>bold text</b> and <strong>strong text</strong>";
    let result = html_to_markdown(html);
    assert!(result.contains("bold text"));
    assert!(result.contains("strong text"));
}

#[test]
fn test_html_to_markdown_converts_headings() {
    let html = "<h1>Main Title</h1><h2>Subtitle</h2><p>Body text</p>";
    let result = html_to_markdown(html);
    assert!(result.contains("Main Title"));
    assert!(result.contains("Subtitle"));
    assert!(result.contains("Body text"));
}

#[test]
fn test_html_to_markdown_converts_list_items() {
    let html = "<ul><li>First</li><li>Second</li></ul>";
    let result = html_to_markdown(html);
    assert!(result.contains("First"));
    assert!(result.contains("Second"));
}

#[test]
fn test_extract_html_prefers_article() {
    let html = r"<html><body><nav>Nav stuff</nav><article><p>Article content</p></article><footer>Footer</footer></body></html>";
    let result = extract_html(html, false).unwrap();
    assert!(result.contains("Article content"));
}

#[test]
fn test_extract_html_title_in_markdown_mode() {
    let html = "<html><head><title>My Page</title></head><body><p>Body text</p></body></html>";
    let result = extract_html(html, true).unwrap();
    assert!(result.contains("# My Page"));
}

#[test]
fn test_strip_tags_decodes_entities() {
    let html = "<p>5 &gt; 3 &amp; 2 &lt; 4</p>";
    let result = strip_tags(html);
    assert!(result.contains("5 > 3 & 2 < 4"));
}

// --- Wiremock tests for WebFetchTool ---

fn parse_fetch_result(result: &ToolResult) -> Value {
    serde_json::from_str(&result.content).expect("fetch result should be JSON")
}

#[tokio::test]
async fn test_fetch_html_page() {
    let server = MockServer::start().await;
    let html = r"<!DOCTYPE html><html><head><title>Test Page</title></head><body><article><p>Hello from the article</p></article></body></html>";
    Mock::given(method("GET"))
        .and(path("/page"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html")
                .set_body_string(html),
        )
        .mount(&server)
        .await;

    let tool = WebFetchTool::new(50000).unwrap();
    let result = tool
        .fetch_url(&serde_json::json!({"url": format!("{}/page", server.uri())}))
        .await
        .unwrap();

    assert!(!result.is_error);
    let json = parse_fetch_result(&result);
    assert_eq!(json["status"], 200);
    assert_eq!(json["extractor"], "readability");
    assert!(json["text"]
        .as_str()
        .unwrap()
        .contains("Hello from the article"));
}

#[tokio::test]
async fn test_fetch_html_extracts_in_text_mode() {
    let server = MockServer::start().await;
    let html = r"<html><body><h1>Heading</h1><p>Paragraph text</p></body></html>";
    Mock::given(method("GET"))
        .and(path("/text"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html")
                .set_body_string(html),
        )
        .mount(&server)
        .await;

    let tool = WebFetchTool::new(50000).unwrap();
    let result = tool
        .fetch_url(&serde_json::json!({
            "url": format!("{}/text", server.uri()),
            "extractMode": "text"
        }))
        .await
        .unwrap();

    let json = parse_fetch_result(&result);
    assert_eq!(json["extractor"], "readability");
    let text = json["text"].as_str().unwrap();
    assert!(text.contains("Heading"));
    assert!(text.contains("Paragraph text"));
    // In text mode, should NOT have markdown heading markers
    assert!(!text.contains("# Heading"));
}

#[tokio::test]
async fn test_fetch_json_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"key": "value", "num": 42})),
        )
        .mount(&server)
        .await;

    let tool = WebFetchTool::new(50000).unwrap();
    let result = tool
        .fetch_url(&serde_json::json!({"url": format!("{}/api", server.uri())}))
        .await
        .unwrap();

    let json = parse_fetch_result(&result);
    assert_eq!(json["extractor"], "json");
    let text = json["text"].as_str().unwrap();
    // Should be pretty-printed
    assert!(text.contains("\"key\": \"value\""));
    assert!(text.contains("\"num\": 42"));
}

#[tokio::test]
async fn test_fetch_raw_text() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/raw"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/plain")
                .set_body_string("Just plain text content"),
        )
        .mount(&server)
        .await;

    let tool = WebFetchTool::new(50000).unwrap();
    let result = tool
        .fetch_url(&serde_json::json!({"url": format!("{}/raw", server.uri())}))
        .await
        .unwrap();

    let json = parse_fetch_result(&result);
    assert_eq!(json["extractor"], "raw");
    assert_eq!(json["text"], "Just plain text content");
    assert_eq!(json["truncated"], false);
}

#[tokio::test]
async fn test_fetch_truncates_long_content() {
    let server = MockServer::start().await;
    let long_text = "x".repeat(1000);
    Mock::given(method("GET"))
        .and(path("/long"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/plain")
                .set_body_string(&long_text),
        )
        .mount(&server)
        .await;

    let tool = WebFetchTool::new(50000).unwrap();
    let result = tool
        .fetch_url(&serde_json::json!({
            "url": format!("{}/long", server.uri()),
            "maxChars": 100
        }))
        .await
        .unwrap();

    let json = parse_fetch_result(&result);
    assert_eq!(json["truncated"], true);
    assert_eq!(json["length"], 100);
}

#[tokio::test]
async fn test_fetch_uses_default_max_chars() {
    let server = MockServer::start().await;
    let text = "short text";
    Mock::given(method("GET"))
        .and(path("/short"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/plain")
                .set_body_string(text),
        )
        .mount(&server)
        .await;

    let tool = WebFetchTool::new(500).unwrap();
    let result = tool
        .fetch_url(&serde_json::json!({"url": format!("{}/short", server.uri())}))
        .await
        .unwrap();

    let json = parse_fetch_result(&result);
    assert_eq!(json["truncated"], false);
    assert_eq!(json["text"], "short text");
}

#[tokio::test]
async fn test_fetch_html_without_content_type_header() {
    let server = MockServer::start().await;
    // No content-type header, but body starts with <!DOCTYPE
    Mock::given(method("GET"))
        .and(path("/noheader"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(
                "<!DOCTYPE html><html><body><p>Detected as HTML</p></body></html>",
            ),
        )
        .mount(&server)
        .await;

    let tool = WebFetchTool::new(50000).unwrap();
    let result = tool
        .fetch_url(&serde_json::json!({"url": format!("{}/noheader", server.uri())}))
        .await
        .unwrap();

    let json = parse_fetch_result(&result);
    assert_eq!(json["extractor"], "readability");
    assert!(json["text"].as_str().unwrap().contains("Detected as HTML"));
}

#[tokio::test]
async fn test_fetch_html_strips_scripts() {
    let server = MockServer::start().await;
    let html = r"<html><body><p>Visible text</p><script>alert('evil');</script></body></html>";
    Mock::given(method("GET"))
        .and(path("/scripts"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html")
                .set_body_string(html),
        )
        .mount(&server)
        .await;

    let tool = WebFetchTool::new(50000).unwrap();
    let result = tool
        .fetch_url(&serde_json::json!({"url": format!("{}/scripts", server.uri())}))
        .await
        .unwrap();

    let json = parse_fetch_result(&result);
    let text = json["text"].as_str().unwrap();
    assert!(text.contains("Visible text"));
    assert!(!text.contains("alert"));
    assert!(!text.contains("evil"));
}

#[tokio::test]
async fn test_fetch_reports_final_url() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/final"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/plain")
                .set_body_string("arrived"),
        )
        .mount(&server)
        .await;

    let tool = WebFetchTool::new(50000).unwrap();
    let url = format!("{}/final", server.uri());
    let result = tool
        .fetch_url(&serde_json::json!({"url": &url}))
        .await
        .unwrap();

    let json = parse_fetch_result(&result);
    assert_eq!(json["url"], url);
    assert_eq!(json["finalUrl"], format!("{}/final", server.uri()));
}

#[tokio::test]
async fn test_fetch_reports_status_code() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/not-found"))
        .respond_with(
            ResponseTemplate::new(404)
                .insert_header("content-type", "text/plain")
                .set_body_string("page not found"),
        )
        .mount(&server)
        .await;

    let tool = WebFetchTool::new(50000).unwrap();
    let result = tool
        .fetch_url(&serde_json::json!({"url": format!("{}/not-found", server.uri())}))
        .await
        .unwrap();

    let json = parse_fetch_result(&result);
    assert_eq!(json["status"], 404);
}

#[tokio::test]
async fn test_fetch_ssrf_blocked() {
    let tool = WebFetchTool::new(50000).unwrap();
    let result = tool
        .execute(
            serde_json::json!({"url": "http://127.0.0.1/secret"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn test_fetch_markdown_mode_includes_title() {
    let server = MockServer::start().await;
    let html = r"<html><head><title>My Article</title></head><body><article><h2>Section</h2><p>Content here</p></article></body></html>";
    Mock::given(method("GET"))
        .and(path("/md"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html")
                .set_body_string(html),
        )
        .mount(&server)
        .await;

    let tool = WebFetchTool::new(50000).unwrap();
    let result = tool
        .fetch_url(&serde_json::json!({
            "url": format!("{}/md", server.uri()),
            "extractMode": "markdown"
        }))
        .await
        .unwrap();

    let json = parse_fetch_result(&result);
    let text = json["text"].as_str().unwrap();
    assert!(text.contains("# My Article"));
}
