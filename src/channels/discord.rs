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
    inbound_tx: mpsc::UnboundedSender<InboundMessage>,
    allow_list: Vec<String>,
}

#[serenity_async_trait]
impl EventHandler for Handler {
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

        let _ = self.inbound_tx.send(inbound_msg);
    }

    async fn ready(&self, _: Context, ready: Ready) {
        tracing::info!("Discord bot connected as {}", ready.user.name);
    }
}

pub struct DiscordChannel {
    config: DiscordConfig,
    inbound_tx: mpsc::UnboundedSender<InboundMessage>,
    client: Option<Client>,
    _running: Arc<tokio::sync::Mutex<bool>>,
}

impl DiscordChannel {
    pub fn new(config: DiscordConfig, inbound_tx: mpsc::UnboundedSender<InboundMessage>) -> Self {
        Self {
            config,
            inbound_tx,
            client: None,
            _running: Arc::new(tokio::sync::Mutex::new(false)),
        }
    }
}

#[async_trait]
impl BaseChannel for DiscordChannel {
    fn name(&self) -> &str {
        "discord"
    }

    async fn start(&mut self) -> Result<()> {
        tracing::info!("Initializing Discord client...");
        *self._running.lock().await = true;

        let handler = Handler {
            inbound_tx: self.inbound_tx.clone(),
            allow_list: self.config.allow_from.clone(),
        };

        tracing::info!("Connecting to Discord gateway...");
        let client = Client::builder(
            &self.config.token,
            GatewayIntents::GUILD_MESSAGES | GatewayIntents::MESSAGE_CONTENT,
        )
        .event_handler(handler)
        .await?;

        let shard_manager = client.shard_manager.clone();
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            shard_manager.shutdown_all().await;
        });

        // Start the client in a background task
        // Note: serenity Client starts automatically when created, but we need to keep it running
        // The client is already connected, we just need to store it
        self.client = Some(client);
        tracing::info!("Discord channel started successfully");
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        *self._running.lock().await = false;
        if let Some(client) = &self.client {
            client.shard_manager.shutdown_all().await;
        }
        Ok(())
    }

    async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        if msg.channel != "discord" {
            return Ok(());
        }

        if let Some(client) = &self.client {
            let channel_id = msg.chat_id.parse::<u64>()?;
            let chunks = split_message(&msg.content, 2000);

            for chunk in chunks {
                let channel_id_typed = serenity::model::id::ChannelId::new(channel_id);
                let _ = channel_id_typed.say(&client.http, &chunk).await;
            }
        }

        Ok(())
    }
}
