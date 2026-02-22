use crate::providers::anthropic_common;
use crate::providers::base::{ChatRequest, LLMProvider, LLMResponse};
use crate::providers::errors::ProviderErrorHandler;
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

const TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";
const API_URL: &str = "https://api.anthropic.com/v1/messages";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";

/// Default `expires_in` when the OAuth response omits the field (1 hour).
const DEFAULT_EXPIRES_IN_SECS: u64 = 3600;

// Headers that identify the request as a Claude Code client
fn claude_code_headers() -> Vec<(&'static str, &'static str)> {
    vec![
        ("anthropic-version", "2023-06-01"),
        ("anthropic-beta", "claude-code-20250219,oauth-2025-04-20"),
        ("user-agent", "claude-cli/2.1.2 (external, cli)"),
        ("x-app", "cli"),
        ("anthropic-dangerous-direct-browser-access", "true"),
        ("accept", "application/json"),
        ("content-type", "application/json"),
    ]
}

pub struct AnthropicOAuthProvider {
    access_token: Arc<Mutex<String>>,
    refresh_token: Arc<Mutex<String>>,
    expires_at: Arc<Mutex<i64>>,
    default_model: String,
    credentials_path: Option<PathBuf>,
    client: Client,
}

impl AnthropicOAuthProvider {
    pub fn new(
        access_token: String,
        refresh_token: String,
        expires_at: i64,
        default_model: Option<String>,
        credentials_path: Option<PathBuf>,
    ) -> Result<Self> {
        let client = Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .timeout(std::time::Duration::from_mins(2))
            .build()
            .context("Failed to create HTTP client for AnthropicOAuthProvider")?;

        let provider = Self {
            access_token: Arc::new(Mutex::new(access_token)),
            refresh_token: Arc::new(Mutex::new(refresh_token)),
            expires_at: Arc::new(Mutex::new(expires_at)),
            default_model: default_model.map_or_else(
                || "claude-opus-4-6".to_string(),
                |m| {
                    // Strip provider prefix for API usage
                    if let Some(stripped) = m.strip_prefix("anthropic/") {
                        stripped.to_string()
                    } else {
                        m
                    }
                },
            ),
            credentials_path,
            client,
        };

        // Load cached tokens if available (synchronous — called before tokio runtime
        // is running for this provider, so std::sync primitives are fine)
        if let Some(ref path) = provider.credentials_path {
            provider.load_cached_tokens(path);
        }

        Ok(provider)
    }

    fn load_cached_tokens(&self, path: &Path) {
        if !path.exists() {
            return;
        }

        // Acquire shared lock for consistent reads (save_credentials holds exclusive)
        let _lock = (|| -> Option<std::fs::File> {
            let lock_path = path.with_extension("json.lock");
            let lock_file = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&lock_path)
                .ok()?;
            fs2::FileExt::lock_shared(&lock_file).ok()?;
            Some(lock_file)
        })();

