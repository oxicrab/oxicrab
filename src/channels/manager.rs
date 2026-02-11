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
                let max_attempts = 3;
                let mut last_err = None;
                for attempt in 1..=max_attempts {
                    match channel.send(msg).await {
                        Ok(()) => {
                            tracing::info!("Successfully sent message to {} channel", msg.channel);
                            return Ok(());
                        }
                        Err(e) => {
                            if attempt < max_attempts {
                                tracing::warn!(
                                    "Send to {} failed (attempt {}/{}): {}, retrying...",
                                    msg.channel,
                                    attempt,
                                    max_attempts,
                                    e
                                );
                                tokio::time::sleep(tokio::time::Duration::from_secs(
                                    attempt as u64,
                                ))
                                .await;
                            }
                            last_err = Some(e);
                        }
                    }
                }
                if let Some(e) = last_err {
                    tracing::error!(
                        "Error sending message to {} channel after {} attempts: {}",
                        msg.channel,
                        max_attempts,
                        e
                    );
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

    pub async fn send_and_get_id(&self, msg: &OutboundMessage) -> Option<String> {
        for ch in self.channels.iter() {
            if ch.name() == msg.channel {
                match ch.send_and_get_id(msg).await {
                    Ok(id) => return id,
                    Err(e) => {
                        tracing::error!("Failed to send_and_get_id on {}: {}", msg.channel, e);
                        return None;
                    }
                }
            }
        }
        None
    }

    pub async fn edit_message(
        &self,
        channel: &str,
        chat_id: &str,
        message_id: &str,
        content: &str,
    ) {
        for ch in self.channels.iter() {
            if ch.name() == channel {
                if let Err(e) = ch.edit_message(chat_id, message_id, content).await {
                    tracing::debug!("edit_message failed for {}:{}: {}", channel, message_id, e);
                }
                return;
            }
        }
    }
}
