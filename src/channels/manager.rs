use crate::bus::{InboundMessage, OutboundMessage};
use crate::channels::base::BaseChannel;
#[cfg(feature = "channel-discord")]
use crate::channels::discord::DiscordChannel;
#[cfg(feature = "channel-slack")]
use crate::channels::slack::SlackChannel;
#[cfg(feature = "channel-telegram")]
use crate::channels::telegram::TelegramChannel;
#[cfg(feature = "channel-whatsapp")]
use crate::channels::whatsapp::WhatsAppChannel;
use crate::config::Config;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

pub struct ChannelManager {
    channels: Vec<Box<dyn BaseChannel>>,
    enabled_channels: Vec<String>,
}

impl ChannelManager {
    pub fn new(config: Config, inbound_tx: Arc<mpsc::Sender<InboundMessage>>) -> Self {
        let mut channels: Vec<Box<dyn BaseChannel>> = Vec::new();
        let mut enabled = Vec::new();

        #[cfg(feature = "channel-telegram")]
        if config.channels.telegram.enabled && !config.channels.telegram.token.is_empty() {
            debug!("Initializing Telegram channel...");
            channels.push(Box::new(TelegramChannel::new(
                config.channels.telegram.clone(),
                (*inbound_tx).clone(),
            )));
            enabled.push("telegram".to_string());
            info!("Telegram channel enabled");
        }
        #[cfg(not(feature = "channel-telegram"))]
        if config.channels.telegram.enabled {
            warn!("Telegram is enabled in config but not compiled (missing 'channel-telegram' feature)");
        }

        #[cfg(feature = "channel-discord")]
        if config.channels.discord.enabled && !config.channels.discord.token.is_empty() {
            debug!("Initializing Discord channel...");
            channels.push(Box::new(DiscordChannel::new(
                config.channels.discord.clone(),
                (*inbound_tx).clone(),
            )));
            enabled.push("discord".to_string());
            info!("Discord channel enabled");
        }
        #[cfg(not(feature = "channel-discord"))]
        if config.channels.discord.enabled {
            warn!(
                "Discord is enabled in config but not compiled (missing 'channel-discord' feature)"
            );
        }

        #[cfg(feature = "channel-slack")]
        if config.channels.slack.enabled && !config.channels.slack.bot_token.is_empty() {
            debug!("Initializing Slack channel...");
            channels.push(Box::new(SlackChannel::new(
                config.channels.slack.clone(),
                inbound_tx.clone(),
            )));
            enabled.push("slack".to_string());
            info!("Slack channel enabled");
        }
        #[cfg(not(feature = "channel-slack"))]
        if config.channels.slack.enabled {
            warn!("Slack is enabled in config but not compiled (missing 'channel-slack' feature)");
        }

        #[cfg(feature = "channel-whatsapp")]
        if config.channels.whatsapp.enabled {
            debug!("Initializing WhatsApp channel...");
            channels.push(Box::new(WhatsAppChannel::new(
                config.channels.whatsapp.clone(),
                inbound_tx.clone(),
            )));
            enabled.push("whatsapp".to_string());
            info!("WhatsApp channel enabled");
        }
        #[cfg(not(feature = "channel-whatsapp"))]
        if config.channels.whatsapp.enabled {
            warn!("WhatsApp is enabled in config but not compiled (missing 'channel-whatsapp' feature)");
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
            info!("Starting channel: {}", channel_name);
            if let Err(e) = channel.start().await {
                error!("Failed to start channel {}: {}", channel_name, e);
                return Err(anyhow::anyhow!(
                    "Failed to start channel {}: {}",
                    channel_name,
                    e
                ));
            }
            info!("Channel {} started successfully", channel_name);
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
        info!(
            "ChannelManager.send: channel={}, chat_id={}, content_len={}",
            msg.channel,
            msg.chat_id,
            msg.content.len()
        );
        for channel in self.channels.iter() {
            if channel.name() == msg.channel {
                info!("Found matching channel: {}", channel.name());
                let max_attempts = 3;
                let mut last_err = None;
                for attempt in 1..=max_attempts {
                    match channel.send(msg).await {
                        Ok(()) => {
                            info!("Successfully sent message to {} channel", msg.channel);
                            return Ok(());
                        }
                        Err(e) => {
                            if attempt < max_attempts {
                                warn!(
                                    "Send to {} failed (attempt {}/{}): {}, retrying...",
                                    msg.channel, attempt, max_attempts, e
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
                    return Err(anyhow::anyhow!(
                        "Failed to send message to {} channel after {} attempts: {}",
                        msg.channel,
                        max_attempts,
                        e
                    ));
                }
                return Ok(());
            }
        }
        Err(anyhow::anyhow!(
            "No channel found for: {} (available channels: {:?})",
            msg.channel,
            self.channels.iter().map(|c| c.name()).collect::<Vec<_>>()
        ))
    }

    pub async fn send_typing(&self, channel: &str, chat_id: &str) {
        for ch in self.channels.iter() {
            if ch.name() == channel {
                if let Err(e) = ch.send_typing(chat_id).await {
                    debug!("Typing indicator failed for {}: {}", channel, e);
                }
                return;
            }
        }
    }

    pub async fn send_and_get_id(&self, msg: &OutboundMessage) -> Result<Option<String>> {
        for channel in self.channels.iter() {
            if channel.name() == msg.channel {
                return channel.send_and_get_id(msg).await;
            }
        }
        Ok(None)
    }

    pub async fn edit_message(
        &self,
        channel: &str,
        chat_id: &str,
        message_id: &str,
        content: &str,
    ) -> Result<()> {
        for ch in self.channels.iter() {
            if ch.name() == channel {
                return ch.edit_message(chat_id, message_id, content).await;
            }
        }
        Ok(())
    }

    pub async fn delete_message(
        &self,
        channel: &str,
        chat_id: &str,
        message_id: &str,
    ) -> Result<()> {
        for ch in self.channels.iter() {
            if ch.name() == channel {
                return ch.delete_message(chat_id, message_id).await;
            }
        }
        Ok(())
    }
}
