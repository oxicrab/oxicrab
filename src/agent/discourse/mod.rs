//! Discourse entity register for contextual reference resolution.
//!
//! Tracks actionable entities (tasks, issues, files, events, etc.) mentioned in
//! tool results and assistant text so that the LLM can resolve anaphoric references
//! like "that task" or "the second one" without asking unnecessary clarification
//! questions.

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::LazyLock;
use tracing::debug;

/// Maximum number of tracked entities before oldest are evicted.
const MAX_ENTITIES: usize = 20;

/// Entities older than this many turns are pruned.
const MAX_AGE_TURNS: usize = 10;

/// Session metadata key for the entity register.
const METADATA_KEY: &str = "discourse_entities";

/// A tracked entity extracted from tool results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscourseEntity {
    /// Type of entity: "task", "issue", "pr", "file", "event", "reminder", etc.
    pub entity_type: String,
    /// Unique identifier from the source system (e.g. task ID, issue number).
    pub entity_id: String,
    /// Human-readable label (e.g. "Call Sun Logistics", "Fix login bug").
    pub label: String,
    /// Which tool produced this entity.
    pub source_tool: String,
    /// Conversation turn when this entity was last seen.
    pub last_turn: usize,
}

/// Tracks recently mentioned actionable entities for reference resolution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiscourseRegister {
    pub entities: Vec<DiscourseEntity>,
    /// Current turn counter (incremented each agent loop iteration that uses tools).
    pub turn: usize,
}

impl DiscourseRegister {
    /// Load from session metadata, returning a default register if absent.
    pub fn from_session_metadata(metadata: &HashMap<String, Value>) -> Self {
        metadata
            .get(METADATA_KEY)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    }

    /// Persist to session metadata.
    pub fn to_session_metadata(&self, metadata: &mut HashMap<String, Value>) {
        if let Ok(val) = serde_json::to_value(self) {
            metadata.insert(METADATA_KEY.to_string(), val);
        }
    }

    /// Register entities extracted from a tool result, deduplicating by
    /// (`entity_type`, `entity_id`). Existing entities have their turn refreshed.
    pub fn register(&mut self, entities: Vec<DiscourseEntity>) {
        for entity in entities {
            if let Some(existing) = self
                .entities
                .iter_mut()
                .find(|e| e.entity_type == entity.entity_type && e.entity_id == entity.entity_id)
            {
                existing.last_turn = entity.last_turn;
                existing.label = entity.label;
            } else {
                self.entities.push(entity);
            }
        }
        self.prune();
    }

    /// Advance the turn counter.
    pub fn advance_turn(&mut self) {
        self.turn += 1;
    }

    /// Remove stale entities and enforce the size cap.
    fn prune(&mut self) {
        // Remove entities that haven't been seen in MAX_AGE_TURNS
        self.entities
            .retain(|e| self.turn.saturating_sub(e.last_turn) <= MAX_AGE_TURNS);

        // Keep only the most recent MAX_ENTITIES
        if self.entities.len() > MAX_ENTITIES {
            self.entities
                .sort_by_key(|e| std::cmp::Reverse(e.last_turn));
            self.entities.truncate(MAX_ENTITIES);
        }
    }

    /// Generate a context string for injection into the system prompt.
    /// Returns `None` if no entities are tracked.
    pub fn to_context_string(&self) -> Option<String> {
        if self.entities.is_empty() {
            return None;
        }

        let mut lines = Vec::new();
        // Sort by most recent first
        let mut sorted = self.entities.clone();
        sorted.sort_by_key(|e| std::cmp::Reverse(e.last_turn));

        for e in &sorted {
            lines.push(format!(
                "- {} [{}]: {} (from {})",
                e.entity_type, e.entity_id, e.label, e.source_tool
            ));
        }

        Some(lines.join("\n"))
    }

    /// Extract entities from a tool result. Tries JSON parsing first for
    /// structured results, falls back to simple pattern extraction.
    pub fn extract_from_tool_result(
        tool_name: &str,
        result: &str,
        turn: usize,
    ) -> Vec<DiscourseEntity> {
        if result.is_empty() {
            return vec![];
        }

        let mut entities = Vec::new();

        // Try parsing as JSON (single object or array)
        if let Ok(val) = serde_json::from_str::<Value>(result) {
            extract_entities_from_json(tool_name, &val, turn, &mut entities);
        } else {
            // Try to find embedded JSON objects in the text
            extract_entities_from_text(tool_name, result, turn, &mut entities);
        }

        if !entities.is_empty() {
            debug!(
                "extracted {} entities from tool '{}' result",
                entities.len(),
                tool_name
            );
        }

        entities
    }

