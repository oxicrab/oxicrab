use crate::regex_utils::compile_slack_mention;
use crate::utils::{
    DmCheckResult, MAX_AUDIO_DOWNLOAD, MAX_IMAGE_DOWNLOAD, check_dm_access, check_group_access,
    exponential_backoff_delay, format_pairing_reply,
};
use anyhow::Result;
use async_trait::async_trait;
use futures_util::SinkExt;
use oxicrab_core::bus::events::{InboundMessage, OutboundMessage};
use oxicrab_core::channels::base::{BaseChannel, split_message};
use oxicrab_core::config::schema::SlackConfig;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

mod formatting;

const MAX_USER_CACHE: usize = 1000;

/// Subtypes to ignore when processing Slack message events.
/// Unknown subtypes are allowed through (safe default = process).
const IGNORED_SUBTYPES: &[&str] = &[
    "bot_message",
    "message_changed",
    "message_deleted",
    "channel_join",
    "channel_leave",
    "channel_topic",
    "channel_purpose",
    "channel_name",
    "channel_archive",
    "channel_unarchive",
    "group_join",
    "group_leave",
    "ekm_access_denied",
    "me_message",
];

/// Classified Slack API errors for structured handling.
#[derive(Debug)]
enum SlackApiError {
    RateLimited { retry_after_secs: u32 },
    InvalidAuth,
    MissingScope(String),
    ChannelNotFound,
    ServerError(u16),
    Other(String),
}

/// Classify a Slack API error from HTTP status and error field.
///
/// Note: `retry_after_secs` defaults to 1 here because this layer doesn't
/// have access to HTTP response headers. The retry wrappers
/// (`send_slack_api_with_retry` / `send_slack_api_json_with_retry`) parse
/// the actual `Retry-After` header value from the error string metadata
/// and override `retry_after_secs` before using it for the delay.
fn classify_slack_error(http_status: u16, error_field: Option<&str>) -> SlackApiError {
    if http_status == 429 {
        return SlackApiError::RateLimited {
            retry_after_secs: 1,
        };
    }
    if http_status >= 500 {
        return SlackApiError::ServerError(http_status);
    }
    match error_field {
        Some("invalid_auth" | "account_inactive" | "token_revoked") => SlackApiError::InvalidAuth,
        Some(e) if e.starts_with("missing_scope") => SlackApiError::MissingScope(e.to_string()),
        Some("channel_not_found") => SlackApiError::ChannelNotFound,
        Some("ratelimited") => SlackApiError::RateLimited {
            retry_after_secs: 1,
        },
        Some(e) => SlackApiError::Other(e.to_string()),
        None if http_status >= 400 => SlackApiError::Other(format!("HTTP {http_status}")),
        None => SlackApiError::Other("unknown".to_string()),
    }
}

fn is_retryable(err: &SlackApiError) -> bool {
    matches!(
        err,
        SlackApiError::ServerError(status) if *status >= 500
    ) || matches!(err, SlackApiError::RateLimited { .. })
}

/// Parse HTTP status and retry-after from structured error messages
/// produced by `parse_slack_response()` (format: "... [status=429] [retry-after=30]").
fn parse_error_metadata(err_str: &str) -> (u16, Option<u32>) {
    let status = err_str
        .find("[status=")
        .and_then(|pos| {
            let start = pos + "[status=".len();
            err_str[start..]
                .find(']')
                .and_then(|end| err_str[start..start + end].parse().ok())
        })
        .unwrap_or(0);

    let retry_after = err_str.find("[retry-after=").and_then(|pos| {
        let start = pos + "[retry-after=".len();
        err_str[start..]
            .find(']')
            .and_then(|end| err_str[start..start + end].parse().ok())
    });

    (status, retry_after)
}

pub struct SlackChannel {
    config: SlackConfig,
    inbound_tx: Arc<mpsc::Sender<InboundMessage>>,
    bot_user_id: Arc<tokio::sync::Mutex<Option<String>>>,
    mention_regex: Arc<tokio::sync::Mutex<Option<regex::Regex>>>,
    running: Arc<tokio::sync::Mutex<bool>>,
    ws_handle: Option<tokio::task::JoinHandle<()>>,
    seen_messages: Arc<tokio::sync::Mutex<indexmap::IndexSet<String>>>,
    user_cache: Arc<tokio::sync::Mutex<lru::LruCache<String, String>>>,
    client: reqwest::Client,
}

impl SlackChannel {
    pub fn new(config: SlackConfig, inbound_tx: Arc<mpsc::Sender<InboundMessage>>) -> Self {
        Self {
            config,
            inbound_tx,
            bot_user_id: Arc::new(tokio::sync::Mutex::new(None)),
            mention_regex: Arc::new(tokio::sync::Mutex::new(None)),
            running: Arc::new(tokio::sync::Mutex::new(false)),
            ws_handle: None,
            seen_messages: Arc::new(tokio::sync::Mutex::new(indexmap::IndexSet::new())),
            user_cache: Arc::new(tokio::sync::Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(MAX_USER_CACHE).unwrap(),
            ))),
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    fn format_for_slack(text: &str) -> String {
        formatting::format_for_slack(text)
    }

    #[cfg(test)]
    fn convert_tables(text: &str) -> String {
        formatting::convert_tables(text)
    }

    /// Upload a file to a Slack channel using the 3-step upload API.
    ///
    /// 1. `files.getUploadURLExternal` — get a pre-signed upload URL
    /// 2. PUT raw bytes to the upload URL
    /// 3. `files.completeUploadExternal` — finalize and share to channel
    ///
    /// Requires the `files:write` OAuth scope.
    async fn upload_file(&self, channel_id: &str, file_path: &str) -> Result<()> {
        let path = std::path::Path::new(file_path);
        if !path.exists() {
            warn!("slack: media file not found: {}", file_path);
            return Ok(());
        }

        let path_owned = path.to_path_buf();
        let file_bytes = tokio::task::spawn_blocking(move || std::fs::read(&path_owned)).await??;
        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");

        // Step 1: Get upload URL
        let step1_resp = self
            .client
            .post("https://slack.com/api/files.getUploadURLExternal")
            .header("Authorization", format!("Bearer {}", self.config.bot_token))
            .form(&[
                ("filename", filename),
                ("length", &file_bytes.len().to_string()),
            ])
            .send()
            .await?;

        let step1_json: Value = step1_resp.json().await?;
        if step1_json.get("ok").and_then(Value::as_bool) != Some(true) {
            let error = step1_json
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            warn!("slack: files.getUploadURLExternal failed: {}", error);
            return Err(anyhow::anyhow!("Slack file upload step 1 failed: {error}"));
        }

        let upload_url = step1_json
            .get("upload_url")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing upload_url in response"))?;
        let file_id = step1_json
            .get("file_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing file_id in response"))?;

