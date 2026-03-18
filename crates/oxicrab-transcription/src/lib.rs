use anyhow::{Context, Result, bail};
use oxicrab_core::config::schema::TranscriptionConfig;
use reqwest::Client;
use std::path::Path;
#[cfg(feature = "local-whisper")]
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs;
use tracing::{debug, info, warn};

#[cfg(feature = "local-whisper")]
const WHISPER_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin";
#[cfg(feature = "local-whisper")]
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct TranscriptionService {
    // Cloud backend
    client: Client,
    api_base: String,
    api_key: String,
    model: String,
    // Local backend
    #[cfg(feature = "local-whisper")]
    whisper_ctx: Option<Arc<WhisperContext>>,
    prefer_local: bool,
    #[cfg(feature = "local-whisper")]
    whisper_threads: u16,
}

/// Download the whisper GGML model to the given path.
/// Writes to a `.tmp` sibling first, then atomically renames.
#[cfg(feature = "local-whisper")]
fn download_model(dest: &Path) -> anyhow::Result<()> {
    use std::io::{Read, Write};

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }

    let tmp_path = dest.with_extension("bin.tmp");
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_mins(10))
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new());

    let mut response = client
        .get(WHISPER_MODEL_URL)
        .send()
        .context("failed to start whisper model download")?;

    if !response.status().is_success() {
        bail!("whisper model download returned HTTP {}", response.status());
    }

    let mut file = std::fs::File::create(&tmp_path)
        .with_context(|| format!("failed to create {}", tmp_path.display()))?;

    let mut downloaded: u64 = 0;
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = response
            .read(&mut buf)
            .context("error reading whisper model download")?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .context("error writing whisper model to disk")?;
        downloaded += n as u64;
        // Log progress every ~100MB
        if downloaded % (100 * 1024 * 1024) < (256 * 1024) as u64 {
            info!("whisper model download: {}MB", downloaded / (1024 * 1024));
        }
    }

    file.flush().context("failed to flush whisper model file")?;
    drop(file);

    std::fs::rename(&tmp_path, dest).with_context(|| {
        format!(
            "failed to rename {} to {}",
            tmp_path.display(),
            dest.display()
        )
    })?;

    info!(
        "whisper model saved to {} ({}MB)",
        dest.display(),
        downloaded / (1024 * 1024)
    );
    Ok(())
}

