use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities};
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
            client: crate::utils::http::default_http_client(),
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
                    "unknown provider '{}'. Use 'openai' or 'google'",
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
                    "no image generation provider configured (need OpenAI or Gemini API key)"
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
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(String::from))
                .unwrap_or_else(|| format!("HTTP {status}"));
            anyhow::bail!("OpenAI image generation failed: {}", msg);
        }

        let json: Value = resp.json().await?;

        let data = json
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("OpenAI response missing 'data' array"))?;
        let b64 = data
            .first()
            .and_then(|d| d["b64_json"].as_str())
            .ok_or_else(|| {
                anyhow::anyhow!("OpenAI response 'data' array is empty or missing b64_json")
            })?;

        // Check base64 length before decoding to prevent OOM
        if b64.len() > 30 * 1024 * 1024 {
            anyhow::bail!("image data too large ({} bytes encoded)", b64.len());
        }
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

        let body = serde_json::json!({
            "instances": [{ "prompt": prompt }],
            "parameters": {
                "sampleCount": 1,
                "aspectRatio": aspect_ratio
            }
        });

        let resp = self
            .client
            .post(&self.google_base_url)
            .header("x-goog-api-key", api_key)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(String::from))
                .unwrap_or_else(|| format!("HTTP {status}"));
            anyhow::bail!("Google Imagen failed: {}", msg);
        }

        let json: Value = resp.json().await?;

        let predictions = json
            .get("predictions")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("Google response missing 'predictions' array"))?;
        let prediction = predictions
            .first()
            .ok_or_else(|| anyhow::anyhow!("Google response 'predictions' array is empty"))?;
        let b64 = prediction["bytesBase64Encoded"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing bytesBase64Encoded in Google response"))?;

        let mime = prediction["mimeType"].as_str().unwrap_or("image/png");
        let ext = if mime.contains("jpeg") || mime.contains("jpg") {
            "jpg"
        } else {
            "png"
        };

        // Check base64 length before decoding to prevent OOM
        if b64.len() > 30 * 1024 * 1024 {
            anyhow::bail!("image data too large ({} bytes encoded)", b64.len());
        }
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

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: true,
            subagent_access: SubagentAccess::Denied,
            actions: vec![],
        }
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
                "missing required 'prompt' parameter".to_string(),
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
            other => {
                return Ok(ToolResult::error(format!(
                    "unsupported image provider: '{}'",
                    other
                )));
            }
        };

        match result {
            Ok(path) => Ok(ToolResult::new(format!(
                "Image saved to: {}",
                crate::utils::path_sanitize::sanitize_path(std::path::Path::new(&path), None,)
            ))),
            Err(e) => Ok(ToolResult::error(format!("image generation failed: {}", e))),
        }
    }
}

#[cfg(test)]
mod tests;
