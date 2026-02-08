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
    allow_list: Vec<String>,
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
        let normalized: std::collections::HashSet<String> = self
            .allow_list
            .iter()
            .map(|a| a.trim_start_matches('+').to_string())
            .collect();

        if !self.allow_list.is_empty() && !normalized.contains(&sender_id) {
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

        let handler = Handler {
            inbound_tx: self.inbound_tx.clone(),
            allow_list: self.config.allow_from.clone(),
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

        // For sending messages, we need the HTTP client
        // Since we can't easily access it from shard_manager, we'll create a temporary HTTP client
        // This is a limitation - ideally we'd keep the client alive, but serenity's API makes this difficult
        let channel_id = msg.chat_id.parse::<u64>()?;
        let chunks = split_message(&msg.content, 2000);
        let http = serenity::http::Http::new(&self.config.token);

        for chunk in chunks {
            let channel_id_typed = serenity::model::id::ChannelId::new(channel_id);
            if let Err(e) = channel_id_typed.say(&http, &chunk).await {
                tracing::error!("Failed to send Discord message: {}", e);
            }
        }

        Ok(())
    }
}
