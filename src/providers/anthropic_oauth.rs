use crate::providers::anthropic_common;
use crate::providers::base::{ChatRequest, LLMProvider, LLMResponse};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

const TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";
const API_URL: &str = "https://api.anthropic.com/v1/messages";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";

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
    refresh_token: String,
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
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .context("Failed to create HTTP client for AnthropicOAuthProvider")?;

        let provider = Self {
            access_token: Arc::new(Mutex::new(access_token)),
            refresh_token,
            expires_at: Arc::new(Mutex::new(expires_at)),
            default_model: default_model.unwrap_or_else(|| "anthropic/claude-opus-4-6".to_string()),
            credentials_path,
            client,
        };

        // Load cached tokens if available
        if let Some(ref path) = provider.credentials_path {
            provider.load_cached_tokens(path);
        }

        Ok(provider)
    }

    fn load_cached_tokens(&self, path: &Path) {
        if !path.exists() {
            return;
        }

        match std::fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str::<Value>(&content) {
                Ok(data) => {
                    if let Some(cached_at) = data.get("expires_at").and_then(|v| v.as_i64()) {
                        let current_expires = *self.expires_at.blocking_lock();
                        if cached_at > current_expires {
                            if let (Some(access), Some(_refresh)) = (
                                data.get("access_token").and_then(|v| v.as_str()),
                                data.get("refresh_token").and_then(|v| v.as_str()),
                            ) {
                                *self.access_token.blocking_lock() = access.to_string();
                                *self.expires_at.blocking_lock() = cached_at;
                                info!("Loaded refreshed OAuth tokens from cache");
                            }
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
        let refresh_token = self.refresh_token.clone();
        let expires_at = *self.expires_at.lock().await;

        if !refresh_token.is_empty() && expires_at > 0 {
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .context("System time is before UNIX epoch")
                .map(|d| d.as_millis() as i64)?;

            if now_ms > expires_at {
                info!("OAuth token expired, refreshing...");
                match self.refresh_token_internal().await {
                    Ok(_) => {
                        info!("OAuth token refreshed successfully");
                    }
                    Err(e) => {
                        warn!("Token refresh failed: {}, using existing token", e);
                    }
                }
            }
        }

        Ok(self.access_token.lock().await.clone())
    }

    async fn refresh_token_internal(&self) -> Result<()> {
        let refresh_token = self.refresh_token.clone();

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        let payload = json!({
            "grant_type": "refresh_token",
            "client_id": CLIENT_ID,
            "refresh_token": refresh_token,
        });

        let resp = client
            .post(TOKEN_URL)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .context("Failed to refresh OAuth token")?;

        let data: Value = resp
            .json()
            .await
            .context("Failed to parse refresh response")?;

        let access_token = data["access_token"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing access_token in refresh response"))?
            .to_string();

        let _new_refresh_token = data
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.refresh_token.clone());

        // expires_in is in seconds, store as ms with 5min buffer
        let expires_in_secs = data["expires_in"].as_u64().unwrap_or(0);
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("System time is before UNIX epoch")
            .map(|d| d.as_millis() as i64)?;
        let expires_at = now_ms + (expires_in_secs * 1000) as i64 - 300_000;

        *self.access_token.lock().await = access_token;
        *self.expires_at.lock().await = expires_at;

        // Persist refreshed credentials if path is configured
        if let Some(ref path) = self.credentials_path {
            self.save_credentials(path).await;
        }

        Ok(())
    }

    async fn save_credentials(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                warn!("Failed to create credentials directory: {}", e);
                return;
            }
        }

        let data = json!({
            "access_token": *self.access_token.lock().await,
            "refresh_token": self.refresh_token,
            "expires_at": *self.expires_at.lock().await,
        });

        if let Err(e) = std::fs::write(
            path,
            serde_json::to_string_pretty(&data).unwrap_or_default(),
        ) {
            warn!("Failed to save OAuth credentials: {}", e);
        } else {
            // Restrict permissions to owner-only (0o600) to protect tokens
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Err(e) =
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                {
                    warn!("Failed to set credentials file permissions: {}", e);
                }
            }
            debug!("OAuth credentials saved to {}", path.display());
        }
    }

    pub fn from_credentials_file(
        path: &Path,
        default_model: Option<String>,
    ) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(path).context("Failed to read credentials file")?;

        let data: Value =
            serde_json::from_str(&content).context("Failed to parse credentials file")?;

        let access_token = data["access_token"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing access_token"))?
            .to_string();

        let refresh_token = data
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default();

        let expires_at = data.get("expires_at").and_then(|v| v.as_i64()).unwrap_or(0);

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
            .and_then(|v| v.as_object())
            .ok_or_else(|| anyhow::anyhow!("Invalid profiles structure"))?;

        // Try lastGood first, then any anthropic profile
        let mut candidates = Vec::new();
        if let Some(last_good) = data.get("lastGood").and_then(|v| v.as_object()) {
            if let Some(anthropic_id) = last_good.get("anthropic").and_then(|v| v.as_str()) {
                candidates.push(anthropic_id.to_string());
            }
        }

        for (pid, _) in profiles {
            if pid.starts_with("anthropic:") {
                candidates.push(pid.clone());
            }
        }

        for pid in candidates {
            if let Some(cred) = profiles.get(&pid).and_then(|v| v.as_object()) {
                if cred.get("provider").and_then(|v| v.as_str()) != Some("anthropic") {
                    continue;
                }

                if let Some(cred_type) = cred.get("type").and_then(|v| v.as_str()) {
                    if cred_type == "oauth" {
                        if let Some(access) = cred.get("access").and_then(|v| v.as_str()) {
                            let refresh =
                                cred.get("refresh").and_then(|v| v.as_str()).unwrap_or("");
                            let expires = cred.get("expires").and_then(|v| v.as_i64()).unwrap_or(0);

                            return Ok(Some(Self::new(
                                access.to_string(),
                                refresh.to_string(),
                                expires,
                                default_model,
                                Some(store_path),
                            )?));
                        }
                    } else if cred_type == "token" {
                        if let Some(token) = cred.get("token").and_then(|v| v.as_str()) {
                            let expires = cred.get("expires").and_then(|v| v.as_i64()).unwrap_or(0);

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
            .and_then(|v| v.as_object())
            .ok_or_else(|| anyhow::anyhow!("Missing claudeAiOauth"))?;

        if let Some(access) = oauth.get("accessToken").and_then(|v| v.as_str()) {
            let refresh = oauth
                .get("refreshToken")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let expires = oauth.get("expiresAt").and_then(|v| v.as_i64()).unwrap_or(0);

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
        let model = req
            .model
            .map(|m| {
                // Strip provider prefix (e.g. "anthropic/claude-opus-4-6" -> "claude-opus-4-6")
                if m.contains('/') {
                    m.split_once('/').map(|x| x.1).unwrap_or(m)
                } else {
                    m
                }
            })
            .unwrap_or(&self.default_model);

        let token = self.ensure_valid_token().await?;

        let (system, anthropic_messages) = anthropic_common::convert_messages(req.messages);

        let mut payload = json!({
            "model": model,
            "messages": anthropic_messages,
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
        });

        if let Some(system) = system {
            payload["system"] = json!(system);
        }

        if let Some(tools) = req.tools {
            payload["tools"] = json!(anthropic_common::convert_tools(tools));
            payload["tool_choice"] = json!({"type": "auto"});
        }

        let mut request = self
            .client
            .post(API_URL)
            .header("Authorization", format!("Bearer {}", token));

        for (key, value) in claude_code_headers() {
            request = request.header(key, value);
        }

        let resp = request
            .json(&payload)
            .send()
            .await
            .context("Failed to send request to Anthropic OAuth API")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!("Anthropic OAuth API error {}: {}", status, body);
            return Err(anyhow::anyhow!(
                "Anthropic OAuth API error {}: {}",
                status,
                body
            ));
        }

        let json: Value = resp.json().await.context("Failed to parse response")?;
        Ok(anthropic_common::parse_response(&json))
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }
}
