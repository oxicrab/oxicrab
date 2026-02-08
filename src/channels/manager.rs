use crate::bus::{InboundMessage, OutboundMessage};
use crate::channels::base::BaseChannel;
use crate::channels::{
    discord::DiscordChannel, slack::SlackChannel, telegram::TelegramChannel,
    whatsapp::WhatsAppChannel,
};
use crate::config::Config;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct ChannelManager {
    channels: Vec<Box<dyn BaseChannel>>,
    enabled_channels: Vec<String>,
}

impl ChannelManager {
    pub fn new(config: Config, inbound_tx: Arc<mpsc::Sender<InboundMessage>>) -> Self {
        let mut channels: Vec<Box<dyn BaseChannel>> = Vec::new();
        let mut enabled = Vec::new();

        // For WhatsApp, create a separate channel for outbound messages
        let (_, outbound_rx) = tokio::sync::mpsc::channel::<OutboundMessage>(1000);

        if config.channels.telegram.enabled && !config.channels.telegram.token.is_empty() {
            tracing::debug!("Initializing Telegram channel...");
            channels.push(Box::new(TelegramChannel::new(
                config.channels.telegram.clone(),
                (*inbound_tx).clone(),
            )));
            enabled.push("telegram".to_string());
            tracing::info!("Telegram channel enabled");
        }

        if config.channels.discord.enabled && !config.channels.discord.token.is_empty() {
            tracing::debug!("Initializing Discord channel...");
            channels.push(Box::new(DiscordChannel::new(
                config.channels.discord.clone(),
                (*inbound_tx).clone(),
            )));
            enabled.push("discord".to_string());
            tracing::info!("Discord channel enabled");
        }

        if config.channels.slack.enabled && !config.channels.slack.bot_token.is_empty() {
            tracing::debug!("Initializing Slack channel...");
            channels.push(Box::new(SlackChannel::new(
                config.channels.slack.clone(),
                inbound_tx.clone(),
            )));
            enabled.push("slack".to_string());
            tracing::info!("Slack channel enabled");
        }

        if config.channels.whatsapp.enabled {
            tracing::debug!("Initializing WhatsApp channel...");
            channels.push(Box::new(WhatsAppChannel::new(
                config.channels.whatsapp.clone(),
                inbound_tx.clone(),
                outbound_rx,
            )));
            enabled.push("whatsapp".to_string());
            tracing::info!("WhatsApp channel enabled");
        }

        Self {
            channels,
            enabled_channels: enabled,
        }
    }

    pub fn enabled_channels(&self) -> &[String] {
        &self.enabled_channels
    }

    pub async fn start_all(&mut self) -> Result<()> {
        for (idx, channel) in self.channels.iter_mut().enumerate() {
            let channel_name = self
                .enabled_channels
                .get(idx)
                .map(|s| s.as_str())
                .unwrap_or("unknown");
            tracing::info!("Starting channel: {}", channel_name);
            if let Err(e) = channel.start().await {
                tracing::error!("Failed to start channel {}: {}", channel_name, e);
                return Err(anyhow::anyhow!(
                    "Failed to start channel {}: {}",
                    channel_name,
                    e
                ));
            }
            tracing::info!("Channel {} started successfully", channel_name);
        }
        Ok(())
    }

    pub async fn stop_all(&mut self) -> Result<()> {
        for channel in self.channels.iter_mut() {
            channel.stop().await?;
        }
        Ok(())
    }

    pub async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        tracing::info!(
            "ChannelManager.send: channel={}, chat_id={}, content_len={}",
            msg.channel,
            msg.chat_id,
            msg.content.len()
        );
        for channel in self.channels.iter() {
            if channel.name() == msg.channel {
                tracing::info!("Found matching channel: {}", channel.name());
                if let Err(e) = channel.send(msg).await {
                    tracing::error!("Error sending message to {} channel: {}", msg.channel, e);
                } else {
                    tracing::info!("Successfully sent message to {} channel", msg.channel);
                }
                return Ok(());
            }
        }
        tracing::error!(
            "No channel found for: {} (available channels: {:?})",
            msg.channel,
            self.channels.iter().map(|c| c.name()).collect::<Vec<_>>()
        );
        Ok(())
    }

    pub async fn send_typing(&self, channel: &str, chat_id: &str) {
        for ch in self.channels.iter() {
            if ch.name() == channel {
                if let Err(e) = ch.send_typing(chat_id).await {
                    tracing::debug!("Typing indicator failed for {}: {}", channel, e);
                }
                return;
            }
        }
    }
}
