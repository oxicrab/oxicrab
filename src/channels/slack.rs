use crate::bus::{InboundMessage, OutboundMessage};
use crate::channels::base::{BaseChannel, split_message};
use crate::config::SlackConfig;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use futures_util::SinkExt;

pub struct SlackChannel {
    config: SlackConfig,
    inbound_tx: Arc<mpsc::UnboundedSender<InboundMessage>>,
    bot_user_id: Option<String>,
    running: Arc<tokio::sync::Mutex<bool>>,
    ws_handle: Option<tokio::task::JoinHandle<()>>,
    seen_messages: Arc<tokio::sync::Mutex<std::collections::HashSet<String>>>,
}

impl SlackChannel {
    pub fn new(
        config: SlackConfig,
        inbound_tx: Arc<mpsc::UnboundedSender<InboundMessage>>,
    ) -> Self {
        Self {
            config,
            inbound_tx,
            bot_user_id: None,
            running: Arc::new(tokio::sync::Mutex::new(false)),
            ws_handle: None,
            seen_messages: Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new())),
        }
    }

    fn format_for_slack(text: &str) -> String {
        if text.is_empty() {
            return String::new();
        }
        // Slack uses *bold* not **bold**
        let re_bold = Regex::new(r"\*\*(.+?)\*\*").unwrap();
        let text = re_bold.replace_all(text, r"*\1*");
        // Slack uses ~strike~ not ~~strike~~
        let re_strike = Regex::new(r"~~(.+?)~~").unwrap();
        let text = re_strike.replace_all(&text, r"~\1~");
        // Slack links: [text](url) -> <url|text>
        let re_link = Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").unwrap();
        re_link.replace_all(&text, r"<$2|$1>").to_string()
    }

    async fn send_slack_api(&self, method: &str, params: &HashMap<&str, Value>) -> Result<Value> {
        let client = reqwest::Client::new();
        let url = format!("https://slack.com/api/{}", method);
        let mut form = params.clone();
        form.insert("token", Value::String(self.config.bot_token.clone()));

        let response = client
            .post(&url)
            .form(&form)
            .send()
            .await?;
        
        let json: Value = response.json().await?;
        if json.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let error = json.get("error").and_then(|v| v.as_str()).unwrap_or("unknown");
            return Err(anyhow::anyhow!("Slack API error: {}", error));
        }
        Ok(json)
    }


    async fn handle_message_event(&self, event: &Value) -> Result<()> {
        // Ignore bot messages and message_changed subtypes
        if event.get("subtype").is_some() {
            return Ok(());
        }

        let user_id = event.get("user").and_then(|v| v.as_str()).unwrap_or("");
        let channel_id = event.get("channel").and_then(|v| v.as_str()).unwrap_or("");
        let mut text = event.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();

        if user_id.is_empty() || channel_id.is_empty() {
            return Ok(());
        }

        // Ignore messages from the bot itself
        if let Some(ref bot_id) = self.bot_user_id {
            if user_id == bot_id {
                debug!("Ignoring message from bot itself (user_id: {})", user_id);
                return Ok(());
            }
        }

        // Deduplicate messages using timestamp (ts) as unique identifier
        if let Some(ts) = event.get("ts").and_then(|v| v.as_str()) {
            let mut seen = self.seen_messages.lock().await;
            let msg_key = format!("{}:{}:{}", channel_id, user_id, ts);
            if seen.contains(&msg_key) {
                debug!("Ignoring duplicate Slack message: {}", msg_key);
                return Ok(());
            }
            seen.insert(msg_key);
            // Clean up old entries (keep last 1000)
            if seen.len() > 1000 {
                let entries: Vec<String> = seen.iter().cloned().collect();
                seen.clear();
                // Keep the most recent 1000 entries (simple approach: just clear and rebuild)
                // In practice, we could use a more sophisticated LRU, but this works for now
                for entry in entries.into_iter().take(1000) {
                    seen.insert(entry);
                }
            }
        }

        info!("Slack: received message from {} in {}", user_id, channel_id);

        // Strip the bot @mention from text
        if let Some(ref bot_id) = self.bot_user_id {
            // Escape special regex characters in bot_id
            let escaped_id = regex::escape(bot_id);
            let re_mention = Regex::new(&format!(r"<@{}\s*>\s*", escaped_id)).unwrap();
            text = re_mention.replace_all(&text, "").to_string();
        }

        if text.trim().is_empty() {
            return Ok(());
        }

        // Check allowlist
        if !self.config.allow_from.is_empty() {
            let allowed = self.config.allow_from.iter().any(|a: &String| {
                let a_clean = a.trim_start_matches('+');
                user_id == a || user_id == a_clean || user_id.contains(a_clean)
            });
            if !allowed {
                return Ok(());
            }
        }

        // Build sender_id — try to enrich with username
        let mut sender_id = user_id.to_string();
        let mut params = HashMap::new();
        params.insert("user", Value::String(user_id.to_string()));
        if let Ok(user_info) = self.send_slack_api("users.info", &params).await {
            if let Some(name) = user_info.get("user").and_then(|u| u.get("name")).and_then(|n| n.as_str()) {
                sender_id = format!("{}|{}", user_id, name);
            }
        }

        // Handle file attachments (images)
        let mut media_paths = Vec::new();
        let mut content_parts = vec![text.clone()];

        if let Some(files) = event.get("files").and_then(|v| v.as_array()) {
            for file in files {
                if let Some(mimetype) = file.get("mimetype").and_then(|v| v.as_str()) {
                    if mimetype.starts_with("image/") {
                        if let Some(url) = file.get("url_private_download").and_then(|v| v.as_str()) {
                            if let Some(file_id) = file.get("id").and_then(|v| v.as_str()) {
                                // Download file
                                match self.download_file(url, file_id, mimetype).await {
                                    Ok(path) => {
                                        media_paths.push(path.clone());
                                        content_parts.push(format!("[image: {}]", path));
                                    }
                                    Err(e) => {
                                        warn!("Failed to download Slack file: {}", e);
                                    }
                                }
                            }
                        }
                    } else {
                        let file_name = file.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
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

        debug!("Slack message from {} in {}: {}...", sender_id, channel_id, &content[..content.len().min(50)]);

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

        self.inbound_tx.send(inbound_msg).map_err(|e| anyhow::anyhow!("Send error: {}", e))?;
        Ok(())
    }

    async fn download_file(&self, url: &str, file_id: &str, mimetype: &str) -> Result<String> {
        use std::path::PathBuf;
        let media_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".nanobot")
            .join("media");
        std::fs::create_dir_all(&media_dir)?;

        let ext = match mimetype {
            "image/jpeg" => ".jpg",
            "image/png" => ".png",
            "image/gif" => ".gif",
            "image/webp" => ".webp",
            _ => ".bin",
        };
        let file_path = media_dir.join(format!("slack_{}{}", file_id, ext));

        let client = reqwest::Client::new();
        let response = client
            .get(url)
            .header("Authorization", format!("Bearer {}", self.config.bot_token))
            .send()
            .await?;
        let bytes = response.error_for_status()?.bytes().await?;
        std::fs::write(&file_path, bytes)?;

        Ok(file_path.to_string_lossy().to_string())
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
                self.bot_user_id = auth.get("user_id").and_then(|v| v.as_str()).map(|s| s.to_string());
                let user = auth.get("user").and_then(|v| v.as_str()).unwrap_or("unknown");
                let user_id = auth.get("user_id").and_then(|v| v.as_str()).unwrap_or("");
                info!("Slack bot connected as {} (ID: {})", user, user_id);
            }
            Err(e) => {
                error!("Slack auth_test failed: {}", e);
                return Err(anyhow::anyhow!("Slack auth_test failed: {}", e));
            }
        }

        // Set presence to active
        let mut presence_params = HashMap::new();
        presence_params.insert("presence", Value::String("auto".to_string()));
        if let Err(e) = self.send_slack_api("users.setPresence", &presence_params).await {
            warn!("Failed to set Slack presence: {}", e);
        }

        info!("Starting Slack bot (Socket Mode)...");

        // Connect to Socket Mode via WebSocket
        let app_token = self.config.app_token.clone();
        let inbound_tx = self.inbound_tx.clone();
        let config_allow = self.config.allow_from.clone();
        let bot_user_id = self.bot_user_id.clone();
        let bot_token = self.config.bot_token.clone();

        let ws_task = tokio::spawn(async move {
            use tokio_tungstenite::tungstenite::Message;
            use futures_util::StreamExt;

            // Slack Socket Mode connection
            // First, call apps.connections.open to get the WebSocket URL
            // Then connect to that URL
            
            loop {
                debug!("Attempting to connect to Slack Socket Mode...");
                debug!("Token starts with: {}", app_token.chars().take(10).collect::<String>());
                debug!("Token length: {} characters", app_token.len());
                
                // Get WebSocket URL from Slack API
                let client = reqwest::Client::new();
                let response = match client
                    .post("https://slack.com/api/apps.connections.open")
                    .header("Authorization", format!("Bearer {}", app_token))
                    .send()
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        error!("Failed to call apps.connections.open: {}", e);
                        warn!("Retrying Slack Socket Mode connection in 5 seconds...");
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                        continue;
                    }
                };
                
                let json: Value = match response.json().await {
                    Ok(j) => j,
                    Err(e) => {
                        error!("Failed to parse apps.connections.open response: {}", e);
                        warn!("Retrying Slack Socket Mode connection in 5 seconds...");
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                        continue;
                    }
                };
                
                if json.get("ok").and_then(|v| v.as_bool()) != Some(true) {
                    let error = json.get("error").and_then(|v| v.as_str()).unwrap_or("unknown");
                    error!("Slack apps.connections.open error: {}", error);
                    if error == "invalid_auth" {
                        warn!("Invalid app_token - check that it starts with 'xapp-' and has 'connections:write' scope");
                    }
                    warn!("Retrying Slack Socket Mode connection in 5 seconds...");
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    continue;
                }
                
                let ws_url = match json.get("url").and_then(|v| v.as_str()) {
                    Some(url) => url,
                    None => {
                        error!("No 'url' field in apps.connections.open response");
                        warn!("Retrying Slack Socket Mode connection in 5 seconds...");
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                        continue;
                    }
                };
                
                debug!("Received WebSocket URL from Slack (length: {} chars)", ws_url.len());
                
                let url = match url::Url::parse(ws_url) {
                    Ok(u) => u,
                    Err(e) => {
                        error!("Failed to parse WebSocket URL: {}", e);
                        warn!("Retrying Slack Socket Mode connection in 5 seconds...");
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                        continue;
                    }
                };
                
                match tokio_tungstenite::connect_async(url.as_str()).await {
                    Ok((ws_stream, response)) => {
                        info!("Connected to Slack Socket Mode (status: {})", response.status());
                        let (mut write, mut read) = ws_stream.split();

                        while let Some(msg) = read.next().await {
                            match msg {
                                Ok(Message::Text(text)) => {
                                    if let Ok(event) = serde_json::from_str::<Value>(&text) {
                                        let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
                                        
                                        // Handle hello message
                                        if event_type == "hello" {
                                            info!("Received Socket Mode hello message");
                                            continue;
                                        }
                                        
                                        // Acknowledge events_api messages via WebSocket
                                        // Slack Socket Mode requires acknowledgments to be sent back through the WebSocket
                                        if event_type == "events_api" {
                                            if let Some(envelope_id) = event.get("envelope_id") {
                                                let ack_msg = serde_json::json!({
                                                    "envelope_id": envelope_id.as_str().unwrap_or(""),
                                                    "payload": {}
                                                });
                                                debug!("Sending Socket Mode acknowledgment for envelope_id: {}", envelope_id.as_str().unwrap_or(""));
                                                if let Err(e) = write.send(Message::Text(ack_msg.to_string())).await {
                                                    error!("Failed to send Socket Mode acknowledgment: {}", e);
                                                }
                                            }
                                        }
                                        
                                        // Process the event
                                        if event_type == "events_api" {
                                            if let Some(payload) = event.get("payload") {
                                                if let Some(event_data) = payload.get("event") {
                                                    let inner_event_type = event_data.get("type").and_then(|v| v.as_str()).unwrap_or("");
                                                    
                                                    match inner_event_type {
                                                        "message" | "app_mention" => {
                                                            let channel = SlackChannel {
                                                                config: SlackConfig {
                                                                    enabled: true,
                                                                    bot_token: bot_token.clone(),
                                                                    app_token: app_token.clone(),
                                                                    allow_from: config_allow.clone(),
                                                                },
                                                                inbound_tx: inbound_tx.clone(),
                                                                bot_user_id: bot_user_id.clone(),
                                                                running: Arc::new(tokio::sync::Mutex::new(true)),
                                                                ws_handle: None,
                                                                seen_messages: Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new())),
                                                            };
                                                            if let Err(e) = channel.handle_message_event(event_data).await {
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
                                    break;
                                }
                                Ok(Message::Ping(_)) => {
                                    let _ = write.send(Message::Pong(vec![])).await;
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
                        warn!("Retrying Slack Socket Mode connection in 5 seconds...");
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
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
