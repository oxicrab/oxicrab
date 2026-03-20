use crate::bus::{InboundMessage, OutboundMessage};
use crate::safety::LeakDetector;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
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
/// Maximum outbound message content length (1 MB)
const MAX_OUTBOUND_CONTENT_LEN: usize = 1_000_000;
/// Maximum number of tracked senders/destinations before forced pruning
const MAX_TRACKED_ENDPOINTS: usize = 5000;

/// Rate-limit state protected by a `std::sync::Mutex` (held only briefly for
/// timestamp bookkeeping, never across awaits).
struct RateLimitState {
    rate_limit: usize,
    outbound_rate_limit: usize,
    rate_window: Duration,
    sender_timestamps: HashMap<String, Vec<Instant>>,
    outbound_timestamps: HashMap<String, Vec<Instant>>,
}

pub struct MessageBus {
    pub inbound_tx: mpsc::Sender<InboundMessage>,
    inbound_rx: Mutex<Option<mpsc::Receiver<InboundMessage>>>,
    pub outbound_tx: mpsc::Sender<OutboundMessage>,
    outbound_rx: Mutex<Option<mpsc::Receiver<OutboundMessage>>>,
    rate_state: Mutex<RateLimitState>,
    leak_detector: Arc<LeakDetector>,
}

impl MessageBus {
    pub fn new(
        rate_limit: usize,
        rate_window_secs: f64,
        inbound_capacity: usize,
        outbound_capacity: usize,
    ) -> Self {
        Self::with_leak_detector(
            rate_limit,
            rate_window_secs,
            inbound_capacity,
            outbound_capacity,
            Arc::new(LeakDetector::new()),
        )
    }

