use crate::config::TranscriptionConfig;
use anyhow::{bail, Context, Result};
use reqwest::Client;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct TranscriptionService {
    // Cloud backend
    client: Client,
    api_base: String,
    api_key: String,
    model: String,
    // Local backend
    whisper_ctx: Option<Arc<WhisperContext>>,
    prefer_local: bool,
    whisper_threads: u16,
}

/// Expand a leading `~/` in a path to the user's home directory.
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

impl TranscriptionService {
    /// Create a new transcription service from config.
    /// Returns `None` if transcription is not enabled or no backend is available.
    pub fn new(config: &TranscriptionConfig) -> Option<Self> {
        if !config.enabled {
            return None;
        }

        let has_cloud = !config.api_key.is_empty();

        // Try to load local whisper model
        let whisper_ctx = if config.local_model_path.is_empty() {
            None
        } else {
            let model_path = expand_tilde(&config.local_model_path);
            if model_path.exists() {
                info!("loading whisper model from {}", model_path.display());
                match WhisperContext::new_with_params(
                    model_path.to_str().unwrap_or_default(),
                    WhisperContextParameters::default(),
                ) {
                    Ok(ctx) => {
                        info!("whisper model loaded successfully");
                        Some(Arc::new(ctx))
                    }
                    Err(e) => {
                        warn!("failed to load whisper model: {}", e);
                        None
                    }
                }
            } else {
                warn!(
                    "whisper model not found at {}, local transcription disabled",
                    model_path.display()
                );
                None
            }
        };

        let has_local = whisper_ctx.is_some();

        // Need at least one backend
        if !has_cloud && !has_local {
            return None;
        }

        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_mins(1))
            .build()
            .unwrap_or_else(|_| Client::new());

        Some(Self {
            client,
            api_base: config.api_base.clone(),
            api_key: config.api_key.clone(),
            model: config.model.clone(),
            whisper_ctx,
            prefer_local: config.prefer_local,
            whisper_threads: config.threads,
        })
    }

    /// Transcribe an audio file, routing between local and cloud backends.
    pub async fn transcribe(&self, audio_path: &Path) -> Result<String> {
        if self.prefer_local {
            if let Some(ref ctx) = self.whisper_ctx {
                match self.transcribe_local(ctx, audio_path).await {
                    Ok(text) => return Ok(text),
                    Err(e) => warn!("local transcription failed, trying cloud: {}", e),
                }
            }
            if !self.api_key.is_empty() {
                return self.transcribe_cloud(audio_path).await;
            }
            bail!("local transcription failed and no cloud API configured")
        }
        // Cloud first, local fallback
        if !self.api_key.is_empty() {
            return self.transcribe_cloud(audio_path).await;
        }
        if let Some(ref ctx) = self.whisper_ctx {
            return self.transcribe_local(ctx, audio_path).await;
        }
        bail!("no transcription backend available")
    }

    /// Transcribe using the cloud Whisper API.
    async fn transcribe_cloud(&self, audio_path: &Path) -> Result<String> {
        let file_name = audio_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("audio.ogg")
            .to_string();
        let ext = audio_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("ogg");
        let mime_type = match ext {
            "mp3" => "audio/mpeg",
            "mp4" | "m4a" => "audio/mp4",
            "wav" => "audio/wav",
            "webm" => "audio/webm",
            "flac" => "audio/flac",
            _ => "audio/ogg",
        };

        let data = tokio::fs::read(audio_path)
            .await
            .with_context(|| format!("failed to read audio file: {}", audio_path.display()))?;

        debug!(
            "transcribing via cloud: {} ({}, {} bytes)",
            file_name,
            mime_type,
            data.len()
        );

        let file_part = reqwest::multipart::Part::bytes(data)
            .file_name(file_name)
            .mime_str(mime_type)?;
        let form = reqwest::multipart::Form::new()
            .part("file", file_part)
            .text("model", self.model.clone())
            .text("response_format", "json")
            .text("temperature", "0");

        let response = self
            .client
            .post(&self.api_base)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .send()
            .await
            .context("whisper API request failed")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("whisper API returned {}: {}", status, body);
        }

        let body: serde_json::Value = response
            .json()
            .await
            .context("failed to parse whisper API response")?;
        let text = body["text"].as_str().unwrap_or("").trim().to_string();

        if text.is_empty() {
            warn!("whisper API returned empty transcription");
        }

        Ok(text)
    }

    /// Transcribe locally using whisper-rs via ffmpeg PCM conversion.
    async fn transcribe_local(
        &self,
        ctx: &Arc<WhisperContext>,
        audio_path: &Path,
    ) -> Result<String> {
        debug!("transcribing locally: {}", audio_path.display());

        let pcm = convert_audio_to_pcm(audio_path).await?;
        let ctx = Arc::clone(ctx);
        let threads = self.whisper_threads;

        let text = tokio::task::spawn_blocking(move || -> Result<String> {
            let mut state = ctx
                .create_state()
                .context("failed to create whisper state")?;
            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            params.set_n_threads(i32::from(threads));
            params.set_language(Some("en"));
            params.set_print_special(false);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_timestamps(false);

            state
                .full(params, &pcm)
                .context("whisper inference failed")?;

            let num_segments = state.full_n_segments();
            let mut result = String::new();
            for i in 0..num_segments {
                if let Some(segment) = state.get_segment(i) {
                    if let Ok(text) = segment.to_str_lossy() {
                        result.push_str(&text);
                    }
                }
            }

            Ok(result.trim().to_string())
        })
        .await
        .context("whisper task panicked")??;

        if text.is_empty() {
            warn!("local whisper returned empty transcription");
        } else {
            debug!("local transcription: {} chars", text.len());
        }

        Ok(text)
    }
}

/// Convert an audio file to 16kHz mono f32 PCM using ffmpeg.
async fn convert_audio_to_pcm(audio_path: &Path) -> Result<Vec<f32>> {
    let output = tokio::process::Command::new("ffmpeg")
        .args([
            "-i",
            audio_path
                .to_str()
                .context("audio path is not valid UTF-8")?,
            "-ar",
            "16000",
            "-ac",
            "1",
            "-f",
            "f32le",
            "-hide_banner",
            "-loglevel",
            "error",
            "pipe:1",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .context("failed to run ffmpeg — is it installed?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("ffmpeg failed: {}", stderr.trim());
    }

    let bytes = &output.stdout;
    if bytes.len() % 4 != 0 {
        bail!(
            "ffmpeg output has unexpected length ({} bytes, not a multiple of 4)",
            bytes.len()
        );
    }

    let pcm: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();

    debug!(
        "converted audio to PCM: {} samples ({:.1}s at 16kHz)",
        pcm.len(),
        pcm.len() as f64 / 16000.0
    );

    Ok(pcm)
}