        // Step 2: Upload file bytes via POST multipart form-data
        let form = reqwest::multipart::Form::new().part(
            "file",
            reqwest::multipart::Part::bytes(file_bytes)
                .file_name(filename.to_string())
                .mime_str("application/octet-stream")
                .expect("hardcoded MIME type is valid"),
        );
        let step2_resp = self.client.post(upload_url).multipart(form).send().await?;

        if !step2_resp.status().is_success() {
            let status = step2_resp.status();
            let body = step2_resp.text().await.unwrap_or_default();
            warn!(
                "slack: file upload POST failed: status={}, body={}",
                status, body
            );
            return Err(anyhow::anyhow!(
                "Slack file upload POST failed: {status} — {body}"
            ));
        }

        // Step 3: Complete upload and share to channel
        let files_payload = serde_json::json!([{"id": file_id}]);
        let step3_resp = self
            .client
            .post("https://slack.com/api/files.completeUploadExternal")
            .header("Authorization", format!("Bearer {}", self.config.bot_token))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "files": files_payload,
                "channel_id": channel_id,
            }))
            .send()
            .await?;

        let step3_json: Value = step3_resp.json().await?;
        if step3_json.get("ok").and_then(Value::as_bool) != Some(true) {
            let error = step3_json
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            warn!("slack: files.completeUploadExternal failed: {}", error);
            return Err(anyhow::anyhow!("Slack file upload step 3 failed: {error}"));
        }

        info!("slack: uploaded file '{}' to {}", filename, channel_id);
        Ok(())
    }

    async fn send_slack_api(&self, method: &str, params: &HashMap<&str, Value>) -> Result<Value> {
        let url = format!("https://slack.com/api/{method}");

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.bot_token))
            .form(params)
            .send()
            .await?;

        Self::parse_slack_response(response).await
    }

    /// Send a JSON-body Slack API request (needed for Block Kit `blocks` payloads).
    async fn send_slack_api_json(&self, method: &str, body: &Value) -> Result<Value> {
        let url = format!("https://slack.com/api/{method}");

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.bot_token))
            .header("Content-Type", "application/json; charset=utf-8")
            .json(body)
            .send()
            .await?;

        Self::parse_slack_response(response).await
    }

    /// Parse a Slack API response, extracting HTTP status and Retry-After for
    /// error classification upstream.
    async fn parse_slack_response(response: reqwest::Response) -> Result<Value> {
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u32>().ok());

        let json: Value = response.json().await?;
        if json.get("ok").and_then(Value::as_bool) != Some(true) {
            let error = json
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            // Include HTTP status and Retry-After in the error for the retry
            // wrapper to parse. Format: "Slack API error: {error} [status={status}]"
            // with optional "[retry-after={secs}]" suffix.
            let mut msg = format!("Slack API error: {error} [status={status}]");
            if let Some(secs) = retry_after {
                use std::fmt::Write;
                let _ = write!(msg, " [retry-after={secs}]");
            }
            return Err(anyhow::anyhow!("{msg}"));
        }
        Ok(json)
    }

    /// Retry wrapper for Slack API calls. Retries up to 3 times for transient
    /// (5xx) errors and rate limits (429) with Retry-After backoff.
    async fn send_slack_api_with_retry(
        &self,
        method: &str,
        params: &HashMap<&str, Value>,
    ) -> Result<Value> {
        let mut last_err = None;
        for attempt in 0..3u32 {
            match self.send_slack_api(method, params).await {
                Ok(json) => return Ok(json),
                Err(e) => {
                    let err_str = e.to_string();
                    let (http_status, retry_after) = parse_error_metadata(&err_str);
                    let error_field = err_str
                        .strip_prefix("Slack API error: ")
                        .map(|s| s.split(" [status=").next().unwrap_or(s));
                    let mut classified = classify_slack_error(http_status, error_field);
                    // Override retry_after_secs with the parsed Retry-After header value
                    if let (SlackApiError::RateLimited { retry_after_secs }, Some(parsed)) =
                        (&mut classified, retry_after)
                    {
                        *retry_after_secs = parsed;
                    }
                    if !is_retryable(&classified) {
                        match &classified {
                            SlackApiError::InvalidAuth => {
                                error!("slack: invalid auth for {method}");
                            }
                            SlackApiError::MissingScope(scope) => {
                                error!("slack: missing scope for {method}: {scope}");
                            }
                            SlackApiError::ChannelNotFound => {
                                warn!("slack: channel not found for {method}");
                            }
                            SlackApiError::Other(msg) => {
                                warn!("slack: API error on {method}: {msg}");
                            }
                            SlackApiError::RateLimited { .. } | SlackApiError::ServerError(_) => {}
                        }
                        return Err(e);
                    }
                    let delay = match &classified {
                        SlackApiError::RateLimited { retry_after_secs } => {
                            u64::from(*retry_after_secs)
                        }
                        _ => 1u64 << attempt,
                    };
                    warn!(
                        "slack: retryable error on {method} (attempt {}): {err_str}, retrying in {delay}s",
                        attempt + 1
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("Slack API retry exhausted")))
    }

    /// Retry wrapper for JSON-body Slack API calls.
    async fn send_slack_api_json_with_retry(&self, method: &str, body: &Value) -> Result<Value> {
        let mut last_err = None;
        for attempt in 0..3u32 {
            match self.send_slack_api_json(method, body).await {
                Ok(json) => return Ok(json),
                Err(e) => {
                    let err_str = e.to_string();
                    let (http_status, retry_after) = parse_error_metadata(&err_str);
                    let error_field = err_str
                        .strip_prefix("Slack API error: ")
                        .map(|s| s.split(" [status=").next().unwrap_or(s));
                    let mut classified = classify_slack_error(http_status, error_field);
                    if let (SlackApiError::RateLimited { retry_after_secs }, Some(parsed)) =
                        (&mut classified, retry_after)
                    {
                        *retry_after_secs = parsed;
                    }
                    if !is_retryable(&classified) {
                        return Err(e);
                    }
                    let delay = match &classified {
                        SlackApiError::RateLimited { retry_after_secs } => {
                            u64::from(*retry_after_secs)
                        }
                        _ => 1u64 << attempt,
                    };
                    warn!(
                        "slack: retryable error on {method} (attempt {}): {err_str}, retrying in {delay}s",
                        attempt + 1
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("Slack API retry exhausted")))
    }
}

#[async_trait]
impl BaseChannel for SlackChannel {
    fn name(&self) -> &'static str {
        "slack"
    }

    #[allow(clippy::too_many_lines)]
    async fn start(&mut self) -> Result<()> {
        info!("Initializing Slack channel...");
        if self.config.bot_token.is_empty() {
            error!("Slack bot_token not configured");
            return Err(anyhow::anyhow!("Slack bot_token not configured"));
        }
        if self.config.app_token.is_empty() {
            error!("Slack app_token not configured (needed for Socket Mode)");
            return Err(anyhow::anyhow!("Slack app_token not configured"));
        }

        *self.running.lock().await = true;

        info!("Starting Slack bot (Socket Mode)...");

        // Connect to Socket Mode via WebSocket
        // Share channel state with the WS task via Arc
        let app_token = self.config.app_token.clone();
        let bot_token = self.config.bot_token.clone();
        let config_allow = self.config.allow_from.clone();
        let config_allow_groups = self.config.allow_groups.clone();
        let dm_policy = self.config.dm_policy.clone();
        let inbound_tx = self.inbound_tx.clone();
        let bot_user_id = self.bot_user_id.clone();
        let mention_regex = self.mention_regex.clone();
        let seen_messages = self.seen_messages.clone();
        let user_cache = self.user_cache.clone();
        let ws_client = self.client.clone();
        let running = self.running.clone();
        let thinking_emoji = self.config.thinking_emoji.clone();

        let ws_task = tokio::spawn(async move {
            use futures_util::StreamExt;
            use tokio_tungstenite::tungstenite::Message;

            // Slack Socket Mode connection
            // First, call apps.connections.open to get the WebSocket URL
            // Then connect to that URL

            let mut reconnect_attempt = 0u32;
            let mut auth_completed = false;
            loop {
                // Check running flag for clean shutdown
                if !*running.lock().await {
                    info!("Slack WebSocket shutting down (running=false)");
                    break;
                }

                // Fetch bot's own user ID (retried each reconnect until successful)
                if !auth_completed {
                    let auth_url = "https://slack.com/api/auth.test";
                    match ws_client
                        .post(auth_url)
                        .header("Authorization", format!("Bearer {bot_token}"))
                        .send()
                        .await
                    {
                        Ok(resp) => match resp.json::<Value>().await {
                            Ok(auth) if auth.get("ok").and_then(Value::as_bool) == Some(true) => {
                                let new_bot_id = auth
                                    .get("user_id")
                                    .and_then(Value::as_str)
                                    .map(ToString::to_string);
                                if let Some(ref id) = new_bot_id
                                    && let Ok(re) = compile_slack_mention(id)
                                {
                                    *mention_regex.lock().await = Some(re);
                                }
                                *bot_user_id.lock().await = new_bot_id;
                                let user = auth
                                    .get("user")
                                    .and_then(Value::as_str)
                                    .unwrap_or("unknown");
                                let uid = auth
                                    .get("user_id")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default();
                                info!("Slack bot authenticated as {} (ID: {})", user, uid);
                                auth_completed = true;

                                // Set presence to active (optional)
                                let presence_resp = ws_client
                                    .post("https://slack.com/api/users.setPresence")
                                    .header("Authorization", format!("Bearer {bot_token}"))
                                    .form(&[("presence", "auto")])
                                    .send()
                                    .await;
                                if let Err(e) = presence_resp {
                                    let error_msg = e.to_string();
                                    if error_msg.contains("missing_scope") {
                                        debug!(
                                            "Slack presence setting skipped (missing users:write scope)"
                                        );
                                    } else {
                                        warn!("failed to set Slack presence: {}", e);
                                    }
                                }
                            }
                            Ok(auth) => {
                                let err = auth
                                    .get("error")
                                    .and_then(Value::as_str)
                                    .unwrap_or("unknown");
                                error!("Slack auth.test failed: {}", err);
                                let delay = exponential_backoff_delay(reconnect_attempt, 5, 60);
                                reconnect_attempt += 1;
                                warn!("retrying Slack auth in {} seconds...", delay);
                                tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                                continue;
                            }
                            Err(e) => {
                                error!("Slack auth.test response parse error: {}", e);
                                let delay = exponential_backoff_delay(reconnect_attempt, 5, 60);
                                reconnect_attempt += 1;
                                warn!("retrying Slack auth in {} seconds...", delay);
                                tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                                continue;
                            }
                        },
                        Err(e) => {
                            error!("Slack auth.test network error: {}", e);
                            let delay = exponential_backoff_delay(reconnect_attempt, 5, 60);
                            reconnect_attempt += 1;
                            warn!("retrying Slack connection in {} seconds...", delay);
                            tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                            continue;
                        }
                    }
                }
                debug!("Attempting to connect to Slack Socket Mode...");
                debug!(
                    "Slack app token configured (length: {} chars)",
                    app_token.len()
                );

                // Get WebSocket URL from Slack API
                let response = match ws_client
                    .post("https://slack.com/api/apps.connections.open")
                    .header("Authorization", format!("Bearer {app_token}"))
                    .send()
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        error!("Failed to call apps.connections.open: {}", e);
                        let delay = exponential_backoff_delay(reconnect_attempt, 5, 60);
                        reconnect_attempt += 1;
                        warn!(
                            "Retrying Slack Socket Mode connection in {} seconds...",
                            delay
                        );
                        tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                        continue;
                    }
                };

                let json: Value = match response.json().await {
                    Ok(j) => j,
                    Err(e) => {
                        error!("Failed to parse apps.connections.open response: {}", e);
                        let delay = exponential_backoff_delay(reconnect_attempt, 5, 60);
                        reconnect_attempt += 1;
                        warn!(
                            "Retrying Slack Socket Mode connection in {} seconds...",
                            delay
                        );
                        tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                        continue;
                    }
                };

                if json.get("ok").and_then(Value::as_bool) != Some(true) {
                    let error = json
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    error!("Slack apps.connections.open error: {}", error);
                    if error == "invalid_auth" {
                        warn!(
                            "Invalid app_token - check that it starts with 'xapp-' and has 'connections:write' scope"
                        );
                    }
                    let delay = exponential_backoff_delay(reconnect_attempt, 5, 60);
                    reconnect_attempt += 1;
                    warn!(
                        "Retrying Slack Socket Mode connection in {} seconds...",
                        delay
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                    continue;
                }

                let Some(ws_url) = json.get("url").and_then(Value::as_str) else {
                    error!("No 'url' field in apps.connections.open response");
                    let delay = exponential_backoff_delay(reconnect_attempt, 5, 60);
                    reconnect_attempt += 1;
                    warn!(
                        "Retrying Slack Socket Mode connection in {} seconds...",
                        delay
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                    continue;
                };

                debug!(
                    "Received WebSocket URL from Slack (length: {} chars)",
                    ws_url.len()
                );

                let url = match url::Url::parse(ws_url) {
                    Ok(u) => u,
                    Err(e) => {
                        error!("Failed to parse WebSocket URL: {}", e);
                        let delay = exponential_backoff_delay(reconnect_attempt, 5, 60);
                        reconnect_attempt += 1;
                        warn!(
                            "Retrying Slack Socket Mode connection in {} seconds...",
                            delay
                        );
                        tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                        continue;
                    }
                };

                match tokio_tungstenite::connect_async(url.as_str()).await {
                    Ok((ws_stream, response)) => {
                        info!(
                            "Connected to Slack Socket Mode (status: {})",
                            response.status()
                        );
                        reconnect_attempt = 0;
                        let conn_start = std::time::Instant::now();
                        let (mut write, mut read) = ws_stream.split();

                        while let Some(msg) = read.next().await {
                            match msg {
                                Ok(Message::Text(text)) => {
                                    if let Ok(event) = serde_json::from_str::<Value>(&text) {
                                        let event_type = event
                                            .get("type")
                                            .and_then(Value::as_str)
                                            .unwrap_or_default();

                                        // Handle hello message
                                        if event_type == "hello" {
                                            info!("Received Socket Mode hello message");
                                            continue;
                                        }

                                        // Handle disconnect message — Slack asks
                                        // us to reconnect (server rotation, etc.)
                                        if event_type == "disconnect" {
                                            let reason = event
                                                .get("reason")
                                                .and_then(Value::as_str)
                                                .unwrap_or("unknown");
                                            info!(
                                                "Slack Socket Mode disconnect requested: {}",
                                                reason
                                            );
                                            break;
                                        }

                                        // Acknowledge events_api and interactive messages via WebSocket
                                        // Slack Socket Mode requires acknowledgments to be sent back through the WebSocket
                                        if (event_type == "events_api"
                                            || event_type == "interactive")
                                            && let Some(envelope_id) = event.get("envelope_id")
                                        {
                                            let envelope_id_str =
                                                envelope_id.as_str().unwrap_or_default();
                                            let ack_msg = serde_json::json!({
                                                "envelope_id": envelope_id_str,
                                                "payload": {}
                                            });
                                            debug!(
                                                "Sending Socket Mode acknowledgment for envelope_id: {}",
                                                envelope_id_str
                                            );
                                            if let Err(e) =
                                                write.send(Message::text(ack_msg.to_string())).await
                                            {
                                                error!(
                                                    "Failed to send Socket Mode acknowledgment: {}",
                                                    e
                                                );
                                            }
                                        }

                                        // Handle interactive payloads (button clicks)
                                        if event_type == "interactive"
                                            && let Some(payload) = event.get("payload")
                                            && let Err(e) = handle_interactive_payload(
                                                payload,
                                                &inbound_tx,
                                                &config_allow,
                                                &config_allow_groups,
                                                &dm_policy,
                                                &bot_token,
                                                &ws_client,
                                                &thinking_emoji,
                                            )
                                            .await
                                        {
                                            error!(
                                                "Error handling Slack interactive payload: {}",
                                                e
                                            );
                                        }

                                        // Process the event
                                        if event_type == "events_api"
                                            && let Some(payload) = event.get("payload")
                                            && let Some(event_data) = payload.get("event")
                                        {
                                            let inner_event_type = event_data
                                                .get("type")
                                                .and_then(Value::as_str)
                                                .unwrap_or_default();

                                            match inner_event_type {
                                                "message" | "app_mention" => {
                                                    if let Err(e) = handle_slack_event(
                                                        event_data,
                                                        &bot_user_id,
                                                        &mention_regex,
                                                        &seen_messages,
                                                        &user_cache,
                                                        &inbound_tx,
                                                        &config_allow,
                                                        &config_allow_groups,
                                                        &dm_policy,
                                                        &bot_token,
                                                        &ws_client,
                                                        &thinking_emoji,
                                                    )
                                                    .await
                                                    {
                                                        error!(
                                                            "Error handling Slack message event: {}",
                                                            e
                                                        );
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                }
                                Ok(Message::Close(_)) => {
                                    info!("Slack Socket Mode connection closed");
                                    break;
                                }
                                Ok(Message::Ping(data)) => {
                                    if let Err(e) = write.send(Message::Pong(data)).await {
                                        error!("Failed to send Slack WebSocket pong: {}", e);
                                    }
                                }
                                Err(e) => {
                                    error!("WebSocket error: {}", e);
                                    break;
                                }
                                _ => {}
                            }
                        }

                        // Connection dropped — decay backoff if it was stable
                        let elapsed = conn_start.elapsed().as_secs();
                        if elapsed > 300 {
                            reconnect_attempt = 0;
                        } else if elapsed > 60 && reconnect_attempt > 0 {
                            reconnect_attempt /= 2;
                        }

                        // Check if we should still reconnect
                        if !*running.lock().await {
                            break;
                        }
                        let delay = exponential_backoff_delay(reconnect_attempt, 5, 60);
                        reconnect_attempt += 1;
                        warn!(
                            "Slack Socket Mode connection lost, reconnecting in {} seconds...",
                            delay
                        );
                        tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                    }
                    Err(e) => {
                        let error_str = e.to_string();
                        error!("Slack Socket Mode connection error: {}", error_str);
                        if error_str.contains("400") {
                            warn!("400 Bad Request - The token format might be incorrect.");
                            warn!(
                                "Make sure your app_token starts with 'xapp-' and is a Socket Mode token."
                            );
                            warn!(
                                "You can generate a new token at: https://api.slack.com/apps/<your-app-id>/socket-mode"
                            );
                        } else if error_str.contains("403") {
                            warn!("403 Forbidden - Check that:");
                            warn!("  1. Socket Mode is enabled in your Slack app settings");
                            warn!("  2. app_token is a Socket Mode token (starts with xapp-)");
                            warn!("  3. The token has not expired");
                        }
                        let delay = exponential_backoff_delay(reconnect_attempt, 5, 60);
                        reconnect_attempt += 1;
                        warn!(
                            "Retrying Slack Socket Mode connection in {} seconds...",
                            delay
                        );
                        tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                    }
                }
            }
        });

        self.ws_handle = Some(ws_task);
        info!("Slack channel started successfully (Socket Mode connecting in background)");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        *self.running.lock().await = false;
        if let Some(handle) = self.ws_handle.take() {
            handle.abort();
        }
        info!("Stopping Slack bot...");
        Ok(())
    }

    async fn is_healthy(&self) -> bool {
        if let Some(ref handle) = self.ws_handle {
            !handle.is_finished()
        } else {
            false
        }
    }

    async fn send_typing(&self, chat_id: &str) -> Result<()> {
        // Slack has no typing indicator API for bots (only legacy RTM supported it).
        // Visible progress is provided via editable status messages instead.
        debug!("Slack typing indicator requested for {} (no-op)", chat_id);
        Ok(())
    }

    async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        if msg.channel != "slack" {
            return Ok(());
        }

        // Upload media attachments first
        for path in &msg.media {
            if let Err(e) = self.upload_file(&msg.chat_id, path).await {
                warn!("slack: failed to upload file {}: {}", path, e);
            }
        }

        let content = Self::format_for_slack(&msg.content);
        let buttons = convert_buttons_to_blocks(&msg.metadata);

        // Split long messages (Slack limit is ~40k but 4000 is more readable)
        // Thread replies: use reply_to or inbound ts metadata for threading
        let thread_ts = msg.reply_to.as_deref().or_else(|| {
            msg.metadata
                .get(oxicrab_core::bus::events::meta::TS)
                .and_then(|v| v.as_str())
        });
        let chunks = split_message(&content, 4000);
        let chunk_count = chunks.len();
        for (i, chunk) in chunks.iter().enumerate() {
            let is_last = i == chunk_count - 1;

            // Attach buttons (Block Kit) to the last chunk via JSON body
            if is_last && !buttons.is_empty() {
                let mut body = serde_json::json!({
                    "channel": msg.chat_id,
                    "text": chunk,
                    "mrkdwn": true,
                    "blocks": [
                        {"type": "section", "text": {"type": "mrkdwn", "text": chunk}},
                    ],
                });
                // Append button action rows to blocks
                let blocks = body["blocks"].as_array_mut().unwrap();
                blocks.extend(buttons.clone());
                if let Some(ts) = thread_ts {
                    body["thread_ts"] = Value::String(ts.to_string());
                }
                if let Err(e) = self
                    .send_slack_api_json_with_retry("chat.postMessage", &body)
                    .await
                {
                    error!("Error sending Slack message with blocks: {}", e);
                    return Err(anyhow::anyhow!("Slack send error: {e}"));
                }
            } else {
                let mut params = HashMap::new();
                params.insert("channel", Value::String(msg.chat_id.clone()));
                params.insert("text", Value::String(chunk.clone()));
                params.insert("mrkdwn", Value::Bool(true));
                if let Some(ts) = thread_ts {
                    params.insert("thread_ts", Value::String(ts.to_string()));
                }
                if let Err(e) = self
                    .send_slack_api_with_retry("chat.postMessage", &params)
                    .await
                {
                    error!("Error sending Slack message: {}", e);
                    return Err(anyhow::anyhow!("Slack send error: {e}"));
                }
            }
        }

        // If no text chunks but we have buttons, send buttons standalone
        if chunks.is_empty() && !buttons.is_empty() {
            let mut body = serde_json::json!({
                "channel": msg.chat_id,
                "text": " ",
                "blocks": buttons,
            });
            if let Some(ts) = thread_ts {
                body["thread_ts"] = Value::String(ts.to_string());
            }
            if let Err(e) = self
                .send_slack_api_json_with_retry("chat.postMessage", &body)
                .await
            {
                error!("Error sending Slack buttons-only message: {}", e);
                return Err(anyhow::anyhow!("Slack send error: {e}"));
            }
        }

        // Swap thinking → done reaction (fire-and-forget)
        if let Some(ts) = msg
            .metadata
            .get(oxicrab_core::bus::events::meta::TS)
            .and_then(|v| v.as_str())
        {
            let client = self.client.clone();
            let token = self.config.bot_token.clone();
            let channel = msg.chat_id.clone();
            let ts = ts.to_string();
            let thinking = self.config.thinking_emoji.clone();
            let done = self.config.done_emoji.clone();
            tokio::spawn(async move {
                // Remove thinking reaction
                let _ = client
                    .post("https://slack.com/api/reactions.remove")
                    .form(&[
                        ("token", token.as_str()),
                        ("channel", channel.as_str()),
                        ("timestamp", ts.as_str()),
                        ("name", thinking.as_str()),
                    ])
                    .send()
                    .await;
                // Add done reaction
                let _ = client
                    .post("https://slack.com/api/reactions.add")
                    .form(&[
                        ("token", token.as_str()),
                        ("channel", channel.as_str()),
                        ("timestamp", ts.as_str()),
                        ("name", done.as_str()),
                    ])
                    .send()
                    .await;
            });
        }

        Ok(())
    }

    async fn send_and_get_id(&self, msg: &OutboundMessage) -> Result<Option<String>> {
        if msg.channel != "slack" {
            return Ok(None);
        }
        let content = Self::format_for_slack(&msg.content);
        let mut params = HashMap::new();
        params.insert("channel", Value::String(msg.chat_id.clone()));
        params.insert("text", Value::String(content));
        params.insert("mrkdwn", Value::Bool(true));
        let response = self
            .send_slack_api_with_retry("chat.postMessage", &params)
            .await?;
        Ok(response
            .get("ts")
            .and_then(Value::as_str)
            .map(ToString::to_string))
    }

    async fn edit_message(&self, chat_id: &str, message_id: &str, content: &str) -> Result<()> {
        let content = Self::format_for_slack(content);
        let mut params = HashMap::new();
        params.insert("channel", Value::String(chat_id.to_string()));
        params.insert("ts", Value::String(message_id.to_string()));
        params.insert("text", Value::String(content));
        self.send_slack_api_with_retry("chat.update", &params)
            .await?;
        Ok(())
    }

    async fn delete_message(&self, chat_id: &str, message_id: &str) -> Result<()> {
        let mut params = HashMap::new();
        params.insert("channel", Value::String(chat_id.to_string()));
        params.insert("ts", Value::String(message_id.to_string()));
        self.send_slack_api("chat.delete", &params).await?;
        Ok(())
    }
}

/// Check if a URL belongs to a Slack-owned domain.
///
/// Uses proper URL parsing to prevent SSRF via domains like `attacker-slack.com`.
fn is_slack_domain(url_str: &str) -> bool {
    url::Url::parse(url_str)
        .ok()
        .and_then(|u| u.host_str().map(str::to_lowercase))
        .is_some_and(|host| {
            host == "slack.com"
                || host.ends_with(".slack.com")
                || host == "slack-edge.com"
                || host.ends_with(".slack-edge.com")
        })
}

/// Download a file from Slack, following redirects manually to preserve auth.
///
/// Slack's file download redirects through multiple hops:
///   files.slack.com → workspace.slack.com/?redir=/files-pri/... → CDN
/// Standard HTTP clients strip the Authorization header on cross-origin redirects,
/// so we follow the chain manually, re-adding auth at each hop.
///
/// **Requires `files:read` scope** on the bot token. Without it, Slack returns
/// an infinite redirect loop between files.slack.com and the workspace domain.
async fn download_slack_file(
    client: &reqwest::Client,
    bot_token: &str,
    initial_url: &str,
) -> Result<Vec<u8>> {
    let no_redirect_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_else(|_| client.clone());

    let mut url = initial_url.to_string();
    let mut seen_urls = std::collections::HashSet::new();
    let max_redirects = 5;

    for hop in 0..max_redirects {
        if !seen_urls.insert(url.clone()) {
            return Err(anyhow::anyhow!(
                "Slack file download redirect loop detected at: {url}. \
                 This usually means the bot token is missing the 'files:read' scope. \
                 Add it in your Slack app's OAuth settings and reinstall."
            ));
        }

        // Always send auth on the first hop (initial URL from Slack API).
        // On redirect hops, only send auth to Slack-owned domains to prevent
        // token leakage to third-party CDNs.
        let mut req = no_redirect_client.get(&url);
        if hop == 0 || is_slack_domain(&url) {
            req = req.header("Authorization", format!("Bearer {bot_token}"));
        }
        let resp = req.send().await?;

        let status = resp.status();
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string();

        info!(
            "Slack file download hop {}: status={}, content-type={}",
            hop, status, content_type
        );

        if status.is_redirection() {
            let location = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| anyhow::anyhow!("Redirect with no Location header"))?;

            // Resolve ?redir= parameters to direct file paths
            url = resolve_slack_redirect(location);
            continue;
        }

        if !status.is_success() {
            return Err(anyhow::anyhow!(
                "Slack file download failed: status={status}, content-type={content_type}"
            ));
        }

        // Pre-check Content-Length before downloading the full body
        if let Some(len) = resp.content_length()
            && len > MAX_AUDIO_DOWNLOAD as u64
        {
            return Err(anyhow::anyhow!(
                "Slack file too large ({len} bytes, max {MAX_AUDIO_DOWNLOAD})"
            ));
        }

        let bytes = resp.bytes().await?.to_vec();
        if bytes.is_empty() {
            return Err(anyhow::anyhow!("Slack file download returned empty body"));
        }
        return Ok(bytes);
    }

    Err(anyhow::anyhow!(
        "Slack file download exceeded {max_redirects} redirects"
    ))
}

/// Resolve a Slack redirect URL to a direct file URL.
///
/// Slack redirects `files.slack.com/files-pri/...` to
/// `workspace.slack.com/?redir=%2Ffiles-pri%2F...` — a login page.
/// This extracts the file path from the `redir` param and constructs
/// a direct URL: `https://workspace.slack.com/files-pri/...`
fn resolve_slack_redirect(location: &str) -> String {
    if let Ok(url) = url::Url::parse(location) {
        // Look for ?redir= parameter
        for (key, value) in url.query_pairs() {
            if key == "redir" {
                // Construct direct URL: scheme + host + redir path
                let host = url.host_str().unwrap_or("files.slack.com");
                let scheme = url.scheme();
                let direct = format!("{scheme}://{host}{value}");
                return direct;
            }
        }
    }
    // No redir param — use the location as-is
    location.to_string()
}

/// Check if bytes start with known image magic bytes.
use crate::media_utils::is_image_magic_bytes;

/// Convert unified `metadata["buttons"]` to Slack Block Kit action blocks.
///
/// Input format: `[{"id": "yes", "label": "Yes", "style": "primary"}, ...]`
/// Output: Vec of Block Kit action block JSON values.
fn convert_buttons_to_blocks(metadata: &HashMap<String, Value>) -> Vec<Value> {
    let Some(buttons_val) = metadata.get(oxicrab_core::bus::events::meta::BUTTONS) else {
        return Vec::new();
    };
    let Some(buttons_arr) = buttons_val.as_array() else {
        return Vec::new();
    };
    if buttons_arr.is_empty() {
        return Vec::new();
    }

    let elements: Vec<Value> = buttons_arr
        .iter()
        .filter_map(|b| {
            let id = b["id"].as_str()?;
            let label = b["label"].as_str().unwrap_or(id);
            let mut btn = serde_json::json!({
                "type": "button",
                "text": {"type": "plain_text", "text": label},
                "action_id": id,
            });
            // Carry context data in the button's value field (returned on click)
            if let Some(ctx) = b["context"].as_str() {
                btn["value"] = Value::String(ctx.to_string());
            }
            // Slack only supports "primary" and "danger" styles
            match b["style"].as_str() {
                Some("primary") => {
                    btn["style"] = Value::String("primary".to_string());
                }
                Some("danger") => {
                    btn["style"] = Value::String("danger".to_string());
                }
                _ => {} // omit style for secondary/success/unknown
            }
            Some(btn)
        })
        .collect();

    if elements.is_empty() {
        return Vec::new();
    }

    vec![serde_json::json!({
        "type": "actions",
        "elements": elements,
    })]
}

/// Handle a Slack interactive payload (button clicks via Socket Mode).
///
/// Parses `block_actions` payloads and creates an `InboundMessage` with
/// `[button:{action_id}]` content, matching Discord's button click format.
#[allow(clippy::too_many_arguments)]
async fn handle_interactive_payload(
    payload: &Value,
    inbound_tx: &Arc<mpsc::Sender<InboundMessage>>,
    allow_from: &[String],
    allow_groups: &[String],
    dm_policy: &oxicrab_core::config::schema::DmPolicy,
    bot_token: &str,
    client: &reqwest::Client,
    thinking_emoji: &str,
) -> Result<()> {
    let payload_type = payload["type"].as_str().unwrap_or_default();
    if payload_type != "block_actions" {
        debug!("slack: ignoring interactive payload type: {payload_type}");
        return Ok(());
    }

    let actions = payload["actions"].as_array();
    let Some(actions) = actions else {
        return Ok(());
    };
    let Some(first_action) = actions.first() else {
        return Ok(());
    };

    let action_id = first_action["action_id"].as_str().unwrap_or_default();
    let action_value = first_action["value"].as_str().unwrap_or_default();
    let user_id = payload["user"]["id"].as_str().unwrap_or_default();
    let channel_id = payload["channel"]["id"].as_str().unwrap_or_default();
    let message_ts = payload["message"]["ts"].as_str().unwrap_or_default();

    if action_id.is_empty() || user_id.is_empty() || channel_id.is_empty() {
        return Ok(());
    }

    let is_dm = channel_id.starts_with('D');
    // Access control: same checks as regular messages
    if !is_dm && !check_group_access(channel_id, allow_groups) {
        debug!("slack: ignoring button click from non-allowed channel {channel_id}");
        return Ok(());
    }
    if is_dm {
        match check_dm_access(user_id, allow_from, "slack", dm_policy) {
            DmCheckResult::Allowed => {}
            DmCheckResult::PairingRequired { code } => {
                let reply = format_pairing_reply("slack", user_id, &code);
                let _ = client
                    .post("https://slack.com/api/chat.postMessage")
                    .bearer_auth(bot_token)
                    .form(&[("channel", channel_id), ("text", &reply)])
                    .send()
                    .await;
                return Ok(());
            }
            DmCheckResult::Denied => {
                return Ok(());
            }
        }
    }

    // Try to parse button context as ActionDispatchPayload for direct dispatch
    let (content, dispatch) = if action_value.is_empty() {
        (format!("[button:{action_id}]"), None)
    } else if let Ok(payload) =
        serde_json::from_str::<crate::dispatch::ActionDispatchPayload>(action_value)
    {
        let dispatch = oxicrab_core::dispatch::ActionDispatch {
            tool: payload.tool,
            params: payload.params,
            source: oxicrab_core::dispatch::ActionSource::Button {
                action_id: action_id.to_string(),
            },
        };
        (format!("[button:{action_id}]"), Some(dispatch))
    } else {
        // Legacy fallback: send as text to LLM
        (
            format!("[button:{action_id}]\nButton context: {action_value}"),
            None,
        )
    };
    let is_group = !is_dm;
    let mut builder = InboundMessage::builder(
        "slack",
        user_id.to_string(),
        channel_id.to_string(),
        content,
    )
    .meta("user_id", Value::String(user_id.to_string()))
    .meta("action_id", Value::String(action_id.to_string()))
    .is_group(is_group);
    if !action_value.is_empty() {
        builder = builder.meta("button_context", Value::String(action_value.to_string()));
    }
    if !message_ts.is_empty() {
        builder = builder.meta(
            oxicrab_core::bus::events::meta::TS,
            Value::String(message_ts.to_string()),
        );
    }
    if let Some(d) = dispatch {
        builder = builder.action(d);
    }
    let inbound_msg = builder.build();

    inbound_tx
        .send(inbound_msg)
        .await
        .map_err(|e| anyhow::anyhow!("Send error: {e}"))?;

    // Add thinking reaction to acknowledge button click (fire-and-forget)
    if !message_ts.is_empty() {
        let react_client = client.clone();
        let react_token = bot_token.to_string();
        let react_channel = channel_id.to_string();
        let react_ts = message_ts.to_string();
        let emoji = thinking_emoji.to_string();
        tokio::spawn(async move {
            let _ = react_client
                .post("https://slack.com/api/reactions.add")
                .form(&[
                    ("token", react_token.as_str()),
                    ("channel", react_channel.as_str()),
                    ("timestamp", react_ts.as_str()),
                    ("name", emoji.as_str()),
                ])
                .send()
                .await;
        });
    }

    info!("slack: button click action_id={action_id} from user={user_id} in channel={channel_id}");
    Ok(())
}

/// Standalone message handler that uses shared state instead of constructing a new `SlackChannel`.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
async fn handle_slack_event(
    event: &Value,
    bot_user_id: &Arc<tokio::sync::Mutex<Option<String>>>,
    mention_regex: &Arc<tokio::sync::Mutex<Option<regex::Regex>>>,
    seen_messages: &Arc<tokio::sync::Mutex<indexmap::IndexSet<String>>>,
    user_cache: &Arc<tokio::sync::Mutex<lru::LruCache<String, String>>>,
    inbound_tx: &Arc<mpsc::Sender<InboundMessage>>,
    allow_from: &[String],
    allow_groups: &[String],
    dm_policy: &oxicrab_core::config::schema::DmPolicy,
    bot_token: &str,
    client: &reqwest::Client,
    thinking_emoji: &str,
) -> Result<()> {
    // Ignore well-known non-user subtypes. Unknown subtypes pass through (safe default).
    if let Some(subtype) = event.get("subtype").and_then(Value::as_str)
        && IGNORED_SUBTYPES.contains(&subtype)
    {
        return Ok(());
    }

    let user_id = event
        .get("user")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let channel_id = event
        .get("channel")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut text = event
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    if user_id.is_empty() || channel_id.is_empty() {
        return Ok(());
    }

    // Ignore messages from the bot itself
    if let Some(ref bot_id) = *bot_user_id.lock().await
        && user_id == bot_id
    {
        debug!("Ignoring message from bot itself (user_id: {})", user_id);
        return Ok(());
    }

    // Deduplicate messages
    if let Some(ts) = event.get("ts").and_then(Value::as_str) {
        let mut seen = seen_messages.lock().await;
        let msg_key = format!("{channel_id}:{user_id}:{ts}");
        if seen.contains(&msg_key) {
            debug!("Ignoring duplicate Slack message: {}", msg_key);
            return Ok(());
        }
        seen.insert(msg_key.clone());
        // Evict oldest entries when set grows too large (keep ~5000 most
        // recent). Higher retain count reduces the replay window in burst
        // scenarios where rapid eviction could allow reprocessing.
        if seen.len() > 5500 {
            let drain_count = seen.len() - 5000;
            seen.drain(..drain_count);
            debug!("Pruned Slack dedup set to {} entries", seen.len());
        }
    }

    info!("Slack: received message from {} in {}", user_id, channel_id);

    // Strip the bot @mention from text (regex is compiled once at startup)
    if let Some(ref re) = *mention_regex.lock().await {
        text = re.replace_all(&text, "").to_string();
    }

    let is_dm = channel_id.starts_with('D');
    // Check group allowlist for non-DM channels
    if !is_dm && !check_group_access(channel_id, allow_groups) {
        debug!("slack: ignoring message from non-allowed channel {channel_id}");
        return Ok(());
    }
    // DM access check
    if is_dm {
        match check_dm_access(user_id, allow_from, "slack", dm_policy) {
            DmCheckResult::Allowed => {}
            DmCheckResult::PairingRequired { code } => {
                let reply = format_pairing_reply("slack", user_id, &code);
                // Post pairing reply to DM channel
                let _ = client
                    .post("https://slack.com/api/chat.postMessage")
                    .bearer_auth(bot_token)
                    .form(&[("channel", channel_id), ("text", &reply)])
                    .send()
                    .await;
                return Ok(());
            }
            DmCheckResult::Denied => {
                return Ok(());
            }
        }
    }

    let has_files = event
        .get("files")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty());

    if text.trim().is_empty() && !has_files {
        return Ok(());
    }

    // Build sender_id — try to enrich with username (cached per user_id)
    let sender_id = {
        let mut cache = user_cache.lock().await;
        cache.get(user_id).cloned()
    };
    let sender_id = if let Some(cached) = sender_id {
        cached
    } else {
        let mut enriched = user_id.to_string();
        let url = "https://slack.com/api/users.info";
        let mut form = HashMap::new();
        form.insert("token", Value::String(bot_token.to_string()));
        form.insert("user", Value::String(user_id.to_string()));
        if let Ok(response) = client.post(url).form(&form).send().await
            && let Ok(user_info) = response.json::<Value>().await
            && let Some(name) = user_info
                .get("user")
                .and_then(|u| u.get("name"))
                .and_then(|n| n.as_str())
        {
            enriched = format!("{user_id}|{name}");
        }
        {
            let mut cache = user_cache.lock().await;
            cache.put(user_id.to_string(), enriched.clone());
        }
        enriched
    };

    // Handle file attachments
    let mut media_paths = Vec::new();
    let mut content_parts = vec![text.clone()];

    if let Some(files) = event.get("files").and_then(Value::as_array) {
        for file in files {
            if let Some(mimetype) = file.get("mimetype").and_then(Value::as_str) {
                // Slack voice clips use subtype "slack_audio" but often have
                // video/mp4 or video/webm as their MIME type
                let is_slack_voice = file
                    .get("subtype")
                    .and_then(Value::as_str)
                    .is_some_and(|s| s == "slack_audio");
                let is_audio = mimetype.starts_with("audio/") || is_slack_voice;

                if mimetype.starts_with("image/") && !is_slack_voice {
                    if let (Some(file_url), Some(file_id)) = (
                        file.get("url_private_download").and_then(Value::as_str),
                        file.get("id").and_then(Value::as_str),
                    ) {
                        let ext = match mimetype {
                            "image/jpeg" => ".jpg",
                            "image/png" => ".png",
                            "image/gif" => ".gif",
                            "image/webp" => ".webp",
                            _ => ".bin",
                        };
                        let Ok(media_dir) = crate::media_utils::media_dir() else {
                            warn!("Failed to create media directory");
                            continue;
                        };
                        let file_path = media_dir.join(format!("slack_{file_id}{ext}"));

                        // Download with manual redirect following.
                        // Slack redirects through multiple hops (files.slack.com
                        // -> workspace.slack.com/?redir=... -> CDN). We follow
                        // each hop manually, re-adding auth and resolving
                        // Slack's ?redir= login-page URLs to direct file paths.
                        match download_slack_file(client, bot_token, file_url).await {
                            Ok(bytes) => {
                                if bytes.len() > MAX_IMAGE_DOWNLOAD {
                                    warn!(
                                        "Slack file too large ({} bytes, max {}), skipping",
                                        bytes.len(),
                                        MAX_IMAGE_DOWNLOAD
                                    );
                                } else if is_image_magic_bytes(&bytes) {
                                    info!("Downloaded Slack image: {} bytes", bytes.len());
                                    let fp = file_path.clone();
                                    let b = bytes.clone();
                                    if let Err(e) =
                                        tokio::task::spawn_blocking(move || std::fs::write(&fp, &b))
                                            .await
                                            .unwrap_or_else(|e| Err(std::io::Error::other(e)))
                                    {
                                        warn!("Failed to write Slack media file: {}", e);
                                    }
                                    let path_str = file_path.to_string_lossy().to_string();
                                    media_paths.push(path_str.clone());
                                    content_parts.push(format!("[image: {path_str}]"));
                                } else {
                                    warn!(
                                        "Slack file doesn't look like an image (first bytes: {:02x?}, {} bytes)",
                                        &bytes[..8.min(bytes.len())],
                                        bytes.len()
                                    );
                                }
                            }
                            Err(e) => warn!("Failed to download Slack file: {}", e),
                        }
                    }
                } else if is_audio {
                    if let (Some(file_url), Some(file_id)) = (
                        file.get("url_private_download").and_then(Value::as_str),
                        file.get("id").and_then(Value::as_str),
                    ) {
                        let ext = match mimetype {
                            "audio/mpeg" => ".mp3",
                            "audio/wav" => ".wav",
                            "audio/webm" | "video/webm" => ".webm",
                            "audio/mp4" | "video/mp4" => ".mp4",
                            "audio/flac" => ".flac",
                            _ => ".ogg",
                        };
                        let Ok(media_dir) = crate::media_utils::media_dir() else {
                            warn!("Failed to create media directory");
                            continue;
                        };
                        let file_path = media_dir.join(format!("slack_{file_id}{ext}"));

                        match download_slack_file(client, bot_token, file_url).await {
                            Ok(bytes) => {
                                if bytes.len() > MAX_AUDIO_DOWNLOAD {
                                    warn!(
                                        "Slack audio too large ({} bytes, max {}), skipping",
                                        bytes.len(),
                                        MAX_AUDIO_DOWNLOAD
                                    );
                                    continue;
                                }
                                info!("Downloaded Slack audio: {} bytes", bytes.len());
                                let fp = file_path.clone();
                                let b = bytes.clone();
                                if let Err(e) =
                                    tokio::task::spawn_blocking(move || std::fs::write(&fp, &b))
                                        .await
                                        .unwrap_or_else(|e| Err(std::io::Error::other(e)))
                                {
                                    warn!("Failed to write Slack audio file: {}", e);
                                }
                                let path_str = file_path.to_string_lossy().to_string();
                                media_paths.push(path_str.clone());
                                content_parts.push(format!("[audio: {path_str}]"));
                            }
                            Err(e) => warn!("Failed to download Slack audio file: {}", e),
                        }
                    }
                } else {
                    let file_name = file
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    content_parts.push(format!("[file: {file_name}]"));
                }
            }
        }
    }

    let content = if content_parts.len() > 1 {
        content_parts.join("\n")
    } else {
        text
    };

    // Add thinking reaction to acknowledge receipt (fire-and-forget)
    if let Some(ts) = event.get("ts").and_then(Value::as_str) {
        let react_client = client.clone();
        let react_token = bot_token.to_string();
        let react_channel = channel_id.to_string();
        let react_ts = ts.to_string();
        let emoji = thinking_emoji.to_string();
        tokio::spawn(async move {
            let _ = react_client
                .post("https://slack.com/api/reactions.add")
                .form(&[
                    ("token", react_token.as_str()),
                    ("channel", react_channel.as_str()),
                    ("timestamp", react_ts.as_str()),
                    ("name", emoji.as_str()),
                ])
                .send()
                .await;
        });
    }

    let is_group = !channel_id.starts_with('D');
    let mut builder = InboundMessage::builder("slack", sender_id, channel_id.to_string(), content)
        .media(media_paths)
        .meta("user_id", Value::String(user_id.to_string()))
        .is_group(is_group);
    if let Some(ts) = event.get("ts").and_then(Value::as_str) {
        builder = builder.meta("ts", Value::String(ts.to_string()));
    }
    let inbound_msg = builder.build();

    inbound_tx
        .send(inbound_msg)
        .await
        .map_err(|e| anyhow::anyhow!("Send error: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests;
