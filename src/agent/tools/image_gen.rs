use crate::agent::tools::base::ExecutionContext;
use crate::agent::tools::{Tool, ToolResult, ToolVersion};
use anyhow::Result;
use async_trait::async_trait;
use base64::Engine;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;
use tracing::debug;

const OPENAI_API: &str = "https://api.openai.com/v1/images/generations";
const GOOGLE_API: &str =
    "https://generativelanguage.googleapis.com/v1beta/models/imagen-3.0-generate-002:predict";

pub struct ImageGenTool {
    openai_api_key: Option<String>,
    google_api_key: Option<String>,
    default_provider: String,
    openai_base_url: String,
    google_base_url: String,
    client: Client,
}

impl ImageGenTool {
    pub fn new(
        openai_api_key: Option<String>,
        google_api_key: Option<String>,
        default_provider: String,
    ) -> Self {
        Self {
            openai_api_key,
            google_api_key,
            default_provider,
            openai_base_url: OPENAI_API.to_string(),
            google_base_url: GOOGLE_API.to_string(),
            client: Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_mins(2))
                .build()
                .unwrap_or_else(|_| Client::new()),
        }
    }

    #[cfg(test)]
    fn with_base_urls(
        openai_api_key: Option<String>,
        google_api_key: Option<String>,
        default_provider: String,
        openai_base_url: String,
        google_base_url: String,
    ) -> Self {
        Self {
            openai_api_key,
            google_api_key,
            default_provider,
            openai_base_url,
            google_base_url,
            client: Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| Client::new()),
        }
    }

    /// Resolve which provider to use. If explicitly requested, validate its key
    /// exists. Otherwise, prefer the configured default, falling back to whichever
    /// has a key.
    fn resolve_provider(&self, requested: Option<&str>) -> Result<&str, String> {
        if let Some(provider) = requested {
            match provider {
                "openai" => {
                    if self.openai_api_key.is_some() {
                        Ok("openai")
                    } else {
                        Err("OpenAI provider requested but no API key configured".to_string())
                    }
                }
                "google" => {
                    if self.google_api_key.is_some() {
                        Ok("google")
                    } else {
                        Err("Google provider requested but no API key configured".to_string())
                    }
                }
                _ => Err(format!(
                    "Unknown provider '{}'. Use 'openai' or 'google'",
                    provider
                )),
            }
        } else {
            // Try default first, then fallback to whichever has a key
            let has_openai = self.openai_api_key.is_some();
            let has_google = self.google_api_key.is_some();
            if self.default_provider == "openai" && has_openai {
                Ok("openai")
            } else if self.default_provider == "google" && has_google {
                Ok("google")
            } else if has_openai {
                Ok("openai")
            } else if has_google {
                Ok("google")
            } else {
                Err(
                    "No image generation provider configured (need OpenAI or Gemini API key)"
                        .to_string(),
                )
            }
        }
    }

    async fn generate_openai(&self, prompt: &str, size: &str, quality: &str) -> Result<String> {
        let api_key = self
            .openai_api_key
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing OpenAI API key"))?;

        debug!(
            "generating image via OpenAI: size={}, quality={}",
            size, quality
        );

        let body = serde_json::json!({
            "model": "gpt-image-1",
            "prompt": prompt,
            "n": 1,
            "size": size,
            "quality": quality,
            "output_format": "png"
        });

        let resp = self
            .client
            .post(&self.openai_base_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let json: Value = resp.json().await?;

        if !status.is_success() {
            let msg = json["error"]["message"].as_str().unwrap_or("unknown error");
            anyhow::bail!("OpenAI image generation failed: {}", msg);
        }

        let b64 = json["data"][0]["b64_json"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing b64_json in OpenAI response"))?;

        let bytes = base64::engine::general_purpose::STANDARD.decode(b64)?;
        let path = crate::utils::media::save_media_file(&bytes, "imagegen", "png")?;
        Ok(path)
    }

    async fn generate_google(&self, prompt: &str, aspect_ratio: &str) -> Result<String> {
        let api_key = self
            .google_api_key
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing Google API key"))?;

        debug!(
            "generating image via Google Imagen: aspect_ratio={}",
            aspect_ratio
        );

        let url = format!("{}?key={}", self.google_base_url, api_key);
        let body = serde_json::json!({
            "instances": [{ "prompt": prompt }],
            "parameters": {
                "sampleCount": 1,
                "aspectRatio": aspect_ratio
            }
        });

        let resp = self.client.post(&url).json(&body).send().await?;

        let status = resp.status();
        let json: Value = resp.json().await?;

        if !status.is_success() {
            let msg = json["error"]["message"].as_str().unwrap_or("unknown error");
            anyhow::bail!("Google Imagen failed: {}", msg);
        }

        let b64 = json["predictions"][0]["bytesBase64Encoded"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing bytesBase64Encoded in Google response"))?;

        let mime = json["predictions"][0]["mimeType"]
            .as_str()
            .unwrap_or("image/png");
        let ext = if mime.contains("jpeg") || mime.contains("jpg") {
            "jpg"
        } else {
            "png"
        };

        let bytes = base64::engine::general_purpose::STANDARD.decode(b64)?;
        let path = crate::utils::media::save_media_file(&bytes, "imagegen", ext)?;
        Ok(path)
    }
}

#[async_trait]
impl Tool for ImageGenTool {
    fn name(&self) -> &'static str {
        "image_gen"
    }

    fn description(&self) -> &'static str {
        "Generate images from text prompts using AI (OpenAI gpt-image-1 or Google Imagen 3)"
    }

    fn version(&self) -> ToolVersion {
        ToolVersion::new(1, 0, 0)
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "Text description of the image to generate"
                },
                "provider": {
                    "type": "string",
                    "enum": ["openai", "google"],
                    "description": "Which provider to use. Defaults to configured default."
                },
                "size": {
                    "type": "string",
                    "enum": ["1024x1024", "1024x1536", "1536x1024"],
                    "description": "Image size (OpenAI only, default 1024x1024)"
                },
                "aspect_ratio": {
                    "type": "string",
                    "enum": ["1:1", "3:4", "4:3", "9:16", "16:9"],
                    "description": "Aspect ratio (Google only, default 1:1)"
                },
                "quality": {
                    "type": "string",
                    "enum": ["low", "medium", "high"],
                    "description": "Image quality (OpenAI only, default medium)"
                }
            },
            "required": ["prompt"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let Some(prompt) = params["prompt"].as_str() else {
            return Ok(ToolResult::error(
                "Missing required 'prompt' parameter".to_string(),
            ));
        };

        let provider = match self.resolve_provider(params["provider"].as_str()) {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult::error(e)),
        };

        let result = match provider {
            "openai" => {
                let size = params["size"].as_str().unwrap_or("1024x1024");
                let quality = params["quality"].as_str().unwrap_or("medium");
                self.generate_openai(prompt, size, quality).await
            }
            "google" => {
                let aspect_ratio = params["aspect_ratio"].as_str().unwrap_or("1:1");
                self.generate_google(prompt, aspect_ratio).await
            }
            _ => unreachable!(),
        };

        match result {
            Ok(path) => Ok(ToolResult::new(format!("Image saved to: {}", path))),
            Err(e) => Ok(ToolResult::error(format!("Image generation failed: {}", e))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
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
        assert!(result.content.contains("No image generation provider"));
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
        assert!(result.content.contains("Unknown provider"));
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
            .and(query_param("key", "test_google_key"))
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
}
