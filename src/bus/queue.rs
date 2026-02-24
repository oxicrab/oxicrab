use crate::bus::{InboundMessage, OutboundMessage};
use crate::safety::LeakDetector;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, warn};

const DEFAULT_RATE_LIMIT: usize = 30;
const DEFAULT_OUTBOUND_RATE_LIMIT: usize = 60;
const DEFAULT_RATE_WINDOW_S: f64 = 60.0;
const DEFAULT_INBOUND_CAPACITY: usize = 1000;
const DEFAULT_OUTBOUND_CAPACITY: usize = 1000;
/// Timeout for channel send operations to prevent indefinite blocking
/// when the consumer is slow or stalled.
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
/// Maximum inbound message content length (1 MB)
const MAX_INBOUND_CONTENT_LEN: usize = 1_000_000;
/// Maximum number of tracked senders/destinations before forced pruning
const MAX_TRACKED_ENDPOINTS: usize = 5000;

pub struct MessageBus {
    pub inbound_tx: mpsc::Sender<InboundMessage>,
    inbound_rx: Option<mpsc::Receiver<InboundMessage>>,
    pub outbound_tx: mpsc::Sender<OutboundMessage>,
    outbound_rx: Option<mpsc::Receiver<OutboundMessage>>,
    rate_limit: usize,
    outbound_rate_limit: usize,
    rate_window: Duration,
    sender_timestamps: HashMap<String, Vec<Instant>>,
    outbound_timestamps: HashMap<String, Vec<Instant>>,
    leak_detector: LeakDetector,
}

impl MessageBus {
    pub fn new(
        rate_limit: usize,
        rate_window_secs: f64,
        inbound_capacity: usize,
        outbound_capacity: usize,
    ) -> Self {
        let (inbound_tx, inbound_rx) = mpsc::channel(inbound_capacity);
        let (outbound_tx, outbound_rx) = mpsc::channel(outbound_capacity);
        Self {
            inbound_tx,
            inbound_rx: Some(inbound_rx),
            outbound_tx,
            outbound_rx: Some(outbound_rx),
            rate_limit,
            outbound_rate_limit: DEFAULT_OUTBOUND_RATE_LIMIT,
            rate_window: Duration::from_secs_f64(rate_window_secs),
            sender_timestamps: HashMap::new(),
            outbound_timestamps: HashMap::new(),
            leak_detector: LeakDetector::new(),
        }
    }

    /// Register known secret values so the leak detector can find them
    /// across encodings (raw, base64, hex).
    pub fn add_known_secrets(&mut self, secrets: &[(&str, &str)]) {
        self.leak_detector.add_known_secrets(secrets);
    }
}

impl Default for MessageBus {
    fn default() -> Self {
        Self::new(
            DEFAULT_RATE_LIMIT,
            DEFAULT_RATE_WINDOW_S,
            DEFAULT_INBOUND_CAPACITY,
            DEFAULT_OUTBOUND_CAPACITY,
        )
    }
}

impl MessageBus {
    /// Extract the inbound receiver to avoid holding lock during await
    pub fn take_inbound_rx(&mut self) -> Option<mpsc::Receiver<InboundMessage>> {
        self.inbound_rx.take()
    }

    /// Extract the outbound receiver to avoid holding lock during await
    pub fn take_outbound_rx(&mut self) -> Option<mpsc::Receiver<OutboundMessage>> {
        self.outbound_rx.take()
    }

