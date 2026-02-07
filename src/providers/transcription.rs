use anyhow::Result;
use std::path::Path;
use tracing::warn;

#[allow(dead_code)] // Provider for future transcription features
pub struct GroqTranscriptionProvider {
    api_key: Option<String>,
}

impl GroqTranscriptionProvider {
    #[allow(dead_code)] // Provider for future transcription features
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            api_key: api_key.or_else(|| std::env::var("GROQ_API_KEY").ok()),
        }
    }

    #[allow(dead_code)] // Provider for future transcription features
    pub async fn transcribe(&self, file_path: impl AsRef<Path>) -> Result<String> {
        let api_key = match &self.api_key {
            Some(k) => k,
            None => {
                warn!("Groq API key not configured for transcription");
                return Ok(String::new());
            }
        };

        let path = file_path.as_ref();
        if !path.exists() {
            return Err(anyhow::anyhow!("Audio file not found: {}", path.display()));
        }

        let client = reqwest::Client::new();
        let file_bytes = std::fs::read(path)?;
        let file_name = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("audio");

        let form = reqwest::multipart::Form::new()
            .part("file", reqwest::multipart::Part::bytes(file_bytes).file_name(file_name.to_string()))
            .text("model", "whisper-large-v3");

        let response = client
            .post("https://api.groq.com/openai/v1/audio/transcriptions")
            .header("Authorization", format!("Bearer {}", api_key))
            .multipart(form)
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await?;

        let data: serde_json::Value = response.error_for_status()?.json().await?;
        Ok(data["text"].as_str().unwrap_or("").to_string())
    }
}