    /// Create a `MessageBus` with a shared leak detector.
    ///
    /// Use this to share a single `LeakDetector` (with known secrets already
    /// registered) across the message bus, agent loop, gateway, and subagents.
    pub fn with_leak_detector(
        rate_limit: usize,
        rate_window_secs: f64,
        inbound_capacity: usize,
        outbound_capacity: usize,
        leak_detector: Arc<LeakDetector>,
    ) -> Self {
        let (inbound_tx, inbound_rx) = mpsc::channel(inbound_capacity);
        let (outbound_tx, outbound_rx) = mpsc::channel(outbound_capacity);
        Self {
            inbound_tx,
            inbound_rx: Mutex::new(Some(inbound_rx)),
            outbound_tx,
            outbound_rx: Mutex::new(Some(outbound_rx)),
            rate_state: Mutex::new(RateLimitState {
                rate_limit,
                outbound_rate_limit: DEFAULT_OUTBOUND_RATE_LIMIT,
                rate_window: Duration::from_secs_f64(rate_window_secs),
                sender_timestamps: HashMap::new(),
                outbound_timestamps: HashMap::new(),
            }),
            leak_detector,
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
    /// Extract the inbound receiver (called once at startup).
    pub fn take_inbound_rx(&self) -> Option<mpsc::Receiver<InboundMessage>> {
        self.inbound_rx.lock().ok().and_then(|mut rx| rx.take())
    }

    /// Extract the outbound receiver (called once at startup).
    pub fn take_outbound_rx(&self) -> Option<mpsc::Receiver<OutboundMessage>> {
        self.outbound_rx.lock().ok().and_then(|mut rx| rx.take())
    }

    pub async fn publish_inbound(&self, mut msg: InboundMessage) -> Result<()> {
        metrics::counter!("oxicrab_messages_received_total", "channel" => msg.channel.clone())
            .increment(1);

        // Validate content size to prevent OOM from oversized messages
        if msg.content.len() > MAX_INBOUND_CONTENT_LEN {
            warn!(
                "inbound message too large ({} bytes), truncating to {}",
                msg.content.len(),
                MAX_INBOUND_CONTENT_LEN
            );
            let mut truncate_pos = MAX_INBOUND_CONTENT_LEN;
            while truncate_pos > 0 && !msg.content.is_char_boundary(truncate_pos) {
                truncate_pos -= 1;
            }
            msg.content.truncate(truncate_pos);
        }

        // Rate-limit check (brief lock, no await inside)
        {
            let mut state = self
                .rate_state
                .lock()
                .map_err(|e| anyhow::anyhow!("rate state lock poisoned: {e}"))?;
            let now = Instant::now();
            let key = format!("{}:{}", msg.channel, msg.sender_id);
            let rate_window = state.rate_window;
            let rate_limit = state.rate_limit;

            let timestamps = state.sender_timestamps.entry(key.clone()).or_default();
            let cutoff = now.checked_sub(rate_window).unwrap_or(now);
            timestamps.retain(|&t| t > cutoff);

            if timestamps.len() >= rate_limit {
                warn!(
                    "Rate limit hit for {} ({}/{:.0}s) – dropping message",
                    key,
                    rate_limit,
                    rate_window.as_secs_f64()
                );
                return Err(anyhow::anyhow!("Rate limit exceeded for {key}"));
            }

            timestamps.push(now);

            // Prune inactive senders to prevent unbounded growth
            if state.sender_timestamps.len() > MAX_TRACKED_ENDPOINTS {
                state
                    .sender_timestamps
                    .retain(|_, ts| ts.iter().any(|&t| now.duration_since(t) < rate_window));
            }
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

    pub async fn publish_outbound(&self, mut msg: OutboundMessage) -> Result<()> {
        metrics::counter!("oxicrab_messages_sent_total", "channel" => msg.channel.clone())
            .increment(1);

        // Validate content size to prevent oversized outbound messages
        if msg.content.len() > MAX_OUTBOUND_CONTENT_LEN {
            warn!(
                "outbound message too large ({} bytes), truncating to {}",
                msg.content.len(),
                MAX_OUTBOUND_CONTENT_LEN
            );
            msg.content
                .truncate(msg.content.floor_char_boundary(MAX_OUTBOUND_CONTENT_LEN));
        }

        // Outbound rate limiting per destination (brief lock, no await inside)
        {
            let mut state = self
                .rate_state
                .lock()
                .map_err(|e| anyhow::anyhow!("rate state lock poisoned: {e}"))?;
            let now = Instant::now();
            // Allocates a String key per message; cheap relative to the mutex
            // and simpler than a composite (String, String) key type.
            let key = format!("{}:{}", msg.channel, msg.chat_id);
            let rate_window = state.rate_window;
            let outbound_rate_limit = state.outbound_rate_limit;

            let timestamps = state.outbound_timestamps.entry(key.clone()).or_default();
            let cutoff = now.checked_sub(rate_window).unwrap_or(now);
            timestamps.retain(|&t| t > cutoff);
            if timestamps.len() >= outbound_rate_limit {
                warn!(
                    "outbound rate limit hit for {} ({}/{:.0}s) – dropping message",
                    key,
                    outbound_rate_limit,
                    rate_window.as_secs_f64()
                );
                return Err(anyhow::anyhow!("Outbound rate limit exceeded for {key}"));
            }
            timestamps.push(now);

            // Prune inactive destinations to prevent unbounded growth
            if state.outbound_timestamps.len() > MAX_TRACKED_ENDPOINTS {
                state
                    .outbound_timestamps
                    .retain(|_, ts| ts.iter().any(|&t| now.duration_since(t) < rate_window));
            }
        }

        // Scan for leaked secrets before sending (plaintext + encoded + known)
        let matches = self.leak_detector.scan(&msg.content);
        let known_matches = self.leak_detector.scan_known_secrets(&msg.content);
        if !matches.is_empty() || !known_matches.is_empty() {
            let pattern_names: Vec<&str> = matches.iter().map(|m| m.name).collect();
            let known_names: Vec<&str> = known_matches.iter().map(|m| m.name.as_str()).collect();
            warn!(
                "security: potential secret leak in outbound message: patterns={:?}, known={:?}",
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
mod tests;