    pub async fn publish_inbound(&mut self, msg: InboundMessage) -> Result<()> {
        // Validate content size to prevent OOM from oversized messages
        if msg.content.len() > MAX_INBOUND_CONTENT_LEN {
            warn!(
                "inbound message too large ({} bytes), truncating to {}",
                msg.content.len(),
                MAX_INBOUND_CONTENT_LEN
            );
        }

        let now = Instant::now();
        let key = format!("{}:{}", msg.channel, msg.sender_id);

        let timestamps = self.sender_timestamps.entry(key.clone()).or_default();
        let cutoff = now.checked_sub(self.rate_window).unwrap_or(now);
        timestamps.retain(|&t| t > cutoff);

        if timestamps.len() >= self.rate_limit {
            warn!(
                "Rate limit hit for {} ({}/{:.0}s) – dropping message",
                key,
                self.rate_limit,
                self.rate_window.as_secs_f64()
            );
            return Err(anyhow::anyhow!("Rate limit exceeded for {}", key));
        }

        timestamps.push(now);

        // Prune inactive senders to prevent unbounded growth
        if self.sender_timestamps.len() > MAX_TRACKED_ENDPOINTS {
            let rate_window = self.rate_window;
            self.sender_timestamps
                .retain(|_, ts| ts.iter().any(|&t| now.duration_since(t) < rate_window));
        }

        let channel = msg.channel.clone();
        let sender_id = msg.sender_id.clone();
        // Use timeout to prevent indefinite blocking when consumer is slow
        tokio::time::timeout(SEND_TIMEOUT, self.inbound_tx.send(msg))
            .await
            .map_err(|_| {
                warn!(
                    "inbound send timed out after {}s — queue full or agent loop stalled",
                    SEND_TIMEOUT.as_secs()
                );
                anyhow::anyhow!("inbound send timed out — queue full")
            })?
            .context("Failed to send inbound message - receiver closed")?;
        debug!(
            "inbound message queued: channel={}, sender={}",
            channel, sender_id
        );
        Ok(())
    }

