use crate::bus::OutboundMessage;
use async_trait::async_trait;

#[async_trait]
pub trait BaseChannel: Send + Sync {
    fn name(&self) -> &str;

    async fn start(&mut self) -> anyhow::Result<()>;
    async fn stop(&mut self) -> anyhow::Result<()>;
    async fn send(&self, msg: &OutboundMessage) -> anyhow::Result<()>;

    /// Send a typing indicator to signal the bot is processing.
    /// Default is a no-op for channels that don't support typing indicators.
    async fn send_typing(&self, _chat_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    /// Send a message and return its platform-specific ID for later editing.
    /// Default falls back to `send()` with no ID tracking.
    async fn send_and_get_id(&self, msg: &OutboundMessage) -> anyhow::Result<Option<String>> {
        self.send(msg).await?;
        Ok(None)
    }

    /// Edit a previously sent message by its platform-specific ID.
    /// Default is a no-op for channels that don't support editing.
    async fn edit_message(
        &self,
        _chat_id: &str,
        _message_id: &str,
        _new_content: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Split a message into chunks respecting UTF-8 character boundaries.
pub fn split_message(text: &str, limit: usize) -> Vec<String> {
    if text.len() <= limit {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while remaining.len() > limit {
        // Find the largest valid byte index <= limit that is a char boundary
        let mut split_at = limit;
        while split_at > 0 && !remaining.is_char_boundary(split_at) {
            split_at -= 1;
        }
        if split_at == 0 {
            // Degenerate case: single character wider than limit
            split_at = remaining
                .char_indices()
                .nth(1)
                .map(|(i, _)| i)
                .unwrap_or(remaining.len());
        }

        // Try paragraph boundary within the safe range
        if let Some(idx) = remaining[..split_at].rfind("\n\n") {
            chunks.push(remaining[..idx].trim().to_string());
            remaining = &remaining[idx + 2..];
            continue;
        }

        // Try single newline
        if let Some(idx) = remaining[..split_at].rfind('\n') {
            chunks.push(remaining[..idx].trim().to_string());
            remaining = &remaining[idx + 1..];
            continue;
        }

        // Hard cut at char boundary
        chunks.push(remaining[..split_at].to_string());
        remaining = &remaining[split_at..];
    }

    if !remaining.is_empty() {
        chunks.push(remaining.trim().to_string());
    }

    chunks
}