        match std::fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str::<Value>(&content) {
                Ok(data) => {
                    if let Some(cached_at) = data.get("expires_at").and_then(Value::as_i64) {
                        // Use try_lock since we're in a sync context during construction.
                        // These locks are uncontested at this point (single-threaded init).
                        let current_expires = {
                            let Ok(guard) = self.expires_at.try_lock() else {
                                warn!(
                                    "could not acquire OAuth token lock during init, skipping cache load"
                                );
                                return;
                            };
                            *guard
                            // guard dropped here before acquiring other locks
                        };
                        if cached_at > current_expires
                            && let (Some(access), Some(refresh)) = (
                                data.get("access_token").and_then(Value::as_str),
                                data.get("refresh_token").and_then(Value::as_str),
                            )
                        {
                            if let Ok(mut guard) = self.access_token.try_lock() {
                                *guard = access.to_string();
                            } else {
                                warn!("could not acquire access_token lock during init");
                            }
                            if let Ok(mut guard) = self.refresh_token.try_lock() {
                                *guard = refresh.to_string();
                            } else {
                                warn!("could not acquire refresh_token lock during init");
                            }
                            if let Ok(mut guard) = self.expires_at.try_lock() {
                                *guard = cached_at;
                            } else {
                                warn!("could not acquire expires_at lock during init");
                            }
                            info!("Loaded refreshed OAuth tokens from cache");
                        }
                    }
                }
                Err(e) => {
                    debug!("No cached OAuth tokens: {}", e);
                }
            },
            Err(e) => {
                debug!("Failed to read cached tokens: {}", e);
            }
        }
    }

    async fn ensure_valid_token(&self) -> Result<String> {
        // Read expires_at and drop lock before potentially refreshing.
        // refresh_token_internal() takes its own locks, so we must not hold one here.
        // Minor race: two concurrent callers may both see expired and refresh.
        // This is harmless — the second refresh just wastes one API call.
        let needs_refresh = {
            let expires_at = *self.expires_at.lock().await;
            if expires_at > 0 {
                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .context("System time is before UNIX epoch")
                    .map(|d| d.as_millis() as i64)?;
                now_ms > expires_at
            } else {
                false
            }
        };

        if needs_refresh {
            let refresh_token = self.refresh_token.lock().await.clone();
            if !refresh_token.is_empty() {
                // Re-check after acquiring refresh token (another caller may have refreshed)
                let still_expired = {
                    let expires_at = *self.expires_at.lock().await;
                    let now_ms = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_or(0, |d| d.as_millis() as i64);
                    now_ms > expires_at
                };
                if still_expired {
                    info!("OAuth token expired, refreshing...");
                    match self.refresh_token_internal(&refresh_token).await {
                        Ok(()) => {
                            info!("OAuth token refreshed successfully");
                        }
                        Err(e) => {
                            warn!("Token refresh failed: {}, using existing token", e);
                        }
                    }
                }
            }
        }

        Ok(self.access_token.lock().await.clone())
    }

    async fn refresh_token_internal(&self, refresh_token: &str) -> Result<()> {
        let payload = json!({
            "grant_type": "refresh_token",
            "client_id": CLIENT_ID,
            "refresh_token": refresh_token,
        });

        let resp = self
            .client
            .post(TOKEN_URL)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .context("Failed to refresh OAuth token")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("OAuth token refresh failed (HTTP {}): {}", status, body);
        }

        let data: Value = resp
            .json()
            .await
            .context("Failed to parse refresh response")?;

        let access_token = data["access_token"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing access_token in refresh response"))?
            .to_string();

        let new_refresh_token = data
            .get("refresh_token")
            .and_then(Value::as_str)
            .map_or_else(|| refresh_token.to_string(), ToString::to_string);

        // expires_in is in seconds, store as ms with 5min buffer
        let expires_in_secs = data["expires_in"]
            .as_u64()
            .unwrap_or(DEFAULT_EXPIRES_IN_SECS);
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("System time is before UNIX epoch")
            .map(|d| d.as_millis() as i64)?;
        let expires_at = now_ms + (expires_in_secs * 1000) as i64 - 300_000;

        // Update all three fields atomically (hold all locks at once)
        let (mut at_guard, mut rt_guard, mut ea_guard) = tokio::join!(
            self.access_token.lock(),
            self.refresh_token.lock(),
            self.expires_at.lock()
        );
        *at_guard = access_token;
        *rt_guard = new_refresh_token;
        *ea_guard = expires_at;
        drop((at_guard, rt_guard, ea_guard));

        // Persist refreshed credentials if path is configured
        if let Some(ref path) = self.credentials_path {
            self.save_credentials(path).await;
        }

        Ok(())
    }

    async fn send_chat_request(&self, token: &str, payload: &Value) -> Result<reqwest::Response> {
        let mut request = self
            .client
            .post(API_URL)
            .header("Authorization", format!("Bearer {}", token));

        for (key, value) in claude_code_headers() {
            request = request.header(key, value);
        }

        request
            .json(payload)
            .send()
            .await
            .context("failed to send request to Anthropic OAuth API")
    }

    async fn save_credentials(&self, path: &Path) {
        let data = json!({
            "access_token": *self.access_token.lock().await,
            "refresh_token": *self.refresh_token.lock().await,
            "expires_at": *self.expires_at.lock().await,
        });

        let json_str = match serde_json::to_string_pretty(&data) {
            Ok(s) => s,
            Err(e) => {
                warn!("failed to serialize OAuth credentials: {}", e);
                return;
            }
        };
        let path = path.to_path_buf();

        // Perform all blocking I/O off the async runtime
        let result = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // Cross-process lock to prevent concurrent token refresh races.
            // Hold the lock through the write to prevent TOCTOU races.
            let lock_path = path.with_extension("json.lock");
            let _lock_file = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&lock_path)
                .ok()
                .and_then(|f| fs2::FileExt::lock_exclusive(&f).ok().map(|()| f));
            crate::utils::atomic_write(&path, &json_str)?;
            // Restrict permissions to owner-only (0o600) to protect tokens
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
            Ok(())
        })
        .await;

        match result {
            Ok(Ok(())) => debug!("OAuth credentials saved"),
            Ok(Err(e)) => warn!("Failed to save OAuth credentials: {}", e),
            Err(e) => warn!("Failed to spawn credential save task: {}", e),
        }
    }

    pub fn from_credentials_file(
        path: &Path,
        default_model: Option<String>,
    ) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }

        // Acquire shared lock for consistent reads (save_credentials holds exclusive)
        let _lock = (|| -> Option<std::fs::File> {
            let lock_path = path.with_extension("json.lock");
            let lock_file = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&lock_path)
                .ok()?;
            fs2::FileExt::lock_shared(&lock_file).ok()?;
            Some(lock_file)
        })();

        let content = std::fs::read_to_string(path).context("Failed to read credentials file")?;

        let data: Value =
            serde_json::from_str(&content).context("Failed to parse credentials file")?;

        let access_token = data["access_token"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing access_token"))?
            .to_string();

        let refresh_token = data
            .get("refresh_token")
            .and_then(Value::as_str)
            .map_or_else(String::new, ToString::to_string);

        let expires_at = data.get("expires_at").and_then(Value::as_i64).unwrap_or(0);

        Ok(Some(Self::new(
            access_token,
            refresh_token,
            expires_at,
            default_model,
            Some(path.to_path_buf()),
        )?))
    }

    pub fn from_openclaw(default_model: Option<String>) -> Result<Option<Self>> {
        let store_path = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("No home directory"))?
            .join(".openclaw")
            .join("agents")
            .join("main")
            .join("agent")
            .join("auth-profiles.json");

        if !store_path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&store_path)
            .context("Failed to read OpenClaw auth profiles")?;

        let data: Value =
            serde_json::from_str(&content).context("Failed to parse OpenClaw auth profiles")?;

        let profiles = data
            .get("profiles")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow::anyhow!("Invalid profiles structure"))?;

        // Try lastGood first, then any anthropic profile
        let mut candidates = Vec::new();
        if let Some(last_good) = data.get("lastGood").and_then(Value::as_object)
            && let Some(anthropic_id) = last_good.get("anthropic").and_then(Value::as_str)
        {
            candidates.push(anthropic_id.to_string());
        }

        for (pid, _) in profiles {
            if pid.starts_with("anthropic:") {
                candidates.push(pid.clone());
            }
        }

        for pid in candidates {
            if let Some(cred) = profiles.get(&pid).and_then(Value::as_object) {
                if cred.get("provider").and_then(Value::as_str) != Some("anthropic") {
                    continue;
                }

                if let Some(cred_type) = cred.get("type").and_then(Value::as_str) {
                    if cred_type == "oauth" {
                        if let Some(access) = cred.get("access").and_then(Value::as_str) {
                            let refresh = cred.get("refresh").and_then(Value::as_str).unwrap_or("");
                            let expires = cred.get("expires").and_then(Value::as_i64).unwrap_or(0);

                            return Ok(Some(Self::new(
                                access.to_string(),
                                refresh.to_string(),
                                expires,
                                default_model,
                                Some(store_path),
                            )?));
                        }
                    } else if cred_type == "token"
                        && let Some(token) = cred.get("token").and_then(Value::as_str)
                    {
                        let expires = cred.get("expires").and_then(Value::as_i64).unwrap_or(0);

                        return Ok(Some(Self::new(
                            token.to_string(),
                            String::new(),
                            expires,
                            default_model,
                            Some(store_path),
                        )?));
                    }
                }
            }
        }

        Ok(None)
    }

    pub fn from_claude_cli(default_model: Option<String>) -> Result<Option<Self>> {
        let cred_path = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("No home directory"))?
            .join(".claude")
            .join(".credentials.json");

        if !cred_path.exists() {
            return Ok(None);
        }

        let content =
            std::fs::read_to_string(&cred_path).context("Failed to read Claude CLI credentials")?;

        let data: Value =
            serde_json::from_str(&content).context("Failed to parse Claude CLI credentials")?;

        let oauth = data
            .get("claudeAiOauth")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow::anyhow!("Missing claudeAiOauth"))?;

        if let Some(access) = oauth.get("accessToken").and_then(Value::as_str) {
            let refresh = oauth
                .get("refreshToken")
                .and_then(Value::as_str)
                .unwrap_or("");
            let expires = oauth.get("expiresAt").and_then(Value::as_i64).unwrap_or(0);

            return Ok(Some(Self::new(
                access.to_string(),
                refresh.to_string(),
                expires,
                default_model,
                Some(cred_path),
            )?));
        }

        Ok(None)
    }
}