/// Expand a leading `~/` in a path to the user's home directory.
#[cfg(feature = "local-whisper")]
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
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
        #[cfg(feature = "local-whisper")]
        let whisper_ctx = if config.local_model_path.is_empty() {
            None
        } else {
            let model_path = expand_tilde(&config.local_model_path);
            if model_path.exists() {
                info!("loading whisper model from {}", model_path.display());
                let mut ctx_params = WhisperContextParameters::default();
                ctx_params.use_gpu(false);
                let Some(path_str) = model_path.to_str() else {
                    warn!(
                        "whisper model path is not valid UTF-8: {}",
                        model_path.display()
                    );
                    return None;
                };
                match WhisperContext::new_with_params(path_str, ctx_params) {
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
                info!(
                    "whisper model not found at {}, attempting download (~547MB)",
                    model_path.display()
                );
                match download_model(&model_path) {
                    Ok(()) => {
                        info!("whisper model downloaded, loading");
                        let mut ctx_params = WhisperContextParameters::default();
                        ctx_params.use_gpu(false);
                        let Some(path_str) = model_path.to_str() else {
                            warn!(
                                "whisper model path is not valid UTF-8: {}",
                                model_path.display()
                            );
                            return None;
                        };
                        match WhisperContext::new_with_params(path_str, ctx_params) {
                            Ok(ctx) => {
                                info!("whisper model loaded successfully");
                                Some(Arc::new(ctx))
                            }
                            Err(e) => {
                                warn!("failed to load whisper model: {}", e);
                                None
                            }
                        }
                    }
                    Err(e) => {
                        warn!(
                            "whisper model download failed: {}, local transcription disabled",
                            e
                        );
                        None
                    }
                }
            }
        };

        #[cfg(feature = "local-whisper")]
        let has_local = whisper_ctx.is_some();
        #[cfg(not(feature = "local-whisper"))]
        let has_local = false;

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
            #[cfg(feature = "local-whisper")]
            whisper_ctx,
            prefer_local: config.prefer_local,
            #[cfg(feature = "local-whisper")]
            whisper_threads: config.threads,
        })
    }

    /// Transcribe an audio file, routing between local and cloud backends.
    pub async fn transcribe(&self, audio_path: &Path) -> Result<String> {
        if self.prefer_local {
            #[cfg(feature = "local-whisper")]
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
        #[cfg(feature = "local-whisper")]
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

        let metadata = fs::metadata(audio_path)
            .await
            .with_context(|| format!("failed to stat audio file: {}", audio_path.display()))?;
        if metadata.len() > MAX_AUDIO_FILE_BYTES {
            bail!(
                "audio file too large ({} bytes, max {})",
                metadata.len(),
                MAX_AUDIO_FILE_BYTES
            );
        }

        let data = fs::read(audio_path)
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
            bail!("whisper API returned {status}: {body}");
        }

        let body: serde_json::Value = response
            .json()
            .await
            .context("failed to parse whisper API response")?;
        let text = body["text"].as_str().unwrap_or_default().trim().to_string();

        if text.is_empty() {
            warn!("whisper API returned empty transcription");
        }

        Ok(text)
    }

    /// Transcribe locally using whisper-rs via ffmpeg PCM conversion.
    #[cfg(feature = "local-whisper")]
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
                let Some(segment) = state.get_segment(i) else {
                    warn!("whisper segment {i} out of bounds");
                    continue;
                };
                match segment.to_str_lossy() {
                    Ok(text) => result.push_str(&text),
                    Err(e) => warn!("whisper segment {i} text unavailable: {e}"),
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

/// Lazy wrapper that initializes the transcription service in a background task.
/// The whisper model load (~200ms) doesn't block startup; callers fall back to
/// `strip_audio_tags()` until initialization completes.
pub struct LazyTranscriptionService {
    cell: Arc<tokio::sync::OnceCell<TranscriptionService>>,
}

impl LazyTranscriptionService {
    /// Spawn background initialization of the transcription service.
    pub fn new(config: TranscriptionConfig) -> Self {
        let cell = Arc::new(tokio::sync::OnceCell::new());
        let cell_clone = cell.clone();
        tokio::spawn(async move {
            match tokio::task::spawn_blocking(move || TranscriptionService::new(&config)).await {
                Ok(Some(svc)) => {
                    let _ = cell_clone.set(svc);
                    info!("transcription service initialized (background)");
                }
                Ok(None) => debug!("transcription service not available"),
                Err(e) => warn!("transcription init panicked: {}", e),
            }
        });
        Self { cell }
    }

    /// Get the service if ready, None if still initializing.
    pub fn get(&self) -> Option<&TranscriptionService> {
        self.cell.get()
    }
}

/// Maximum audio file size for cloud upload (25 MB, Whisper API limit).
const MAX_AUDIO_FILE_BYTES: u64 = 25 * 1024 * 1024;

/// Maximum PCM data size from ffmpeg (50 MB).
#[cfg(feature = "local-whisper")]
const MAX_PCM_BYTES: usize = 50 * 1024 * 1024;

/// Convert an audio file to 16kHz mono f32 PCM using ffmpeg.
#[cfg(feature = "local-whisper")]
async fn convert_audio_to_pcm(audio_path: &Path) -> Result<Vec<f32>> {
    let output = scrubbed_command("ffmpeg")
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
    if bytes.len() > MAX_PCM_BYTES {
        bail!(
            "ffmpeg PCM output too large ({} bytes, max {})",
            bytes.len(),
            MAX_PCM_BYTES
        );
    }
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

/// Create a `Command` with a scrubbed environment (only allowlisted env vars).
#[cfg(feature = "local-whisper")]
fn scrubbed_command(program: &str) -> tokio::process::Command {
    const ALLOWED_ENV_VARS: &[&str] = &[
        "PATH",
        "HOME",
        "USER",
        "LANG",
        "LC_ALL",
        "TZ",
        "TERM",
        "RUST_LOG",
        "TMPDIR",
        "XDG_RUNTIME_DIR",
    ];
    let mut cmd = tokio::process::Command::new(program);
    cmd.env_clear();
    for &var in ALLOWED_ENV_VARS {
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }
    cmd
}

#[cfg(test)]
mod tests;
