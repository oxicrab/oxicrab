use crate::bus::{InboundMessage, OutboundMessage};
use crate::channels::base::{BaseChannel, split_message};
use crate::channels::utils::{check_allowed_sender, exponential_backoff_delay};
use crate::config::SlackConfig;
use crate::utils::regex::{RegexPatterns, compile_slack_mention};
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use futures_util::SinkExt;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

pub struct SlackChannel {
    config: SlackConfig,
    inbound_tx: Arc<mpsc::Sender<InboundMessage>>,
    bot_user_id: Arc<tokio::sync::Mutex<Option<String>>>,
    running: Arc<tokio::sync::Mutex<bool>>,
    ws_handle: Option<tokio::task::JoinHandle<()>>,
    seen_messages: Arc<tokio::sync::Mutex<indexmap::IndexSet<String>>>,
    user_cache: Arc<tokio::sync::Mutex<HashMap<String, String>>>,
    client: reqwest::Client,
}

impl SlackChannel {
    pub fn new(config: SlackConfig, inbound_tx: Arc<mpsc::Sender<InboundMessage>>) -> Self {
        Self {
            config,
            inbound_tx,
            bot_user_id: Arc::new(tokio::sync::Mutex::new(None)),
            running: Arc::new(tokio::sync::Mutex::new(false)),
            ws_handle: None,
            seen_messages: Arc::new(tokio::sync::Mutex::new(indexmap::IndexSet::new())),
            user_cache: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    fn format_for_slack(text: &str) -> String {
        if text.is_empty() {
            return String::new();
        }
        // Slack uses *bold* not **bold**
        let text = RegexPatterns::markdown_bold().replace_all(text, r"*$1*");
        // Slack uses ~strike~ not ~~strike~~
        let text = RegexPatterns::markdown_strike().replace_all(&text, r"~$1~");
        // Slack links: [text](url) -> <url|text>
        let re_link = RegexPatterns::markdown_link();
        re_link.replace_all(&text, r"<$2|$1>").to_string()
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

        let file_bytes = std::fs::read(path)?;
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
            return Err(anyhow::anyhow!(
                "Slack file upload step 1 failed: {}",
                error
            ));
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
                .unwrap_or_else(|_| reqwest::multipart::Part::bytes(vec![])),
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
                "Slack file upload POST failed: {} — {}",
                status,
                body
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
            return Err(anyhow::anyhow!(
                "Slack file upload step 3 failed: {}",
                error
            ));
        }

        Ok(())
    }

    async fn send_slack_api(&self, method: &str, params: &HashMap<&str, Value>) -> Result<Value> {
        let url = format!("https://slack.com/api/{}", method);
        let mut form = params.clone();
        form.insert("token", Value::String(self.config.bot_token.clone()));

        let response = self.client.post(&url).form(&form).send().await?;

        let json: Value = response.json().await?;
        if json.get("ok").and_then(Value::as_bool) != Some(true) {
            let error = json
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            return Err(anyhow::anyhow!("Slack API error: {}", error));
        }
        Ok(json)
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

        // Fetch bot's own user ID
        let params = HashMap::new();
        match self.send_slack_api("auth.test", &params).await {
            Ok(auth) => {
                let bot_id = auth
                    .get("user_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string);
                *self.bot_user_id.lock().await = bot_id;
                let user = auth
                    .get("user")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let user_id = auth.get("user_id").and_then(Value::as_str).unwrap_or("");
                info!("Slack bot connected as {} (ID: {})", user, user_id);
            }
            Err(e) => {
                error!("Slack auth_test failed: {}", e);
                return Err(anyhow::anyhow!("Slack auth_test failed: {}", e));
            }
        }

        // Set presence to active (optional - requires users:write scope)
        let mut presence_params = HashMap::new();
        presence_params.insert("presence", Value::String("auto".to_string()));
        if let Err(e) = self
            .send_slack_api("users.setPresence", &presence_params)
            .await
        {
            // Only warn if it's not a missing scope error (which is expected if scope not granted)
            let error_msg = e.to_string();
            if error_msg.contains("missing_scope") {
                debug!("Slack presence setting skipped (missing users:write scope)");
            } else {
                warn!("Failed to set Slack presence: {}", e);
            }
        }

        info!("Starting Slack bot (Socket Mode)...");

        // Connect to Socket Mode via WebSocket
        // Share channel state with the WS task via Arc
        let app_token = self.config.app_token.clone();
        let bot_token = self.config.bot_token.clone();
        let config_allow = self.config.allow_from.clone();
        let inbound_tx = self.inbound_tx.clone();
        let bot_user_id = self.bot_user_id.clone();
        let seen_messages = self.seen_messages.clone();
        let user_cache = self.user_cache.clone();
        let ws_client = self.client.clone();

        let ws_task = tokio::spawn(async move {
            use futures_util::StreamExt;
            use tokio_tungstenite::tungstenite::Message;

            // Slack Socket Mode connection
            // First, call apps.connections.open to get the WebSocket URL
            // Then connect to that URL

            let mut reconnect_attempt = 0u32;
            loop {
                debug!("Attempting to connect to Slack Socket Mode...");
                debug!(
                    "Slack app token configured (length: {} chars)",
                    app_token.len()
                );

                // Get WebSocket URL from Slack API
                let response = match ws_client
                    .post("https://slack.com/api/apps.connections.open")
                    .header("Authorization", format!("Bearer {}", app_token))
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
                        let (mut write, mut read) = ws_stream.split();

                        while let Some(msg) = read.next().await {
                            match msg {
                                Ok(Message::Text(text)) => {
                                    if let Ok(event) = serde_json::from_str::<Value>(&text) {
                                        let event_type =
                                            event.get("type").and_then(Value::as_str).unwrap_or("");

                                        // Handle hello message
                                        if event_type == "hello" {
                                            info!("Received Socket Mode hello message");
                                            continue;
                                        }

                                        // Acknowledge events_api messages via WebSocket
                                        // Slack Socket Mode requires acknowledgments to be sent back through the WebSocket
                                        if event_type == "events_api"
                                            && let Some(envelope_id) = event.get("envelope_id")
                                        {
                                            let envelope_id_str =
                                                envelope_id.as_str().unwrap_or("");
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

                                        // Process the event
                                        if event_type == "events_api"
                                            && let Some(payload) = event.get("payload")
                                            && let Some(event_data) = payload.get("event")
                                        {
                                            let inner_event_type = event_data
                                                .get("type")
                                                .and_then(Value::as_str)
                                                .unwrap_or("");

                                            match inner_event_type {
                                                "message" | "app_mention" => {
                                                    if let Err(e) = handle_slack_event(
                                                        event_data,
                                                        &bot_user_id,
                                                        &seen_messages,
                                                        &user_cache,
                                                        &inbound_tx,
                                                        &config_allow,
                                                        &bot_token,
                                                        &ws_client,
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
                                    reconnect_attempt = 0; // Reset on successful connection
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

        // Split long messages (Slack limit is ~40k but 4000 is more readable)
        for chunk in split_message(&content, 4000) {
            let mut params = HashMap::new();
            params.insert("channel", Value::String(msg.chat_id.clone()));
            params.insert("text", Value::String(chunk));
            params.insert("mrkdwn", Value::Bool(true));

            if let Err(e) = self.send_slack_api("chat.postMessage", &params).await {
                error!("Error sending Slack message: {}", e);
                return Err(anyhow::anyhow!("Slack send error: {}", e));
            }
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
        let response = self.send_slack_api("chat.postMessage", &params).await?;
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
        self.send_slack_api("chat.update", &params).await?;
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
                "Slack file download redirect loop detected at: {}. \
                 This usually means the bot token is missing the 'files:read' scope. \
                 Add it in your Slack app's OAuth settings and reinstall.",
                url
            ));
        }

        let resp = no_redirect_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", bot_token))
            .send()
            .await?;

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
                "Slack file download failed: status={}, content-type={}",
                status,
                content_type
            ));
        }

        let bytes = resp.bytes().await?.to_vec();
        if bytes.is_empty() {
            return Err(anyhow::anyhow!("Slack file download returned empty body"));
        }
        return Ok(bytes);
    }

    Err(anyhow::anyhow!(
        "Slack file download exceeded {} redirects",
        max_redirects
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
                let direct = format!("{}://{}{}", scheme, host, value);
                return direct;
            }
        }
    }
    // No redir param — use the location as-is
    location.to_string()
}

/// Check if bytes start with known image magic bytes.
use crate::utils::media::is_image_magic_bytes;

/// Standalone message handler that uses shared state instead of constructing a new `SlackChannel`.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
async fn handle_slack_event(
    event: &Value,
    bot_user_id: &Arc<tokio::sync::Mutex<Option<String>>>,
    seen_messages: &Arc<tokio::sync::Mutex<indexmap::IndexSet<String>>>,
    user_cache: &Arc<tokio::sync::Mutex<HashMap<String, String>>>,
    inbound_tx: &Arc<mpsc::Sender<InboundMessage>>,
    allow_from: &[String],
    bot_token: &str,
    client: &reqwest::Client,
) -> Result<()> {
    // Ignore bot messages and message_changed subtypes, but allow file_share
    if let Some(subtype) = event.get("subtype").and_then(Value::as_str)
        && subtype != "file_share"
    {
        return Ok(());
    }

    let user_id = event.get("user").and_then(Value::as_str).unwrap_or("");
    let channel_id = event.get("channel").and_then(Value::as_str).unwrap_or("");
    let mut text = event
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
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
        let msg_key = format!("{}:{}:{}", channel_id, user_id, ts);
        if seen.contains(&msg_key) {
            debug!("Ignoring duplicate Slack message: {}", msg_key);
            return Ok(());
        }
        seen.insert(msg_key.clone());
        // Evict oldest entries when set grows too large (keep ~500 most recent).
        // IndexSet preserves insertion order, so drain from the front.
        if seen.len() > 1000 {
            let drain_count = seen.len() - 500;
            seen.drain(..drain_count);
            debug!("Pruned Slack dedup set to {} entries", seen.len());
        }
    }

    info!("Slack: received message from {} in {}", user_id, channel_id);

    // Strip the bot @mention from text
    if let Some(ref bot_id) = *bot_user_id.lock().await {
        match compile_slack_mention(bot_id) {
            Ok(re_mention) => {
                text = re_mention.replace_all(&text, "").to_string();
            }
            Err(e) => {
                warn!("Failed to compile Slack mention regex: {}", e);
            }
        }
    }

    if !check_allowed_sender(user_id, allow_from) {
        return Ok(());
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
        let cache = user_cache.lock().await;
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
            enriched = format!("{}|{}", user_id, name);
        }
        user_cache
            .lock()
            .await
            .insert(user_id.to_string(), enriched.clone());
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
                        let Ok(media_dir) = crate::utils::media::media_dir() else {
                            warn!("Failed to create media directory");
                            continue;
                        };
                        let file_path = media_dir.join(format!("slack_{}{}", file_id, ext));

                        // Download with manual redirect following.
                        // Slack redirects through multiple hops (files.slack.com
                        // -> workspace.slack.com/?redir=... -> CDN). We follow
                        // each hop manually, re-adding auth and resolving
                        // Slack's ?redir= login-page URLs to direct file paths.
                        match download_slack_file(client, bot_token, file_url).await {
                            Ok(bytes) => {
                                if is_image_magic_bytes(&bytes) {
                                    info!("Downloaded Slack image: {} bytes", bytes.len());
                                    if let Err(e) = std::fs::write(&file_path, &bytes) {
                                        warn!("Failed to write Slack media file: {}", e);
                                    }
                                    let path_str = file_path.to_string_lossy().to_string();
                                    media_paths.push(path_str.clone());
                                    content_parts.push(format!("[image: {}]", path_str));
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
                        let Ok(media_dir) = crate::utils::media::media_dir() else {
                            warn!("Failed to create media directory");
                            continue;
                        };
                        let file_path = media_dir.join(format!("slack_{}{}", file_id, ext));

                        match download_slack_file(client, bot_token, file_url).await {
                            Ok(bytes) => {
                                info!("Downloaded Slack audio: {} bytes", bytes.len());
                                if let Err(e) = std::fs::write(&file_path, &bytes) {
                                    warn!("Failed to write Slack audio file: {}", e);
                                }
                                let path_str = file_path.to_string_lossy().to_string();
                                media_paths.push(path_str.clone());
                                content_parts.push(format!("[audio: {}]", path_str));
                            }
                            Err(e) => warn!("Failed to download Slack audio file: {}", e),
                        }
                    }
                } else {
                    let file_name = file
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    content_parts.push(format!("[file: {}]", file_name));
                }
            }
        }
    }

    let content = if content_parts.len() > 1 {
        content_parts.join("\n")
    } else {
        text
    };

    // Add "eyes" reaction to acknowledge receipt (fire-and-forget)
    if let Some(ts) = event.get("ts").and_then(Value::as_str) {
        let react_client = client.clone();
        let react_token = bot_token.to_string();
        let react_channel = channel_id.to_string();
        let react_ts = ts.to_string();
        tokio::spawn(async move {
            let _ = react_client
                .post("https://slack.com/api/reactions.add")
                .form(&[
                    ("token", react_token.as_str()),
                    ("channel", react_channel.as_str()),
                    ("timestamp", react_ts.as_str()),
                    ("name", "eyes"),
                ])
                .send()
                .await;
        });
    }

    let inbound_msg = InboundMessage {
        channel: "slack".to_string(),
        sender_id,
        chat_id: channel_id.to_string(),
        content,
        timestamp: Utc::now(),
        media: media_paths,
        metadata: {
            let mut meta = HashMap::new();
            if let Some(ts) = event.get("ts").and_then(Value::as_str) {
                meta.insert("ts".to_string(), Value::String(ts.to_string()));
            }
            meta.insert("user_id".to_string(), Value::String(user_id.to_string()));
            meta
        },
    };

    inbound_tx
        .send(inbound_msg)
        .await
        .map_err(|e| anyhow::anyhow!("Send error: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests;
