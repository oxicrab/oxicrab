use super::*;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn make_png_bytes() -> Vec<u8> {
    // Minimal valid PNG header
    vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
}

// --- Parameter validation ---

#[tokio::test]
async fn test_missing_prompt() {
    let tool = ImageGenTool::new(Some("key".into()), None, "openai".into());
    let result = tool
        .execute(serde_json::json!({}), &ExecutionContext::default())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("prompt"));
}

#[tokio::test]
async fn test_no_providers_configured() {
    let tool = ImageGenTool::new(None, None, "openai".into());
    let result = tool
        .execute(
            serde_json::json!({"prompt": "a cat"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("no image generation provider"));
}

#[tokio::test]
async fn test_unknown_provider() {
    let tool = ImageGenTool::new(Some("key".into()), None, "openai".into());
    let result = tool
        .execute(
            serde_json::json!({"prompt": "a cat", "provider": "dalle"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("unknown provider"));
}

#[tokio::test]
async fn test_requested_provider_no_key() {
    let tool = ImageGenTool::new(Some("key".into()), None, "openai".into());
    let result = tool
        .execute(
            serde_json::json!({"prompt": "a cat", "provider": "google"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("no API key"));
}

// --- Provider auto-selection ---

#[test]
fn test_auto_select_prefers_default() {
    let tool = ImageGenTool::new(Some("ok".into()), Some("gk".into()), "google".into());
    assert_eq!(tool.resolve_provider(None).unwrap(), "google");
}

#[test]
fn test_auto_select_fallback() {
    // Default is openai but no openai key — should fall back to google
    let tool = ImageGenTool::new(None, Some("gk".into()), "openai".into());
    assert_eq!(tool.resolve_provider(None).unwrap(), "google");
}

// --- OpenAI wiremock ---

#[tokio::test]
async fn test_openai_success() {
    let server = MockServer::start().await;
    let png = make_png_bytes();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);

    Mock::given(method("POST"))
        .and(path("/"))
        .and(header("Authorization", "Bearer test_key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{ "b64_json": b64 }]
        })))
        .mount(&server)
        .await;

    let tool = ImageGenTool::with_base_urls(
        Some("test_key".into()),
        None,
        "openai".into(),
        server.uri(),
        String::new(),
    );
    let result = tool
        .execute(
            serde_json::json!({"prompt": "a sunset"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error, "unexpected error: {}", result.content);
    assert!(result.content.contains("saved to:"));
    assert!(result.content.contains("imagegen"));

    // Clean up generated file
    if let Some(idx) = result.content.find("saved to: ") {
        let path = result.content[idx + 10..].trim();
        let _ = std::fs::remove_file(path);
    }
}

#[tokio::test]
async fn test_openai_api_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": {
                "message": "content policy violation",
                "type": "invalid_request_error"
            }
        })))
        .mount(&server)
        .await;

    let tool = ImageGenTool::with_base_urls(
        Some("test_key".into()),
        None,
        "openai".into(),
        server.uri(),
        String::new(),
    );
    let result = tool
        .execute(
            serde_json::json!({"prompt": "bad prompt"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.content.contains("content policy violation"));
}

// --- Google wiremock ---

#[tokio::test]
async fn test_google_success() {
    let server = MockServer::start().await;
    let png = make_png_bytes();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);

    Mock::given(method("POST"))
        .and(path("/"))
        .and(header("x-goog-api-key", "test_google_key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "predictions": [{
                "bytesBase64Encoded": b64,
                "mimeType": "image/png"
            }]
        })))
        .mount(&server)
        .await;

    let tool = ImageGenTool::with_base_urls(
        None,
        Some("test_google_key".into()),
        "google".into(),
        String::new(),
        server.uri(),
    );
    let result = tool
        .execute(
            serde_json::json!({"prompt": "a mountain"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error, "unexpected error: {}", result.content);
    assert!(result.content.contains("saved to:"));

    if let Some(idx) = result.content.find("saved to: ") {
        let path = result.content[idx + 10..].trim();
        let _ = std::fs::remove_file(path);
    }
}

#[tokio::test]
async fn test_google_api_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": {
                "message": "safety filter triggered",
                "status": "INVALID_ARGUMENT"
            }
        })))
        .mount(&server)
        .await;

    let tool = ImageGenTool::with_base_urls(
        None,
        Some("test_google_key".into()),
        "google".into(),
        String::new(),
        server.uri(),
    );
    let result = tool
        .execute(
            serde_json::json!({"prompt": "bad prompt"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.content.contains("safety filter triggered"));
}

#[tokio::test]
async fn test_openai_with_custom_params() {
    let server = MockServer::start().await;
    let png = make_png_bytes();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);

    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{ "b64_json": b64 }]
        })))
        .mount(&server)
        .await;

    let tool = ImageGenTool::with_base_urls(
        Some("key".into()),
        None,
        "openai".into(),
        server.uri(),
        String::new(),
    );
    let result = tool
        .execute(
            serde_json::json!({
                "prompt": "a cat",
                "size": "1536x1024",
                "quality": "high"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error, "unexpected error: {}", result.content);

    if let Some(idx) = result.content.find("saved to: ") {
        let path = result.content[idx + 10..].trim();
        let _ = std::fs::remove_file(path);
    }
}

#[tokio::test]
async fn test_google_with_aspect_ratio() {
    let server = MockServer::start().await;
    let png = make_png_bytes();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);

    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "predictions": [{
                "bytesBase64Encoded": b64,
                "mimeType": "image/png"
            }]
        })))
        .mount(&server)
        .await;

    let tool = ImageGenTool::with_base_urls(
        None,
        Some("gk".into()),
        "google".into(),
        String::new(),
        server.uri(),
    );
    let result = tool
        .execute(
            serde_json::json!({
                "prompt": "a landscape",
                "aspect_ratio": "16:9"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error, "unexpected error: {}", result.content);

    if let Some(idx) = result.content.find("saved to: ") {
        let path = result.content[idx + 10..].trim();
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn test_image_gen_capabilities() {
    use crate::agent::tools::base::SubagentAccess;
    let tool = ImageGenTool::new(None, None, "openai".into());
    let caps = tool.capabilities();
    assert!(caps.built_in);
    assert!(caps.network_outbound);
    assert_eq!(caps.subagent_access, SubagentAccess::Denied);
    assert!(caps.actions.is_empty());
}
