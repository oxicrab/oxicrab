use crate::bus::{InboundMessage, OutboundMessage};
use crate::channels::base::{split_message, BaseChannel};
use crate::channels::utils::exponential_backoff_delay;
use crate::config::DiscordConfig;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use serenity::async_trait as serenity_async_trait;
use serenity::model::channel::Message as DiscordMessage;
use serenity::model::gateway::{GatewayIntents, Ready};
use serenity::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

struct Handler {
    inbound_tx: mpsc::Sender<InboundMessage>,
    allow_set: std::collections::HashSet<String>,
    has_allow_list: bool,
    http_client: reqwest::Client,
}

#[serenity_async_trait]
impl EventHandler for Handler {
    async fn cache_ready(&self, _ctx: Context, _guilds: Vec<serenity::model::id::GuildId>) {
        info!("Discord cache is ready");
    }

    async fn message(&self, _ctx: Context, msg: DiscordMessage) {
        if msg.author.bot {
            return;
        }

        let sender_id = msg.author.id.to_string();

        if self.has_allow_list && !self.allow_set.contains(&sender_id) {
            return;
        }

        // Download image attachments
        let mut media_paths = Vec::new();
        let mut content = msg.content.clone();
        for attachment in &msg.attachments {
            let content_type = attachment.content_type.as_deref().unwrap_or("");
            let is_image = content_type.starts_with("image/");
            let is_audio = content_type.starts_with("audio/");
            if !is_image && !is_audio {
                continue;
            }
            let (ext, tag) = if is_image {
                (
                    match content_type {
                        "image/jpeg" => "jpg",
                        "image/png" => "png",
                        "image/gif" => "gif",
                        "image/webp" => "webp",
                        _ => "bin",
                    },
                    "image",
                )
            } else {
                (
                    match content_type {
                        "audio/mpeg" => "mp3",
                        "audio/wav" => "wav",
                        "audio/webm" => "webm",
                        "audio/mp4" => "m4a",
                        _ => "ogg",
                    },
                    "audio",
                )
            };
            let media_dir = dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".oxicrab")
                .join("media");
            if let Err(e) = std::fs::create_dir_all(&media_dir) {
                warn!("Failed to create media directory: {}", e);
            }
            let file_path = media_dir.join(format!("discord_{}.{}", attachment.id, ext));

            match self.http_client.get(&attachment.url).send().await {
                Ok(resp) => match resp.bytes().await {
                    Ok(bytes) => {
                        if let Err(e) = std::fs::write(&file_path, &bytes) {
                            warn!("Failed to write Discord media file: {}", e);
                        }
                        let path_str = file_path.to_string_lossy().to_string();
                        media_paths.push(path_str.clone());
                        content = format!("{}\n[{}: {}]", content, tag, path_str);
                    }
                    Err(e) => warn!("Failed to download Discord attachment: {}", e),
                },
                Err(e) => warn!("Failed to download Discord attachment: {}", e),
            }
        }

        let inbound_msg = InboundMessage {
            channel: "discord".to_string(),
            sender_id,
            chat_id: msg.channel_id.to_string(),
            content,
            timestamp: Utc::now(),
            media: media_paths,
            metadata: HashMap::new(),
        };

        if let Err(e) = self.inbound_tx.send(inbound_msg).await {
            error!("Failed to send Discord inbound message: {}", e);
        }
    }

    async fn ready(&self, _: Context, ready: Ready) {
        info!(
            "Discord bot connected as {} (id: {})",
            ready.user.name, ready.user.id
        );
    }
}

pub struct DiscordChannel {
    config: DiscordConfig,
    inbound_tx: mpsc::Sender<InboundMessage>,
    running: Arc<tokio::sync::Mutex<bool>>,
    _client_handle: Option<tokio::task::JoinHandle<()>>,
}

impl DiscordChannel {
    pub fn new(config: DiscordConfig, inbound_tx: mpsc::Sender<InboundMessage>) -> Self {
        Self {
            config,
            inbound_tx,
            running: Arc::new(tokio::sync::Mutex::new(false)),
            _client_handle: None,
        }
    }
}

