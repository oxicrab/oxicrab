use crate::bus::{InboundMessage, OutboundMessage};
use crate::channels::base::{split_message, BaseChannel};
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

struct Handler {
    inbound_tx: mpsc::Sender<InboundMessage>,
    allow_set: std::collections::HashSet<String>,
    has_allow_list: bool,
}

#[serenity_async_trait]
impl EventHandler for Handler {
    async fn cache_ready(&self, _ctx: Context, _guilds: Vec<serenity::model::id::GuildId>) {
        tracing::info!("Discord cache is ready");
    }

    async fn message(&self, _ctx: Context, msg: DiscordMessage) {
        if msg.author.bot {
            return;
        }

        let sender_id = msg.author.id.to_string();

        if self.has_allow_list && !self.allow_set.contains(&sender_id) {
            return;
        }

        let inbound_msg = InboundMessage {
            channel: "discord".to_string(),
            sender_id,
            chat_id: msg.channel_id.to_string(),
            content: msg.content,
            timestamp: Utc::now(),
            media: vec![],
            metadata: HashMap::new(),
        };

        if let Err(e) = self.inbound_tx.send(inbound_msg).await {
            tracing::error!("Failed to send Discord inbound message: {}", e);
        }
    }

    async fn ready(&self, _: Context, ready: Ready) {
        tracing::info!(
            "Discord bot connected as {} (id: {})",
            ready.user.name,
            ready.user.id
        );
    }
}

pub struct DiscordChannel {
    config: DiscordConfig,
    inbound_tx: mpsc::Sender<InboundMessage>,
    _running: Arc<tokio::sync::Mutex<bool>>,
    _client_handle: Option<tokio::task::JoinHandle<()>>,
}

impl DiscordChannel {
    pub fn new(config: DiscordConfig, inbound_tx: mpsc::Sender<InboundMessage>) -> Self {
        Self {
            config,
            inbound_tx,
            _running: Arc::new(tokio::sync::Mutex::new(false)),
            _client_handle: None,
        }
    }
}

#[async_trait]
impl BaseChannel for DiscordChannel {
    fn name(&self) -> &str {
        "discord"
    }

    async fn start(&mut self) -> Result<()> {
        if self.config.token.is_empty() {
            return Err(anyhow::anyhow!("Discord token is empty"));
        }

        tracing::info!("Initializing Discord client...");
        *self._running.lock().await = true;

        let allow_set: std::collections::HashSet<String> = self
            .config
            .allow_from
            .iter()
            .map(|a| a.trim_start_matches('+').to_string())
            .collect();
        let handler = Handler {
            inbound_tx: self.inbound_tx.clone(),
            has_allow_list: !self.config.allow_from.is_empty(),
            allow_set,
        };

        tracing::info!("Connecting to Discord gateway...");
        let mut client = Client::builder(
            &self.config.token,
            GatewayIntents::GUILD_MESSAGES | GatewayIntents::MESSAGE_CONTENT,
        )
        .event_handler(handler)
        .await
        .map_err(|e| {
            tracing::error!("Failed to create Discord client: {}", e);
            anyhow::anyhow!("Failed to create Discord client: {}", e)
        })?;

        let shard_manager = client.shard_manager.clone();
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            shard_manager.shutdown_all().await;
        });

        // Start the client - this actually connects to Discord
        // Keep the client alive by running start() in a spawned task
        let shard_manager_for_error = client.shard_manager.clone();
        let handle = tokio::spawn(async move {
            if let Err(why) = client.start().await {
                tracing::error!("Discord client connection error: {:?}", why);
                shard_manager_for_error.shutdown_all().await;
            }
        });

        self._client_handle = Some(handle);

        tracing::info!(
            "Discord channel started successfully - connection will be established in background"
        );
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        *self._running.lock().await = false;
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
        let channel_id = serenity::model::id::ChannelId::new(id_val);
        let sent = channel_id.say(&http, &msg.content).await?;

        Ok(Some(sent.id.to_string()))
    }

    async fn edit_message(&self, chat_id: &str, message_id: &str, new_content: &str) -> Result<()> {
        let channel_id = chat_id
            .parse::<u64>()
            .map_err(|e| anyhow::anyhow!("Invalid Discord channel_id: {}", e))?;
        let msg_id = message_id
            .parse::<u64>()
            .map_err(|e| anyhow::anyhow!("Invalid Discord message_id: {}", e))?;

        let http = serenity::http::Http::new(&self.config.token);
        let channel_id = serenity::model::id::ChannelId::new(channel_id);
        let msg_id = serenity::model::id::MessageId::new(msg_id);

        channel_id
            .edit_message(
                &http,
                msg_id,
                serenity::builder::EditMessage::new().content(new_content),
            )
            .await?;

        Ok(())
    }
}
