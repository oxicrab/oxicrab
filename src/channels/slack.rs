use crate::bus::{InboundMessage, OutboundMessage};
use crate::channels::base::{split_message, BaseChannel};
use crate::channels::utils::{check_allowed_sender, exponential_backoff_delay};
use crate::config::SlackConfig;
use crate::utils::regex::{compile_slack_mention, RegexPatterns};
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
    seen_messages: Arc<tokio::sync::Mutex<std::collections::HashSet<String>>>,
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
            seen_messages: Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new())),
            client: reqwest::Client::new(),
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

    async fn send_slack_api(&self, method: &str, params: &HashMap<&str, Value>) -> Result<Value> {
        let url = format!("https://slack.com/api/{}", method);
        let mut form = params.clone();
        form.insert("token", Value::String(self.config.bot_token.clone()));

        let response = self.client.post(&url).form(&form).send().await?;

        let json: Value = response.json().await?;
        if json.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let error = json
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            return Err(anyhow::anyhow!("Slack API error: {}", error));
        }
        Ok(json)
    }
}

#[async_trait]
impl BaseChannel for SlackChannel {
    fn name(&self) -> &str {
        "slack"
    }

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
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                *self.bot_user_id.lock().await = bot_id;
                let user = auth
                    .get("user")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let user_id = auth.get("user_id").and_then(|v| v.as_str()).unwrap_or("");
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
            if !error_msg.contains("missing_scope") {
                warn!("Failed to set Slack presence: {}", e);
            } else {
                debug!("Slack presence setting skipped (missing users:write scope)");
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

                if json.get("ok").and_then(|v| v.as_bool()) != Some(true) {
                    let error = json
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    error!("Slack apps.connections.open error: {}", error);
                    if error == "invalid_auth" {
                        warn!("Invalid app_token - check that it starts with 'xapp-' and has 'connections:write' scope");
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

                let ws_url = match json.get("url").and_then(|v| v.as_str()) {
                    Some(url) => url,
                    None => {
                        error!("No 'url' field in apps.connections.open response");
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
                                        let event_type = event
                                            .get("type")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");

                                        // Handle hello message
                                        if event_type == "hello" {
                                            info!("Received Socket Mode hello message");
                                            continue;
                                        }

                                        // Acknowledge events_api messages via WebSocket
                                        // Slack Socket Mode requires acknowledgments to be sent back through the WebSocket
                                        if event_type == "events_api" {
                                            if let Some(envelope_id) = event.get("envelope_id") {
                                                let envelope_id_str =
                                                    envelope_id.as_str().unwrap_or("");
                                                let ack_msg = serde_json::json!({
                                                    "envelope_id": envelope_id_str,
                                                    "payload": {}
                                                });
                                                debug!("Sending Socket Mode acknowledgment for envelope_id: {}", envelope_id_str);
                                                if let Err(e) = write
                                                    .send(Message::text(ack_msg.to_string()))
                                                    .await
                                                {
                                                    error!("Failed to send Socket Mode acknowledgment: {}", e);
                                                }
                                            }
                                        }

                                        // Process the event
                                        if event_type == "events_api" {
                                            if let Some(payload) = event.get("payload") {
                                                if let Some(event_data) = payload.get("event") {
                                                    let inner_event_type = event_data
                                                        .get("type")
                                                        .and_then(|v| v.as_str())
                                                        .unwrap_or("");

                                                    match inner_event_type {
                                                        "message" | "app_mention" => {
                                                            if let Err(e) = handle_slack_event(
                                                                event_data,
                                                                &bot_user_id,
                                                                &seen_messages,
                                                                &inbound_tx,
                                                                &config_allow,
                                                                &bot_token,
                                                                &ws_client,
                                                            )
                                                            .await
                                                            {
                                                                error!("Error handling Slack message event: {}", e);
                                                            }
                                                        }
                                                        _ => {}
                                                    }
                                                }
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
                            warn!("Make sure your app_token starts with 'xapp-' and is a Socket Mode token.");
                            warn!("You can generate a new token at: https://api.slack.com/apps/<your-app-id>/socket-mode");
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
        // Slack doesn't have a dedicated typing indicator API for bots,
        // but posting an ephemeral "typing" status can be simulated via
        // a short-lived message. For now we simply no-op successfully
        // so the agent loop doesn't treat it as an error.
        debug!("Slack typing indicator requested for {}", chat_id);
        Ok(())
    }

    async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        if msg.channel != "slack" {
            return Ok(());
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
fn is_image_magic_bytes(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }
    // PNG: 89 50 4E 47
    if data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        return true;
    }
    // JPEG: FF D8 FF
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return true;
    }
    // GIF: GIF87a or GIF89a
    if data.starts_with(b"GIF8") {
        return true;
    }
    // WebP: RIFF....WEBP
    if data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
        return true;
    }
    false
}

/// Standalone message handler that uses shared state instead of constructing a new `SlackChannel`.
async fn handle_slack_event(
    event: &Value,
    bot_user_id: &Arc<tokio::sync::Mutex<Option<String>>>,
    seen_messages: &Arc<tokio::sync::Mutex<std::collections::HashSet<String>>>,
    inbound_tx: &Arc<mpsc::Sender<InboundMessage>>,
    allow_from: &[String],
    bot_token: &str,
    client: &reqwest::Client,
) -> Result<()> {
    // Ignore bot messages and message_changed subtypes, but allow file_share
    if let Some(subtype) = event.get("subtype").and_then(|v| v.as_str()) {
        if subtype != "file_share" {
            return Ok(());
        }
    }

    let user_id = event.get("user").and_then(|v| v.as_str()).unwrap_or("");
    let channel_id = event.get("channel").and_then(|v| v.as_str()).unwrap_or("");
    let mut text = event
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if user_id.is_empty() || channel_id.is_empty() {
        return Ok(());
    }

    // Ignore messages from the bot itself
    if let Some(ref bot_id) = *bot_user_id.lock().await {
        if user_id == bot_id {
            debug!("Ignoring message from bot itself (user_id: {})", user_id);
            return Ok(());
        }
    }

    // Deduplicate messages
    if let Some(ts) = event.get("ts").and_then(|v| v.as_str()) {
        let mut seen = seen_messages.lock().await;
        let msg_key = format!("{}:{}:{}", channel_id, user_id, ts);
        if seen.contains(&msg_key) {
            debug!("Ignoring duplicate Slack message: {}", msg_key);
            return Ok(());
        }
        seen.insert(msg_key.clone());
        // Evict oldest entries when set grows too large (keep ~500 most recent).
        // HashSet has no ordering; we remove a random half to avoid unbounded growth
        // while retaining enough entries to catch most duplicates.
        if seen.len() > 1000 {
            let to_remove: Vec<String> = seen.iter().take(500).cloned().collect();
            for key in to_remove {
                seen.remove(&key);
            }
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
        .and_then(|v| v.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);

    if text.trim().is_empty() && !has_files {
        return Ok(());
    }

    // Build sender_id — try to enrich with username
    let mut sender_id = user_id.to_string();
    let url = "https://slack.com/api/users.info";
    let mut form = HashMap::new();
    form.insert("token", Value::String(bot_token.to_string()));
    form.insert("user", Value::String(user_id.to_string()));
    if let Ok(response) = client.post(url).form(&form).send().await {
        if let Ok(user_info) = response.json::<Value>().await {
            if let Some(name) = user_info
                .get("user")
                .and_then(|u| u.get("name"))
                .and_then(|n| n.as_str())
            {
                sender_id = format!("{}|{}", user_id, name);
            }
        }
    }

    // Handle file attachments
    let mut media_paths = Vec::new();
    let mut content_parts = vec![text.clone()];

    if let Some(files) = event.get("files").and_then(|v| v.as_array()) {
        for file in files {
            if let Some(mimetype) = file.get("mimetype").and_then(|v| v.as_str()) {
                if mimetype.starts_with("image/") {
                    if let (Some(file_url), Some(file_id)) = (
                        file.get("url_private_download").and_then(|v| v.as_str()),
                        file.get("id").and_then(|v| v.as_str()),
                    ) {
                        let ext = match mimetype {
                            "image/jpeg" => ".jpg",
                            "image/png" => ".png",
                            "image/gif" => ".gif",
                            "image/webp" => ".webp",
                            _ => ".bin",
                        };
                        let media_dir = dirs::home_dir()
                            .unwrap_or_else(|| std::path::PathBuf::from("."))
                            .join(".nanobot")
                            .join("media");
                        let _ = std::fs::create_dir_all(&media_dir);
                        let file_path = media_dir.join(format!("slack_{}{}", file_id, ext));

                        // Download with manual redirect following.
                        // Slack redirects through multiple hops (files.slack.com
                        // -> workspace.slack.com/?redir=... -> CDN). We follow
                        // each hop manually, re-adding auth and resolving
                        // Slack's ?redir= login-page URLs to direct file paths.
                        match download_slack_file(client, bot_token, file_url).await {
                            Ok(bytes) => {
                                if !is_image_magic_bytes(&bytes) {
                                    warn!(
                                        "Slack file doesn't look like an image (first bytes: {:02x?}, {} bytes)",
                                        &bytes[..8.min(bytes.len())],
                                        bytes.len()
                                    );
                                } else {
                                    info!("Downloaded Slack image: {} bytes", bytes.len());
                                    let _ = std::fs::write(&file_path, &bytes);
                                    let path_str = file_path.to_string_lossy().to_string();
                                    media_paths.push(path_str.clone());
                                    content_parts.push(format!("[image: {}]", path_str));
                                }
                            }
                            Err(e) => warn!("Failed to download Slack file: {}", e),
                        }
                    }
                } else {
                    let file_name = file
                        .get("name")
                        .and_then(|v| v.as_str())
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
    if let Some(ts) = event.get("ts").and_then(|v| v.as_str()) {
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
            if let Some(ts) = event.get("ts").and_then(|v| v.as_str()) {
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
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // --- resolve_slack_redirect tests ---

    #[test]
    fn test_resolve_slack_redirect_with_redir_param() {
        let location =
            "https://myworkspace.slack.com/?redir=%2Ffiles-pri%2FT123-F456%2Fdownload%2Fimage.png";
        let result = resolve_slack_redirect(location);
        assert_eq!(
            result,
            "https://myworkspace.slack.com/files-pri/T123-F456/download/image.png"
        );
    }

    #[test]
    fn test_resolve_slack_redirect_no_redir_param() {
        let location = "https://cdn.slack.com/files/image.png";
        let result = resolve_slack_redirect(location);
        assert_eq!(result, "https://cdn.slack.com/files/image.png");
    }

    #[test]
    fn test_resolve_slack_redirect_invalid_url() {
        let location = "not-a-url";
        let result = resolve_slack_redirect(location);
        assert_eq!(result, "not-a-url");
    }

    #[test]
    fn test_resolve_slack_redirect_encoded_special_chars() {
        let location = "https://ws.slack.com/?redir=%2Ffiles-pri%2FT1-F2%2Fdownload%2Fscreenshot%202026%4016.45.png";
        let result = resolve_slack_redirect(location);
        assert_eq!(
            result,
            "https://ws.slack.com/files-pri/T1-F2/download/screenshot 2026@16.45.png"
        );
    }

    #[test]
    fn test_resolve_slack_redirect_with_extra_query_params() {
        let location =
            "https://ws.slack.com/?foo=bar&redir=%2Ffiles-pri%2FT1-F2%2Fdownload%2Fimg.png&baz=1";
        let result = resolve_slack_redirect(location);
        assert_eq!(
            result,
            "https://ws.slack.com/files-pri/T1-F2/download/img.png"
        );
    }

    #[test]
    fn test_resolve_slack_redirect_preserves_scheme() {
        let location = "http://ws.slack.com/?redir=%2Ffiles-pri%2FT1-F2%2Fdownload%2Fimg.png";
        let result = resolve_slack_redirect(location);
        assert!(result.starts_with("http://"), "should preserve http scheme");
    }

    // --- is_image_magic_bytes tests ---

    #[test]
    fn test_is_image_magic_bytes_png() {
        assert!(is_image_magic_bytes(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A]));
    }

    #[test]
    fn test_is_image_magic_bytes_jpeg() {
        assert!(is_image_magic_bytes(&[0xFF, 0xD8, 0xFF, 0xE0]));
    }

    #[test]
    fn test_is_image_magic_bytes_gif87a() {
        assert!(is_image_magic_bytes(b"GIF87a..."));
    }

    #[test]
    fn test_is_image_magic_bytes_gif89a() {
        assert!(is_image_magic_bytes(b"GIF89a..."));
    }

    #[test]
    fn test_is_image_magic_bytes_webp() {
        let mut webp = Vec::new();
        webp.extend_from_slice(b"RIFF");
        webp.extend_from_slice(&[0x00; 4]);
        webp.extend_from_slice(b"WEBP");
        assert!(is_image_magic_bytes(&webp));
    }

    #[test]
    fn test_is_image_magic_bytes_html() {
        assert!(!is_image_magic_bytes(b"<!DOCTYPE html>"));
    }

    #[test]
    fn test_is_image_magic_bytes_json() {
        assert!(!is_image_magic_bytes(b"{\"error\": \"missing_scope\"}"));
    }

    #[test]
    fn test_is_image_magic_bytes_too_short() {
        assert!(!is_image_magic_bytes(&[0x89]));
        assert!(!is_image_magic_bytes(&[0xFF, 0xD8]));
        assert!(!is_image_magic_bytes(&[]));
    }

    #[test]
    fn test_is_image_magic_bytes_webp_too_short() {
        assert!(!is_image_magic_bytes(b"RIFF\x00\x00\x00\x00WEB"));
    }

    // --- download_slack_file tests (wiremock) ---

    #[tokio::test]
    async fn test_download_slack_file_success() {
        let server = MockServer::start().await;
        let png_body: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

        Mock::given(method("GET"))
            .and(path("/files-pri/T1-F2/download/image.png"))
            .and(header("Authorization", "Bearer xoxb-test"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(png_body.clone())
                    .insert_header("Content-Type", "image/png"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let url = format!("{}/files-pri/T1-F2/download/image.png", server.uri());
        let result = download_slack_file(&client, "xoxb-test", &url).await;

        assert!(result.is_ok());
        let bytes = result.unwrap();
        assert_eq!(bytes, png_body);
    }

    #[tokio::test]
    async fn test_download_slack_file_sends_auth_header() {
        let server = MockServer::start().await;

        // Only match requests with the correct auth header
        Mock::given(method("GET"))
            .and(path("/file.png"))
            .and(header("Authorization", "Bearer my-secret-token"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0x89, 0x50, 0x4E, 0x47]))
            .expect(1)
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let url = format!("{}/file.png", server.uri());
        let result = download_slack_file(&client, "my-secret-token", &url).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_download_slack_file_error_status() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/file.png"))
            .respond_with(ResponseTemplate::new(403))
            .expect(1)
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let url = format!("{}/file.png", server.uri());
        let result = download_slack_file(&client, "xoxb-test", &url).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("403"));
    }

    #[tokio::test]
    async fn test_download_slack_file_empty_body_is_error() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/file.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(Vec::<u8>::new()))
            .expect(1)
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let url = format!("{}/file.png", server.uri());
        let result = download_slack_file(&client, "xoxb-test", &url).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty body"));
    }

    #[tokio::test]
    async fn test_download_slack_file_follows_single_redirect() {
        let server = MockServer::start().await;
        let jpeg_body: Vec<u8> = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];

        // First request: redirect
        Mock::given(method("GET"))
            .and(path("/start"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/actual.jpg", server.uri())),
            )
            .expect(1)
            .mount(&server)
            .await;

        // Second request: file content
        Mock::given(method("GET"))
            .and(path("/actual.jpg"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(jpeg_body.clone())
                    .insert_header("Content-Type", "image/jpeg"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let url = format!("{}/start", server.uri());
        let result = download_slack_file(&client, "xoxb-test", &url).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), jpeg_body);
    }

    #[tokio::test]
    async fn test_download_slack_file_follows_multiple_redirects() {
        let server = MockServer::start().await;
        let png_body: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47];

        // Hop 0 -> Hop 1
        Mock::given(method("GET"))
            .and(path("/hop0"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/hop1", server.uri())),
            )
            .expect(1)
            .mount(&server)
            .await;

        // Hop 1 -> Hop 2
        Mock::given(method("GET"))
            .and(path("/hop1"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/hop2", server.uri())),
            )
            .expect(1)
            .mount(&server)
            .await;

        // Hop 2 -> final file
        Mock::given(method("GET"))
            .and(path("/hop2"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(png_body.clone()))
            .expect(1)
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let url = format!("{}/hop0", server.uri());
        let result = download_slack_file(&client, "xoxb-test", &url).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), png_body);
    }

    #[tokio::test]
    async fn test_download_slack_file_redirect_preserves_auth_on_each_hop() {
        let server = MockServer::start().await;

        // Both hops require the correct auth header
        Mock::given(method("GET"))
            .and(path("/hop0"))
            .and(header("Authorization", "Bearer xoxb-hop-test"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/hop1", server.uri())),
            )
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/hop1"))
            .and(header("Authorization", "Bearer xoxb-hop-test"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0x89, 0x50, 0x4E, 0x47]))
            .expect(1)
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let url = format!("{}/hop0", server.uri());
        let result = download_slack_file(&client, "xoxb-hop-test", &url).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_download_slack_file_redirect_loop_detection() {
        let server = MockServer::start().await;

        // Always redirect to self
        Mock::given(method("GET"))
            .and(path("/loop"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/loop", server.uri())),
            )
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let url = format!("{}/loop", server.uri());
        let result = download_slack_file(&client, "xoxb-test", &url).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("redirect loop"),
            "Expected redirect loop error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_download_slack_file_redirect_loop_mentions_files_read() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/loop"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/loop", server.uri())),
            )
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let url = format!("{}/loop", server.uri());
        let result = download_slack_file(&client, "xoxb-test", &url).await;

        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("files:read"),
            "Error should mention missing files:read scope, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_download_slack_file_exceeds_max_redirects() {
        let server = MockServer::start().await;

        // Chain of unique redirects that exceeds max_redirects=5.
        // No .expect() — some hops may not be reached before the limit.
        for i in 0..6 {
            Mock::given(method("GET"))
                .and(path(format!("/hop{}", i)))
                .respond_with(
                    ResponseTemplate::new(302)
                        .insert_header("Location", format!("{}/hop{}", server.uri(), i + 1)),
                )
                .mount(&server)
                .await;
        }

        let client = reqwest::Client::new();
        let url = format!("{}/hop0", server.uri());
        let result = download_slack_file(&client, "xoxb-test", &url).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("exceeded"));
    }

    #[tokio::test]
    async fn test_download_slack_file_500_error() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/file.png"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let url = format!("{}/file.png", server.uri());
        let result = download_slack_file(&client, "xoxb-test", &url).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("500"));
    }

    // --- format_for_slack tests ---

    #[test]
    fn test_format_for_slack_bold() {
        assert_eq!(SlackChannel::format_for_slack("**bold**"), "*bold*");
    }

    #[test]
    fn test_format_for_slack_link() {
        assert_eq!(
            SlackChannel::format_for_slack("[text](https://example.com)"),
            "<https://example.com|text>"
        );
    }

    #[test]
    fn test_format_for_slack_strikethrough() {
        assert_eq!(SlackChannel::format_for_slack("~~strike~~"), "~strike~");
    }

    #[test]
    fn test_format_for_slack_empty() {
        assert_eq!(SlackChannel::format_for_slack(""), "");
    }

    #[test]
    fn test_format_for_slack_plain_text() {
        assert_eq!(
            SlackChannel::format_for_slack("no formatting here"),
            "no formatting here"
        );
    }
}