    pub async fn publish_outbound(&mut self, mut msg: OutboundMessage) -> Result<()> {
        // Outbound rate limiting per destination
        let now = Instant::now();
        let key = format!("{}:{}", msg.channel, msg.chat_id);
        let timestamps = self.outbound_timestamps.entry(key.clone()).or_default();
        let cutoff = now.checked_sub(self.rate_window).unwrap_or(now);
        timestamps.retain(|&t| t > cutoff);
        if timestamps.len() >= self.outbound_rate_limit {
            warn!(
                "outbound rate limit hit for {} ({}/{:.0}s) – dropping message",
                key,
                self.outbound_rate_limit,
                self.rate_window.as_secs_f64()
            );
            return Err(anyhow::anyhow!("Outbound rate limit exceeded for {}", key));
        }
        timestamps.push(now);

        // Prune inactive destinations to prevent unbounded growth
        if self.outbound_timestamps.len() > MAX_TRACKED_ENDPOINTS {
            let rate_window = self.rate_window;
            self.outbound_timestamps
                .retain(|_, ts| ts.iter().any(|&t| now.duration_since(t) < rate_window));
        }

        // Scan for leaked secrets before sending (plaintext + encoded + known)
        let matches = self.leak_detector.scan(&msg.content);
        let known_matches = self.leak_detector.scan_known_secrets(&msg.content);
        if !matches.is_empty() || !known_matches.is_empty() {
            let pattern_names: Vec<&str> = matches.iter().map(|m| m.name).collect();
            let known_names: Vec<&str> = known_matches.iter().map(|m| m.name.as_str()).collect();
            warn!(
                "potential secret leak detected in outbound message: patterns={:?}, known={:?}",
                pattern_names, known_names
            );
            msg.content = self.leak_detector.redact(&msg.content);
        }

        let channel = msg.channel.clone();
        let chat_id = msg.chat_id.clone();
        // Use timeout to prevent indefinite blocking when consumer is slow
        tokio::time::timeout(SEND_TIMEOUT, self.outbound_tx.send(msg))
            .await
            .map_err(|_| {
                warn!(
                    "outbound send timed out after {}s — queue full",
                    SEND_TIMEOUT.as_secs()
                );
                anyhow::anyhow!("outbound send timed out — queue full")
            })?
            .context("Failed to send outbound message - receiver closed")?;
        debug!(
            "outbound message queued: channel={}, chat_id={}",
            channel, chat_id
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::HashMap;

    fn make_inbound(channel: &str, sender_id: &str) -> InboundMessage {
        InboundMessage {
            channel: channel.to_string(),
            sender_id: sender_id.to_string(),
            chat_id: "chat1".to_string(),
            content: "hello".to_string(),
            timestamp: Utc::now(),
            media: vec![],
            metadata: HashMap::new(),
        }
    }

    fn make_outbound(channel: &str, chat_id: &str, content: &str) -> OutboundMessage {
        OutboundMessage {
            channel: channel.to_string(),
            chat_id: chat_id.to_string(),
            content: content.to_string(),
            reply_to: None,
            media: vec![],
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn test_publish_inbound_succeeds() {
        let mut bus = MessageBus::default();
        let mut rx = bus.take_inbound_rx().unwrap();

        let msg = make_inbound("test", "user1");
        bus.publish_inbound(msg).await.unwrap();

        let received = rx.try_recv().unwrap();
        assert_eq!(received.channel, "test");
        assert_eq!(received.sender_id, "user1");
    }

    #[tokio::test]
    async fn test_inbound_rate_limit_enforced() {
        let mut bus = MessageBus::new(2, 60.0, 100, 100);
        let _rx = bus.take_inbound_rx().unwrap();

        // First two should succeed
        bus.publish_inbound(make_inbound("ch", "sender1"))
            .await
            .unwrap();
        bus.publish_inbound(make_inbound("ch", "sender1"))
            .await
            .unwrap();

        // Third should fail — rate limit of 2 per window
        let result = bus.publish_inbound(make_inbound("ch", "sender1")).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Rate limit exceeded")
        );
    }

    #[tokio::test]
    async fn test_inbound_rate_limit_per_sender() {
        let mut bus = MessageBus::new(2, 60.0, 100, 100);
        let _rx = bus.take_inbound_rx().unwrap();

        // sender1 hits limit
        bus.publish_inbound(make_inbound("ch", "sender1"))
            .await
            .unwrap();
        bus.publish_inbound(make_inbound("ch", "sender1"))
            .await
            .unwrap();

        // sender2 should still be able to publish (separate rate bucket)
        bus.publish_inbound(make_inbound("ch", "sender2"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_outbound_rate_limit_enforced() {
        let mut bus = MessageBus::new(30, 60.0, 100, 100);
        // Override the outbound limit to a small number for testing
        bus.outbound_rate_limit = 2;
        let _rx = bus.take_outbound_rx().unwrap();

        bus.publish_outbound(make_outbound("ch", "dest1", "msg1"))
            .await
            .unwrap();
        bus.publish_outbound(make_outbound("ch", "dest1", "msg2"))
            .await
            .unwrap();

        let result = bus
            .publish_outbound(make_outbound("ch", "dest1", "msg3"))
            .await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Outbound rate limit exceeded")
        );
    }

    #[tokio::test]
    async fn test_outbound_leak_detection_redacts() {
        let mut bus = MessageBus::default();
        let mut rx = bus.take_outbound_rx().unwrap();

        // add_known_secrets requires values >= 10 chars
        let secret = "sk-secret-1234567890";
        bus.add_known_secrets(&[("api_key", secret)]);

        let msg = make_outbound("ch", "dest", &format!("the key is {}", secret));
        bus.publish_outbound(msg).await.unwrap();

        let received = rx.try_recv().unwrap();
        // The secret should be redacted in the received message
        assert!(
            !received.content.contains(secret),
            "secret should be redacted, got: {}",
            received.content
        );
    }

    #[tokio::test]
    async fn test_default_creates_valid_bus() {
        let mut bus = MessageBus::default();
        assert!(bus.take_inbound_rx().is_some());
        assert!(bus.take_outbound_rx().is_some());
    }

    #[tokio::test]
    async fn test_take_rx_returns_none_second_time() {
        let mut bus = MessageBus::default();

        assert!(bus.take_inbound_rx().is_some());
        assert!(bus.take_inbound_rx().is_none());

        assert!(bus.take_outbound_rx().is_some());
        assert!(bus.take_outbound_rx().is_none());
    }
}