    /// Extract entities from assistant text responses.
    ///
    /// Catches entities mentioned in natural language — action claims ("Created: X"),
    /// numbered/bulleted list items, and explicit entity references. This provides
    /// robustness when tool results are unavailable (e.g. hallucinated actions) or
    /// when the assistant summarizes entities in prose.
    pub fn extract_from_assistant_text(text: &str, turn: usize) -> Vec<DiscourseEntity> {
        if text.is_empty() {
            return vec![];
        }

        let mut entities = Vec::new();

        // Pattern 1: Action claims — "Created: Task Name", "Updated: Something", etc.
        extract_action_claim_entities(text, turn, &mut entities);

        // Pattern 2: Numbered/bulleted list items that look like actionable entities
        extract_list_entities(text, turn, &mut entities);

        if !entities.is_empty() {
            debug!("extracted {} entities from assistant text", entities.len());
        }

        entities
    }
}

/// Extract entities from a parsed JSON value.
fn extract_entities_from_json(
    tool_name: &str,
    val: &Value,
    turn: usize,
    entities: &mut Vec<DiscourseEntity>,
) {
    match val {
        Value::Array(arr) => {
            for item in arr {
                extract_entity_from_object(tool_name, item, turn, entities);
            }
        }
        Value::Object(_) => {
            // Check if this is a wrapper with an inner array (e.g. {"tasks": [...]})
            if let Some(inner_arr) = find_entity_array(val) {
                for item in inner_arr {
                    extract_entity_from_object(tool_name, item, turn, entities);
                }
            } else {
                extract_entity_from_object(tool_name, val, turn, entities);
            }
        }
        _ => {}
    }
}

/// Look for an array field that likely contains entities.
fn find_entity_array(obj: &Value) -> Option<&Vec<Value>> {
    const ARRAY_KEYS: &[&str] = &[
        "tasks",
        "items",
        "issues",
        "results",
        "events",
        "files",
        "entries",
        "records",
        "jobs",
        "reminders",
        "notes",
        "pulls",
        "pull_requests",
        "notifications",
        "workflows",
    ];

    if let Value::Object(map) = obj {
        for key in ARRAY_KEYS {
            if let Some(Value::Array(arr)) = map.get(*key) {
                return Some(arr);
            }
        }
        // If there's exactly one array field, use it
        let arrays: Vec<&Vec<Value>> = map
            .values()
            .filter_map(|v| {
                if let Value::Array(a) = v {
                    Some(a)
                } else {
                    None
                }
            })
            .collect();
        if arrays.len() == 1 {
            return Some(arrays[0]);
        }
    }
    None
}

/// Try to extract an entity from a JSON object by looking for common field patterns.
fn extract_entity_from_object(
    tool_name: &str,
    val: &Value,
    turn: usize,
    entities: &mut Vec<DiscourseEntity>,
) {
    let Some(obj) = val.as_object() else {
        return;
    };

    // Extract entity ID — try several common keys
    let entity_id = obj
        .get("id")
        .or_else(|| obj.get("task_id"))
        .or_else(|| obj.get("issue_id"))
        .or_else(|| obj.get("number"))
        .or_else(|| obj.get("job_id"))
        .or_else(|| obj.get("event_id"))
        .and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        });

    // Extract label — try several common keys
    let label = obj
        .get("content")
        .or_else(|| obj.get("title"))
        .or_else(|| obj.get("name"))
        .or_else(|| obj.get("summary"))
        .or_else(|| obj.get("description"))
        .or_else(|| obj.get("subject"))
        .and_then(|v| v.as_str())
        .map(truncate_label);

    // Need at least an ID or a label to register an entity
    let (entity_id, label) = match (entity_id, label) {
        (Some(id), Some(lbl)) => (id, lbl),
        (Some(id), None) => (id.clone(), id),
        (None, Some(lbl)) => (lbl.clone(), lbl),
        (None, None) => return,
    };

    // Infer entity type from tool name and object fields
    let entity_type = infer_entity_type(tool_name, obj);

    entities.push(DiscourseEntity {
        entity_type,
        entity_id,
        label,
        source_tool: tool_name.to_string(),
        last_turn: turn,
    });
}