#[async_trait]
impl LLMProvider for AnthropicOAuthProvider {
    async fn chat(&self, req: ChatRequest<'_>) -> Result<LLMResponse> {
        let model = req.model.map_or(self.default_model.as_str(), |m| {
            // Strip provider prefix (e.g. "anthropic/claude-opus-4-6" -> "claude-opus-4-6")
            if m.contains('/') {
                m.split_once('/').map_or(m, |x| x.1)
            } else {
                m
            }
        });

        let token = self.ensure_valid_token().await?;

        let (system, anthropic_messages) = anthropic_common::convert_messages(req.messages);

        let mut payload = json!({
            "model": model,
            "messages": anthropic_messages,
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
        });

        if let Some(system) = system {
            payload["system"] = anthropic_common::system_to_content_blocks(&system);
        }

        if let Some(tools) = req.tools {
            payload["tools"] = serde_json::Value::Array(anthropic_common::convert_tools(tools));
            let choice = req.tool_choice.as_deref().unwrap_or("auto");
            payload["tool_choice"] = json!({"type": choice});
        }

        // Try the request, and on 401 refresh the token and retry once.
        // This handles clock skew and stale expires_at timestamps that
        // cause ensure_valid_token() to skip the proactive refresh.
        let resp = self.send_chat_request(&token, &payload).await?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            let refresh_token = self.refresh_token.lock().await.clone();
            if !refresh_token.is_empty() {
                info!("got 401, attempting OAuth token refresh and retry");
                match self.refresh_token_internal(&refresh_token).await {
                    Ok(()) => {
                        let new_token = self.access_token.lock().await.clone();
                        let retry_resp = self.send_chat_request(&new_token, &payload).await?;
                        let retry_resp =
                            ProviderErrorHandler::check_http_status(retry_resp, "AnthropicOAuth")
                                .await?;
                        let json: Value = retry_resp
                            .json()
                            .await
                            .context("failed to parse response")?;
                        return Ok(anthropic_common::parse_response(&json));
                    }
                    Err(e) => {
                        warn!("token refresh failed after 401: {}", e);
                    }
                }
            }
            // No refresh token or refresh failed — surface as auth error
            anyhow::bail!(
                "OAuth token expired and refresh failed. \
                 Re-authenticate with: oxicrab auth login"
            );
        }

        let resp = ProviderErrorHandler::check_http_status(resp, "AnthropicOAuth").await?;
        let json: Value = resp.json().await.context("failed to parse response")?;
        Ok(anthropic_common::parse_response(&json))
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    async fn warmup(&self) -> Result<()> {
        let start = std::time::Instant::now();
        let token = self.ensure_valid_token().await?;
        let payload = serde_json::json!({
            "model": self.default_model,
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 1,
        });
        let result = self
            .client
            .post(API_URL)
            .header("Authorization", format!("Bearer {}", token))
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .timeout(std::time::Duration::from_secs(15))
            .json(&payload)
            .send()
            .await;
        match result {
            Ok(resp) if !resp.status().is_success() => {
                warn!(
                    "anthropic oauth warmup got HTTP {} (non-fatal)",
                    resp.status()
                );
            }
            Ok(_) => info!(
                "anthropic oauth provider warmed up in {}ms",
                start.elapsed().as_millis()
            ),
            Err(e) => warn!("anthropic oauth warmup request failed (non-fatal): {}", e),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
