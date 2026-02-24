use crate::bus::{InboundMessage, OutboundMessage};
use crate::channels::base::BaseChannel;
#[cfg(feature = "channel-discord")]
use crate::channels::discord::DiscordChannel;
#[cfg(feature = "channel-slack")]
use crate::channels::slack::SlackChannel;
#[cfg(feature = "channel-telegram")]
use crate::channels::telegram::TelegramChannel;
#[cfg(feature = "channel-twilio")]
use crate::channels::twilio::TwilioChannel;
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
    #[allow(clippy::needless_pass_by_value)]
    // Arc is designed to be passed by value
    // Variables used conditionally inside #[cfg(feature)] blocks
    #[allow(unused_variables, unused_mut)]
    pub fn new(config: &Config, inbound_tx: Arc<mpsc::Sender<InboundMessage>>) -> Self {
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
            warn!(
                "Telegram is enabled in config but not compiled (missing 'channel-telegram' feature)"
            );
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
            warn!(
                "WhatsApp is enabled in config but not compiled (missing 'channel-whatsapp' feature)"
            );
        }

        #[cfg(feature = "channel-twilio")]
        if config.channels.twilio.enabled
            && !config.channels.twilio.account_sid.is_empty()
            && !config.channels.twilio.auth_token.is_empty()
        {
            debug!("Initializing Twilio channel...");
            channels.push(Box::new(TwilioChannel::new(
                config.channels.twilio.clone(),
                inbound_tx.clone(),
            )));
            enabled.push("twilio".to_string());
            info!("Twilio channel enabled");
        }
        #[cfg(not(feature = "channel-twilio"))]
        if config.channels.twilio.enabled {
            warn!(
                "Twilio is enabled in config but not compiled (missing 'channel-twilio' feature)"
            );
        }

        Self {
            channels,
            enabled_channels: enabled,
        }
    }

    #[cfg(test)]
    fn with_channels(channels: Vec<Box<dyn BaseChannel>>) -> Self {
        let enabled = channels.iter().map(|c| c.name().to_string()).collect();
        Self {
            channels,
            enabled_channels: enabled,
        }
    }

    pub fn enabled_channels(&self) -> &[String] {
        &self.enabled_channels
    }

    pub async fn start_all(&mut self) -> Result<()> {
        let channel_count = self.channels.len();
        let mut handles = Vec::with_capacity(channel_count);
        let channels: Vec<Box<dyn BaseChannel>> = std::mem::take(&mut self.channels);

        for (idx, mut channel) in channels.into_iter().enumerate() {
            let name = self
                .enabled_channels
                .get(idx)
                .cloned()
                .unwrap_or_else(|| "unknown".to_string());
            handles.push(tokio::spawn(async move {
                info!("starting channel: {}", name);
                let result = channel.start().await;
                (name, channel, result)
            }));
        }

        let mut started = Vec::with_capacity(channel_count);
        let mut failed = None;
        for handle in handles {
            match handle.await {
                Ok((name, channel, result)) => match result {
                    Ok(()) => {
                        info!("channel {} started successfully", name);
                        started.push(channel);
                    }
                    Err(e) => {
                        error!("failed to start channel {}: {}", name, e);
                        failed = Some((name, e));
                        // Don't break — let other handles complete to avoid orphaned tasks
                    }
                },
                Err(e) => {
                    error!("channel start task panicked: {}", e);
                    failed = Some((
                        "unknown".to_string(),
                        anyhow::anyhow!("task panicked: {}", e),
                    ));
                    // Continue awaiting remaining handles
                }
            }
        }

        if let Some((name, e)) = failed {
            // Rollback: stop channels that started
            for ch in &mut started {
                if let Err(stop_err) = ch.stop().await {
                    warn!("error stopping {} during cleanup: {}", ch.name(), stop_err);
                }
            }
            self.channels = started;
            return Err(anyhow::anyhow!("failed to start channel {}: {}", name, e));
        }

        self.channels = started;
        Ok(())
    }

    pub async fn stop_all(&mut self) -> Result<()> {
        for channel in &mut self.channels {
            if let Err(e) = channel.stop().await {
                tracing::warn!("error stopping channel {}: {}", channel.name(), e);
            }
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
        for channel in &self.channels {
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
        for ch in &self.channels {
            if ch.name() == channel {
                if let Err(e) = ch.send_typing(chat_id).await {
                    debug!("Typing indicator failed for {}: {}", channel, e);
                }
                return;
            }
        }
    }

    pub async fn send_and_get_id(&self, msg: &OutboundMessage) -> Result<Option<String>> {
        for channel in &self.channels {
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
        for ch in &self.channels {
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
        for ch in &self.channels {
            if ch.name() == channel {
                return ch.delete_message(chat_id, message_id).await;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Mock channel with configurable failure behavior.
    /// `fail_count` controls how many times `send()` fails before succeeding.
    /// Set to `usize::MAX` for a channel that always fails.
    struct MockChannel {
        channel_name: String,
        fail_count: Arc<AtomicUsize>,
        send_attempts: Arc<AtomicUsize>,
    }

    impl MockChannel {
        fn new(name: &str, fail_count: usize) -> Self {
            Self {
                channel_name: name.to_string(),
                fail_count: Arc::new(AtomicUsize::new(fail_count)),
                send_attempts: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    #[async_trait]
    impl BaseChannel for MockChannel {
        fn name(&self) -> &str {
            &self.channel_name
        }

        async fn start(&mut self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn stop(&mut self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn send(&self, _msg: &OutboundMessage) -> anyhow::Result<()> {
            let attempt = self.send_attempts.fetch_add(1, Ordering::SeqCst);
            if attempt < self.fail_count.load(Ordering::SeqCst) {
                Err(anyhow::anyhow!(
                    "mock send failure (attempt {})",
                    attempt + 1
                ))
            } else {
                Ok(())
            }
        }
    }

    fn make_outbound(channel: &str) -> OutboundMessage {
        OutboundMessage {
            channel: channel.to_string(),
            chat_id: "chat1".to_string(),
            content: "hello".to_string(),
            reply_to: None,
            media: vec![],
            metadata: std::collections::HashMap::new(),
        }
    }

    #[tokio::test]
    async fn test_send_no_matching_channel() {
        let mgr = ChannelManager::with_channels(vec![]);
        let result = mgr.send(&make_outbound("nonexistent")).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No channel found"));
    }

    #[tokio::test]
    async fn test_send_matching_channel_succeeds() {
        let mgr = ChannelManager::with_channels(vec![Box::new(MockChannel::new("test", 0))]);
        let result = mgr.send(&make_outbound("test")).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_retries_on_failure() {
        // Fail first 2 attempts, succeed on 3rd
        let mgr = ChannelManager::with_channels(vec![Box::new(MockChannel::new("test", 2))]);
        let result = mgr.send(&make_outbound("test")).await;
        assert!(result.is_ok(), "should succeed after retries");
    }

    #[tokio::test]
    async fn test_send_exhausts_retries() {
        // Always fail (fail_count > max_attempts=3)
        let mgr =
            ChannelManager::with_channels(vec![Box::new(MockChannel::new("test", usize::MAX))]);
        let result = mgr.send(&make_outbound("test")).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("after 3 attempts"));
    }

    #[tokio::test]
    async fn test_enabled_channels_empty_by_default() {
        let mgr = ChannelManager::with_channels(vec![]);
        assert!(mgr.enabled_channels().is_empty());
    }

    #[tokio::test]
    async fn test_send_typing_no_channel_does_not_panic() {
        let mgr = ChannelManager::with_channels(vec![]);
        // Should return silently, not panic
        mgr.send_typing("nonexistent", "chat1").await;
    }
}