#[async_trait]
impl BaseChannel for DiscordChannel {
    fn name(&self) -> &'static str {
        "discord"
    }

    async fn start(&mut self) -> Result<()> {
        if self.config.token.is_empty() {
            return Err(anyhow::anyhow!("Discord token is empty"));
        }

        info!("Initializing Discord client...");
        *self.running.lock().await = true;

        let token = self.config.token.clone();
        let allow_from = self.config.allow_from.clone();
        let inbound_tx = self.inbound_tx.clone();
        let running = self.running.clone();

        let handle = tokio::spawn(async move {
            let mut reconnect_attempt = 0u32;
            loop {
                if !*running.lock().await {
                    info!("Discord channel stopped, exiting retry loop");
                    break;
                }

                let allow_set: std::collections::HashSet<String> = allow_from
                    .iter()
                    .map(|a| a.trim_start_matches('+').to_string())
                    .collect();
                let handler = Handler {
                    inbound_tx: inbound_tx.clone(),
                    has_allow_list: !allow_from.is_empty(),
                    allow_set,
                    http_client: reqwest::Client::builder()
                        .connect_timeout(std::time::Duration::from_secs(10))
                        .timeout(std::time::Duration::from_secs(30))
                        .build()
                        .unwrap_or_else(|_| reqwest::Client::new()),
                };

                info!("Connecting to Discord gateway...");
                match Client::builder(
                    &token,
                    GatewayIntents::GUILD_MESSAGES | GatewayIntents::MESSAGE_CONTENT,
                )
                .event_handler(handler)
                .await
                {
                    Ok(mut client) => {
                        reconnect_attempt = 0; // Reset on successful client creation
                        if let Err(why) = client.start().await {
                            error!("Discord client connection error: {:?}", why);
                        }
                    }
                    Err(e) => {
                        error!("Failed to create Discord client: {}", e);
                    }
                }

                // Check if we should reconnect
                if !*running.lock().await {
                    break;
                }

                let delay = exponential_backoff_delay(reconnect_attempt, 5, 60);
                reconnect_attempt += 1;
                warn!(
                    "Discord client exited, reconnecting in {} seconds...",
                    delay
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
            }
        });

        self._client_handle = Some(handle);

        info!(
            "Discord channel started successfully - connection will be established in background"
        );
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        *self.running.lock().await = false;
        // Client will be dropped when handle completes
        if let Some(handle) = self._client_handle.take() {
            handle.abort();
        }
        Ok(())
    }

    async fn send_typing(&self, chat_id: &str) -> Result<()> {
        let channel_id = chat_id
            .parse::<u64>()
            .map_err(|e| anyhow::anyhow!("Invalid Discord channel_id: {}", e))?;
        let http = serenity::http::Http::new(&self.config.token);
        let channel_id_typed = serenity::model::id::ChannelId::new(channel_id);
        channel_id_typed.broadcast_typing(&http).await?;
        Ok(())
    }

    async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        if msg.channel != "discord" {
            return Ok(());
        }

        let id_val = msg.chat_id.parse::<u64>()?;
        let chunks = split_message(&msg.content, 2000);
        let http = serenity::http::Http::new(&self.config.token);

        // Check if chat_id is a user ID (from allow_from) — if so, open a DM channel
        let is_user_id = self
            .config
            .allow_from
            .iter()
            .any(|a| a.trim_start_matches('+') == msg.chat_id);

        let target_channel_id = if is_user_id {
            let user_id = serenity::model::id::UserId::new(id_val);
            let dm_channel = user_id.create_dm_channel(&http).await?;
            dm_channel.id
        } else {
            serenity::model::id::ChannelId::new(id_val)
        };

        // Send media attachments first
        for path in &msg.media {
            let file_path = std::path::Path::new(path);
            if !file_path.exists() {
                warn!("discord: media file not found: {}", path);
                continue;
            }
            match serenity::builder::CreateAttachment::path(file_path).await {
                Ok(attachment) => {
                    let builder = serenity::builder::CreateMessage::new().add_file(attachment);
                    if let Err(e) = target_channel_id.send_message(&http, builder).await {
                        warn!("discord: failed to send attachment {}: {}", path, e);
                    }
                }
                Err(e) => {
                    warn!("discord: failed to read attachment {}: {}", path, e);
                }
            }
        }

        // Send text content
        for chunk in chunks {
            target_channel_id
                .say(&http, &chunk)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to send Discord message: {}", e))?;
        }

        Ok(())
    }

    async fn send_and_get_id(&self, msg: &OutboundMessage) -> Result<Option<String>> {
        if msg.channel != "discord" {
            return Ok(None);
        }
        let id_val = msg.chat_id.parse::<u64>()?;
        let http = serenity::http::Http::new(&self.config.token);
        let target = serenity::model::id::ChannelId::new(id_val);
        let sent = target
            .say(&http, &msg.content)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send Discord message: {}", e))?;
        Ok(Some(sent.id.to_string()))
    }

    async fn edit_message(&self, chat_id: &str, message_id: &str, content: &str) -> Result<()> {
        let channel_id = chat_id.parse::<u64>()?;
        let msg_id = message_id.parse::<u64>()?;
        let http = serenity::http::Http::new(&self.config.token);
        let channel = serenity::model::id::ChannelId::new(channel_id);
        let builder = serenity::builder::EditMessage::new().content(content);
        channel
            .edit_message(&http, serenity::model::id::MessageId::new(msg_id), builder)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to edit Discord message: {}", e))?;
        Ok(())
    }

    async fn delete_message(&self, chat_id: &str, message_id: &str) -> Result<()> {
        let channel_id = chat_id.parse::<u64>()?;
        let msg_id = message_id.parse::<u64>()?;
        let http = serenity::http::Http::new(&self.config.token);
        let channel = serenity::model::id::ChannelId::new(channel_id);
        channel
            .delete_message(&http, serenity::model::id::MessageId::new(msg_id))
            .await
            .map_err(|e| anyhow::anyhow!("Failed to delete Discord message: {}", e))?;
        Ok(())
    }
}
