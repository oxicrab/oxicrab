use crate::bus::{InboundMessage, OutboundMessage};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::warn;

const DEFAULT_RATE_LIMIT: usize = 30;
const DEFAULT_RATE_WINDOW_S: f64 = 60.0;
const DEFAULT_INBOUND_CAPACITY: usize = 1000;
const DEFAULT_OUTBOUND_CAPACITY: usize = 1000;

pub struct MessageBus {
    pub inbound_tx: mpsc::Sender<InboundMessage>,
    inbound_rx: Option<mpsc::Receiver<InboundMessage>>,
    pub outbound_tx: mpsc::Sender<OutboundMessage>,
    outbound_rx: Option<mpsc::Receiver<OutboundMessage>>,
    rate_limit: usize,
    rate_window: Duration,
    sender_timestamps: HashMap<String, Vec<Instant>>,
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
            rate_window: Duration::from_secs_f64(rate_window_secs),
            sender_timestamps: HashMap::new(),
        }
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
        let now = Instant::now();
        let key = format!("{}:{}", msg.channel, msg.sender_id);

        let timestamps = self.sender_timestamps.entry(key.clone()).or_default();
        let cutoff = now.checked_sub(self.rate_window).unwrap();
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
        self.inbound_tx
            .send(msg)
            .await
            .context("Failed to send inbound message - receiver closed")?;
        Ok(())
    }

    pub async fn publish_outbound(&self, msg: OutboundMessage) -> Result<()> {
        self.outbound_tx
            .send(msg)
            .await
            .context("Failed to send outbound message - receiver closed")?;
        Ok(())
    }
}
