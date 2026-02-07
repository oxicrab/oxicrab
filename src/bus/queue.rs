use crate::bus::{InboundMessage, OutboundMessage};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::warn;

const DEFAULT_RATE_LIMIT: usize = 30;
const DEFAULT_RATE_WINDOW_S: f64 = 60.0;

pub struct MessageBus {
    pub inbound_tx: mpsc::UnboundedSender<InboundMessage>,
    inbound_rx: mpsc::UnboundedReceiver<InboundMessage>,
    pub outbound_tx: mpsc::UnboundedSender<OutboundMessage>,
    pub outbound_rx: mpsc::UnboundedReceiver<OutboundMessage>,
    rate_limit: usize,
    rate_window: Duration,
    sender_timestamps: HashMap<String, Vec<Instant>>,
}

impl MessageBus {
    pub fn new(rate_limit: usize, rate_window_secs: f64) -> Self {
        let (inbound_tx, inbound_rx) = mpsc::unbounded_channel();
        let (outbound_tx, outbound_rx) = mpsc::unbounded_channel();
        Self {
            inbound_tx,
            inbound_rx,
            outbound_tx,
            outbound_rx,
            rate_limit,
            rate_window: Duration::from_secs_f64(rate_window_secs),
            sender_timestamps: HashMap::new(),
        }
    }

    pub fn default() -> Self {
        Self::new(DEFAULT_RATE_LIMIT, DEFAULT_RATE_WINDOW_S)
    }

    pub async fn publish_inbound(&mut self, msg: InboundMessage) {
        let now = Instant::now();
        let key = format!("{}:{}", msg.channel, msg.sender_id);

        let timestamps = self
            .sender_timestamps
            .entry(key.clone())
            .or_insert_with(Vec::new);
        let cutoff = now - self.rate_window;
        timestamps.retain(|&t| t > cutoff);

        if timestamps.len() >= self.rate_limit {
            warn!(
                "Rate limit hit for {} ({}/{:.0}s) – dropping message",
                key,
                self.rate_limit,
                self.rate_window.as_secs_f64()
            );
            return;
        }

        timestamps.push(now);
        let _ = self.inbound_tx.send(msg);
    }

    pub async fn consume_inbound(&mut self) -> Option<InboundMessage> {
        self.inbound_rx.recv().await
    }

    pub async fn publish_outbound(&self, msg: OutboundMessage) {
        let _ = self.outbound_tx.send(msg);
    }

    #[allow(dead_code)] // May be used for alternative message consumption patterns
    pub async fn consume_outbound(&mut self) -> Option<OutboundMessage> {
        self.outbound_rx.recv().await
    }

    #[allow(dead_code)] // May be used for monitoring
    pub fn inbound_size(&self) -> usize {
        self.inbound_rx.len()
    }

    #[allow(dead_code)] // May be used for monitoring
    pub fn outbound_size(&self) -> usize {
        self.outbound_rx.len()
    }
}
