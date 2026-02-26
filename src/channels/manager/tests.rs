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
    let mgr = ChannelManager::with_channels(vec![Box::new(MockChannel::new("test", usize::MAX))]);
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
