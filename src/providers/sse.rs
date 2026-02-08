use serde_json::Value;

/// A parsed Server-Sent Event.
#[derive(Debug)]
#[allow(dead_code)]
pub struct SseEvent {
    pub event_type: Option<String>,
    pub data: Option<Value>,
}

/// Parse a chunk of SSE text into events.
/// SSE events are separated by double newlines. Each event has optional `event:` and `data:` fields.
pub fn parse_sse_chunk(chunk: &str) -> Vec<SseEvent> {
    let mut events = Vec::new();
    let mut current_event_type: Option<String> = None;
    let mut current_data = String::new();

    for line in chunk.lines() {
        if line.is_empty() {
            // Empty line = end of event
            if !current_data.is_empty() {
                let data = current_data.trim().to_string();
                let parsed = if data == "[DONE]" {
                    None
                } else {
                    serde_json::from_str::<Value>(&data).ok()
                };
                events.push(SseEvent {
                    event_type: current_event_type.take(),
                    data: parsed,
                });
                current_data.clear();
            }
            current_event_type = None;
        } else if let Some(event_type) = line.strip_prefix("event: ") {
            current_event_type = Some(event_type.trim().to_string());
        } else if let Some(data) = line.strip_prefix("data: ") {
            if !current_data.is_empty() {
                current_data.push('\n');
            }
            current_data.push_str(data);
        }
    }

    // Handle final event if chunk doesn't end with empty line
    if !current_data.is_empty() {
        let data = current_data.trim().to_string();
        let parsed = if data == "[DONE]" {
            None
        } else {
            serde_json::from_str::<Value>(&data).ok()
        };
        events.push(SseEvent {
            event_type: current_event_type,
            data: parsed,
        });
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_event() {
        let chunk = "event: message_start\ndata: {\"type\":\"message_start\"}\n\n";
        let events = parse_sse_chunk(chunk);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type.as_deref(), Some("message_start"));
        assert!(events[0].data.is_some());
    }

    #[test]
    fn parse_multiple_events() {
        let chunk = "event: content_block_delta\ndata: {\"type\":\"delta\"}\n\nevent: message_stop\ndata: {\"type\":\"stop\"}\n\n";
        let events = parse_sse_chunk(chunk);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn parse_done_sentinel() {
        let chunk = "data: [DONE]\n\n";
        let events = parse_sse_chunk(chunk);
        assert_eq!(events.len(), 1);
        assert!(events[0].data.is_none());
    }

    #[test]
    fn parse_empty_chunk() {
        let events = parse_sse_chunk("");
        assert!(events.is_empty());
    }
}