/// Infer the entity type from the tool name and object fields.
fn infer_entity_type(tool_name: &str, obj: &serde_json::Map<String, Value>) -> String {
    // Tool-name-based heuristics
    let tool_lower = tool_name.to_lowercase();
    if tool_lower.contains("todoist") || tool_lower.contains("task") {
        return "task".to_string();
    }
    if tool_lower.contains("github") {
        if obj.contains_key("pull_request")
            || obj.get("type").and_then(|v| v.as_str()) == Some("pr")
        {
            return "pr".to_string();
        }
        return "issue".to_string();
    }
    if tool_lower.contains("calendar") || tool_lower.contains("event") {
        return "event".to_string();
    }
    if tool_lower.contains("cron") || tool_lower.contains("schedule") {
        return "job".to_string();
    }
    if tool_lower.contains("email") || tool_lower.contains("gmail") {
        return "email".to_string();
    }
    if tool_lower.contains("file") || tool_lower.contains("read") || tool_lower.contains("write") {
        return "file".to_string();
    }
    if tool_lower.contains("reminder") {
        return "reminder".to_string();
    }

    // Field-based heuristics
    if obj.contains_key("due_date") || obj.contains_key("due") || obj.contains_key("priority") {
        return "task".to_string();
    }
    if obj.contains_key("start_time")
        || obj.contains_key("end_time")
        || obj.contains_key("attendees")
    {
        return "event".to_string();
    }
    if obj.contains_key("schedule") || obj.contains_key("cron") || obj.contains_key("next_run") {
        return "job".to_string();
    }
    if obj.contains_key("path") || obj.contains_key("filename") {
        return "file".to_string();
    }

    "item".to_string()
}

/// Try to extract entities from plain-text tool results that contain
/// structured-looking content (e.g. numbered lists, markdown lists).
fn extract_entities_from_text(
    tool_name: &str,
    text: &str,
    turn: usize,
    entities: &mut Vec<DiscourseEntity>,
) {
    // Look for embedded JSON objects or arrays in the text
    for line in text.lines() {
        let trimmed = line.trim();
        if ((trimmed.starts_with('{') && trimmed.ends_with('}'))
            || (trimmed.starts_with('[') && trimmed.ends_with(']')))
            && let Ok(val) = serde_json::from_str::<Value>(trimmed)
        {
            extract_entities_from_json(tool_name, &val, turn, entities);
        }
    }
}

/// Truncate a label to a reasonable display length.
fn truncate_label(s: &str) -> String {
    if s.len() <= 80 {
        s.to_string()
    } else if s.is_char_boundary(77) {
        format!("{}...", &s[..77])
    } else {
        // Find the nearest char boundary
        let mut end = 77;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

// ── Assistant text entity extraction ────────────────────────────

/// Regex for action claims: "Created: Task name", "Updated: Something", etc.
static ACTION_CLAIM_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?mi)^\s*(?:Created|Updated|Deleted|Removed|Added|Saved|Sent|Scheduled|Completed|Closed|Configured|Fixed|Applied|Set up|Marked(?: as)? (?:complete|done))\s*[:—]\s*(.+)",
    )
    .unwrap()
});

/// Regex for numbered list items: "1. Task name", "2) Something", etc.
static NUMBERED_LIST_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*\d+[.)]\s+(.+)").unwrap());

/// Regex for bulleted list items: "- Task name", "• Something", "* Item", etc.
static BULLET_LIST_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*[-•*]\s+(.+)").unwrap());

/// Extract entities from action claim patterns in assistant text.
fn extract_action_claim_entities(text: &str, turn: usize, entities: &mut Vec<DiscourseEntity>) {
    for cap in ACTION_CLAIM_RE.captures_iter(text) {
        let label = cap[1].trim().to_string();
        if label.len() < 3 {
            continue;
        }
        let label = truncate_label(&label);
        entities.push(DiscourseEntity {
            entity_type: "item".to_string(),
            entity_id: label.clone(),
            label,
            source_tool: "assistant_text".to_string(),
            last_turn: turn,
        });
    }
}

/// Extract entities from numbered and bulleted list items in assistant text.
/// Only extracts when the list has 1–10 items (likely an entity list, not prose).
fn extract_list_entities(text: &str, turn: usize, entities: &mut Vec<DiscourseEntity>) {
    // Collect numbered list matches
    let numbered: Vec<String> = NUMBERED_LIST_RE
        .captures_iter(text)
        .map(|c| c[1].trim().to_string())
        .filter(|s| s.len() >= 3)
        .collect();

    // Only use numbered items if the list is a reasonable size (1–10 items)
    if (1..=10).contains(&numbered.len()) {
        for label in numbered {
            let label = truncate_label(&label);
            entities.push(DiscourseEntity {
                entity_type: "item".to_string(),
                entity_id: label.clone(),
                label,
                source_tool: "assistant_text".to_string(),
                last_turn: turn,
            });
        }
        return; // Numbered list found, skip bullet extraction to avoid dupes
    }

    // Fall back to bullet list extraction
    let bullets: Vec<String> = BULLET_LIST_RE
        .captures_iter(text)
        .map(|c| c[1].trim().to_string())
        .filter(|s| s.len() >= 3)
        .collect();

    if (1..=10).contains(&bullets.len()) {
        for label in bullets {
            let label = truncate_label(&label);
            entities.push(DiscourseEntity {
                entity_type: "item".to_string(),
                entity_id: label.clone(),
                label,
                source_tool: "assistant_text".to_string(),
                last_turn: turn,
            });
        }
    }
}

#[cfg(test)]
mod tests;
