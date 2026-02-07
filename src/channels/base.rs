use crate::bus::OutboundMessage;
use async_trait::async_trait;

#[async_trait]
pub trait BaseChannel: Send + Sync {
    fn name(&self) -> &str;

    async fn start(&mut self) -> anyhow::Result<()>;
    async fn stop(&mut self) -> anyhow::Result<()>;
    async fn send(&self, msg: &OutboundMessage) -> anyhow::Result<()>;

    fn is_allowed(&self, sender_id: &str, allow_list: &[String]) -> bool {
        if allow_list.is_empty() {
            return true;
        }

        let normalized: std::collections::HashSet<String> = allow_list
            .iter()
            .map(|a| a.trim_start_matches('+').to_string())
            .collect();

        let sender_str = sender_id.to_string();
        let parts: Vec<&str> = if sender_str.contains('|') {
            sender_str.split('|').collect()
        } else {
            vec![&sender_str]
        };

        for part in parts {
            if !part.is_empty() {
                let part_normalized = part.trim_start_matches('+');
                if normalized.contains(part) || normalized.contains(part_normalized) {
                    return true;
                }
            }
        }
        false
    }
}

pub fn split_message(text: &str, limit: usize) -> Vec<String> {
    if text.len() <= limit {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while remaining.len() > limit {
        // Try paragraph boundary
        if let Some(idx) = remaining[..limit].rfind("\n\n") {
            chunks.push(remaining[..idx].trim().to_string());
            remaining = &remaining[idx + 2..];
            continue;
        }

        // Try single newline
        if let Some(idx) = remaining[..limit].rfind('\n') {
            chunks.push(remaining[..idx].trim().to_string());
            remaining = &remaining[idx + 1..];
            continue;
        }

        // Hard cut
        chunks.push(remaining[..limit].to_string());
        remaining = &remaining[limit..];
    }

    if !remaining.is_empty() {
        chunks.push(remaining.trim().to_string());
    }

    chunks
}
