use crate::bus::{InboundMessage, OutboundMessage};
use crate::safety::LeakDetector;
use anyhow::{Context, Result};
use governor::clock::DefaultClock;
use governor::state::keyed::DefaultKeyedStateStore;
use governor::{Quota, RateLimiter};
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};
use std::time::Duration;
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

type KeyedLimiter = RateLimiter<String, DefaultKeyedStateStore<String>, DefaultClock>;

/// Build a keyed rate limiter: `burst` requests per `window`.
fn build_keyed_limiter(burst: usize, window: Duration) -> KeyedLimiter {
    let burst = NonZeroU32::new(burst.max(1) as u32).expect("burst must be > 0");
    let quota = Quota::with_period(window / burst.get())
        .expect("valid quota period")
        .allow_burst(burst);
    RateLimiter::keyed(quota)
}

/// Recursively redact secrets from a metadata value tree. Strings
/// run through the leak-detector's pattern scan; objects and arrays
/// recurse with a depth bound so a pathologically nested metadata
/// payload can't blow the stack.
fn redact_metadata_value(value: &mut serde_json::Value, detector: &crate::safety::LeakDetector) {
    fn walk(value: &mut serde_json::Value, detector: &crate::safety::LeakDetector, depth: usize) {
        if depth >= 32 {
            return;
        }
        match value {
            serde_json::Value::String(s) => {
                let redacted = detector.redact(s);
                if redacted != *s {
                    warn!("security: secret leak in outbound metadata — redacting");
                    *s = redacted;
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    walk(item, detector, depth + 1);
                }
            }
            serde_json::Value::Object(map) => {
                for v in map.values_mut() {
                    walk(v, detector, depth + 1);
                }
            }
            _ => {}
        }
    }
    walk(value, detector, 0);
}

pub struct MessageBus {
    pub inbound_tx: mpsc::Sender<InboundMessage>,
    inbound_rx: Mutex<Option<mpsc::Receiver<InboundMessage>>>,
    pub outbound_tx: mpsc::Sender<OutboundMessage>,
    outbound_rx: Mutex<Option<mpsc::Receiver<OutboundMessage>>>,
    inbound_limiter: KeyedLimiter,
    outbound_limiter: KeyedLimiter,
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
        let window = Duration::from_secs_f64(rate_window_secs);
        let (inbound_tx, inbound_rx) = mpsc::channel(inbound_capacity);
        let (outbound_tx, outbound_rx) = mpsc::channel(outbound_capacity);
        Self {
            inbound_tx,
            inbound_rx: Mutex::new(Some(inbound_rx)),
            outbound_tx,
            outbound_rx: Mutex::new(Some(outbound_rx)),
            inbound_limiter: build_keyed_limiter(rate_limit, window),
            outbound_limiter: build_keyed_limiter(DEFAULT_OUTBOUND_RATE_LIMIT, window),
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

        // Rate-limit check
        let key = format!("{}:{}", msg.channel, msg.sender_id);
        if self.inbound_limiter.check_key(&key).is_err() {
            warn!("rate limit hit for {key} – dropping message");
            return Err(anyhow::anyhow!("Rate limit exceeded for {key}"));
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

        // Outbound rate limiting per destination
        let key = format!("{}:{}", msg.channel, msg.chat_id);
        if self.outbound_limiter.check_key(&key).is_err() {
            warn!("outbound rate limit hit for {key} – dropping message");
            return Err(anyhow::anyhow!("Outbound rate limit exceeded for {key}"));
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

        // Scan media paths — a filename can carry a secret (e.g. a
        // download cached as `/tmp/sk-ant-{token}.png`). Paths that
        // redact to a different string are dropped from the outbound
        // batch since the underlying file path is what the channel
        // uploads and we can't rewrite the file contents.
        msg.media.retain(|path| {
            let redacted = self.leak_detector.redact(path);
            if redacted == *path {
                true
            } else {
                warn!("security: secret in media path — dropping from outbound");
                false
            }
        });

        // Scan button context metadata for leaked secrets, using the
        // shared meta::BUTTONS constant so a future rename keeps the
        // scan lined up.
        if let Some(buttons) = msg
            .metadata
            .get_mut(oxicrab_core::bus::events::meta::BUTTONS)
            && let Some(arr) = buttons.as_array_mut()
        {
            for btn in arr.iter_mut() {
                if let Some(ctx) = btn.get_mut("context")
                    && let Some(ctx_str) = ctx.as_str()
                {
                    let redacted = self.leak_detector.redact(ctx_str);
                    if redacted != ctx_str {
                        warn!("security: secret leak in button context metadata — redacting");
                        *ctx = serde_json::Value::String(redacted);
                    }
                }
            }
        }

        // Scan all OTHER metadata values for leaks. Tool emitters
        // can stash arbitrary JSON on outbound; the per-string
        // recursion catches secrets in nested fields without
        // forcing each tool to scrub on its way out.
        for (key, value) in &mut msg.metadata {
            if key == oxicrab_core::bus::events::meta::BUTTONS {
                // Already handled above with the button-aware logic.
                continue;
            }
            redact_metadata_value(value, &self.leak_detector);
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
