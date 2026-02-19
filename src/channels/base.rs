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
    /// Default: sends normally, returns None (no editing support).
    async fn send_and_get_id(&self, msg: &OutboundMessage) -> anyhow::Result<Option<String>> {
        self.send(msg).await?;
        Ok(None)
    }

    /// Edit a previously sent message by its platform-specific ID.
    /// Default: no-op for channels that don't support editing.
    async fn edit_message(
        &self,
        _chat_id: &str,
        _message_id: &str,
        _content: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Delete a previously sent message by its platform-specific ID.
    /// Default: no-op for channels that don't support deletion.
    async fn delete_message(&self, _chat_id: &str, _message_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Split a message into chunks respecting UTF-8 character boundaries.
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-slack",
    feature = "channel-whatsapp",
    feature = "channel-twilio",
))]
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
                .map_or(remaining.len(), |(i, _)| i);
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

    // Filter out empty chunks (e.g., from leading "\n\n" producing a trimmed empty string)
    chunks.into_iter().filter(|c| !c.is_empty()).collect()
}

#[cfg(test)]
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-slack",
    feature = "channel-whatsapp",
    feature = "channel-twilio",
))]
mod tests {
    use super::*;

    #[test]
    fn test_short_message_no_split() {
        let result = split_message("hello world", 100);
        assert_eq!(result, vec!["hello world"]);
    }

    #[test]
    fn test_exact_limit_no_split() {
        let msg = "a".repeat(100);
        let result = split_message(&msg, 100);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].len(), 100);
    }

    #[test]
    fn test_split_at_paragraph_boundary() {
        let msg = "first paragraph\n\nsecond paragraph";
        let result = split_message(msg, 25);
        assert_eq!(result, vec!["first paragraph", "second paragraph"]);
    }

    #[test]
    fn test_split_at_newline_boundary() {
        let msg = "first line\nsecond line\nthird line";
        let result = split_message(msg, 20);
        assert_eq!(result[0], "first line");
    }

    #[test]
    fn test_hard_cut_no_boundary() {
        let msg = "a".repeat(200);
        let result = split_message(&msg, 100);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].len(), 100);
        assert_eq!(result[1].len(), 100);
    }

    #[test]
    fn test_utf8_multibyte_boundary_safety() {
        // Each emoji is 4 bytes. 25 chars * 4 bytes = 100 bytes
        let msg = "\u{1F600}".repeat(25);
        assert_eq!(msg.len(), 100);
        // Split at 10 bytes — should not land in the middle of a 4-byte char
        let result = split_message(&msg, 10);
        for chunk in &result {
            // Each chunk must be valid UTF-8 (would panic on construction if not)
            assert!(!chunk.is_empty());
            // Verify all chars are complete
            for c in chunk.chars() {
                assert_eq!(c, '\u{1F600}');
            }
        }
    }

    #[test]
    fn test_utf8_two_byte_chars() {
        // é is 2 bytes in UTF-8
        let msg = "é".repeat(60); // 120 bytes
        let result = split_message(&msg, 50);
        for chunk in &result {
            for c in chunk.chars() {
                assert_eq!(c, 'é');
            }
        }
    }

    #[test]
    fn test_empty_message() {
        let result = split_message("", 100);
        assert_eq!(result, vec![""]);
    }

    #[test]
    fn test_paragraph_preferred_over_newline() {
        let msg = "line1\nline2\n\nline3\nline4";
        let result = split_message(msg, 20);
        // Should split at \n\n first
        assert_eq!(result[0], "line1\nline2");
    }

    #[test]
    fn test_multiple_chunks() {
        let msg = "chunk1\n\nchunk2\n\nchunk3\n\nchunk4";
        let result = split_message(msg, 10);
        assert!(result.len() >= 4);
    }
}
