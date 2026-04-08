mod data_tool;
mod skill;

pub use data_tool::CollectionDataTool;

use crate::actions;
use crate::agent::memory::memory_db::{CollectionSchema, FieldDef, FieldType, MemoryDB};
use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities, ToolCategory};
use crate::agent::tools::tool_search::{SharedToolIndex, ToolIndexEntry};
use crate::agent::tools::{Tool, ToolResult};
use crate::require_param;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::fmt::Write as _;
use std::sync::Arc;
use tracing::info;

/// Validate collection names: alphanumeric and underscores, 1-64 chars,
/// starting with a letter.
fn is_valid_collection_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
}

/// Shared handle for runtime tool registration. Set after the
/// `ToolRegistry` is wrapped in `Arc` during `AgentLoop::new()`.
pub type RuntimeRegistry = Arc<std::sync::OnceLock<Arc<crate::agent::tools::ToolRegistry>>>;

/// Management tool for structured collections. Handles create, list,
/// describe, delete, and schema alteration. Per-collection CRUD is
/// provided by `CollectionDataTool` instances registered dynamically.
pub struct CollectionsTool {
    db: Arc<MemoryDB>,
    /// Shared `tool_search` index for runtime registration of new
    /// per-collection data tools.
    tool_index: Option<SharedToolIndex>,
    /// Handle to the tool registry for runtime deferred registration.
    /// Set after initial setup via `registry_handle()`.
    registry: RuntimeRegistry,
}

impl CollectionsTool {
    pub fn new(db: Arc<MemoryDB>, tool_index: Option<SharedToolIndex>) -> Self {
        Self {
            db,
            tool_index,
            registry: Arc::new(std::sync::OnceLock::new()),
        }
    }

    /// Get the shared registry handle so callers can set it later.
    pub fn registry_handle(&self) -> RuntimeRegistry {
        Arc::clone(&self.registry)
    }

    fn handle_create(&self, params: &Value) -> Result<ToolResult> {
        let name = params["name"].as_str().unwrap_or("").trim().to_lowercase();
        if !is_valid_collection_name(&name) {
            return Ok(ToolResult::error(
                "invalid collection name: must be 1-64 chars, \
                 alphanumeric/underscore, starting with a letter",
            ));
        }

        let description = params["description"].as_str().unwrap_or("").to_string();
        if description.is_empty() {
            return Ok(ToolResult::error("missing 'description' parameter"));
        }

        let fields = match parse_field_defs(&params["fields"]) {
            Ok(f) => f,
            Err(msg) => return Ok(ToolResult::error(msg)),
        };
        if fields.is_empty() {
            return Ok(ToolResult::error(
                "'fields' must be a non-empty array of field definitions",
            ));
        }

        let schema = CollectionSchema {
            fields: fields.clone(),
        };
        self.db.create_collection(&name, &description, &schema)?;

        // Register the per-collection data tool at runtime (deferred)
        let data_tool: Arc<dyn Tool> = Arc::new(CollectionDataTool::new(
            self.db.clone(),
            name.clone(),
            description.clone(),
            schema,
        ));

        if let Some(registry) = self.registry.get() {
            registry.register_runtime_deferred(data_tool);
        }

        // Add to tool_search index so LLM can discover it
        if let Some(ref idx) = self.tool_index {
            idx.lock().unwrap().push(ToolIndexEntry {
                name: name.clone(),
                description: format!("Collection: {name}. {description}"),
                deferred: true,
            });
        }

        // Write auto-generated skill file for context injection
        skill::write_collection_skill(&name, &description, &fields);

        info!("collection '{name}' created with {} fields", fields.len());

        let field_summary: Vec<String> = fields
            .iter()
            .map(|f| {
                let ft = format_field_type(&f.field_type);
                let mut s = format!("  - {} ({ft})", f.name);
                if f.required {
                    s.push_str(", required");
                }
                if !f.values.is_empty() {
                    let _ = write!(s, ", values: {:?}", f.values);
                }
                s
            })
            .collect();

        let fields_str = field_summary.join("\n");
        Ok(ToolResult::new(format!(
            "Collection '{name}' created.\n\nFields:\n{fields_str}\n\n\
             Use tool_search to find the '{name}' tool for adding, \
             querying, updating, and deleting records.",
        )))
    }

    fn handle_list(&self) -> Result<ToolResult> {
        let collections = self.db.list_collections()?;
        if collections.is_empty() {
            return Ok(ToolResult::new(
                "No collections exist yet. Use the 'create' action to make one.",
            ));
        }

        let mut lines = Vec::new();
        for c in &collections {
            let field_names: Vec<&str> = c.schema.fields.iter().map(|f| f.name.as_str()).collect();
            let joined = field_names.join(", ");
            lines.push(format!(
                "- **{}**: {} ({} records, fields: {joined})",
                c.name, c.description, c.record_count,
            ));
        }

        let body = lines.join("\n");
        Ok(ToolResult::new(format!(
            "Collections ({}):\n{body}",
            collections.len(),
        )))
    }

    fn handle_describe(&self, params: &Value) -> Result<ToolResult> {
        let name = params["name"].as_str().unwrap_or("");
        if name.is_empty() {
            return Ok(ToolResult::error("missing 'name' parameter"));
        }

        let coll = self
            .db
            .get_collection(name)?
            .ok_or_else(|| anyhow::anyhow!("collection '{name}' not found"))?;

        let field_lines: Vec<String> = coll
            .schema
            .fields
            .iter()
            .map(|f| {
                let ft = format_field_type(&f.field_type);
                let mut s = format!("  - {} ({ft})", f.name);
                if f.required {
                    s.push_str(", required");
                }
                if !f.values.is_empty() {
                    let _ = write!(s, ", values: {:?}", f.values);
                }
                s
            })
            .collect();

        let fields_str = field_lines.join("\n");
        Ok(ToolResult::new(format!(
            "Collection: {}\nDescription: {}\nRecords: {}\n\
             Created: {}\nUpdated: {}\n\nFields:\n{fields_str}",
            coll.name, coll.description, coll.record_count, coll.created_at, coll.updated_at,
        )))
    }

    fn handle_delete(&self, params: &Value) -> Result<ToolResult> {
        let name = params["name"].as_str().unwrap_or("");
        if name.is_empty() {
            return Ok(ToolResult::error("missing 'name' parameter"));
        }

        if self.db.delete_collection(name)? {
            skill::remove_collection_skill(name);
            info!("collection '{name}' deleted");
            Ok(ToolResult::new(format!("Collection '{name}' deleted.")))
        } else {
            Ok(ToolResult::error(format!("collection '{name}' not found")))
        }
    }

    fn handle_alter_schema(&self, params: &Value) -> Result<ToolResult> {
        let name = params["name"].as_str().unwrap_or("");
        if name.is_empty() {
            return Ok(ToolResult::error("missing 'name' parameter"));
        }

        let add_fields = match params.get("add_fields") {
            Some(v) if !v.is_null() => match parse_field_defs(v) {
                Ok(f) => f,
                Err(msg) => return Ok(ToolResult::error(msg)),
            },
            _ => Vec::new(),
        };

        let remove_fields: Vec<String> = params["remove_fields"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();

        if add_fields.is_empty() && remove_fields.is_empty() {
            return Ok(ToolResult::error(
                "at least one of 'add_fields' or 'remove_fields' is required",
            ));
        }

        self.db
            .alter_collection_schema(name, &add_fields, &remove_fields)?;

        // Regenerate skill file with updated schema
        if let Ok(Some(coll)) = self.db.get_collection(name) {
            skill::write_collection_skill(name, &coll.description, &coll.schema.fields);
        }

        let mut parts = Vec::new();
        if !add_fields.is_empty() {
            let names: Vec<&str> = add_fields.iter().map(|f| f.name.as_str()).collect();
            let joined = names.join(", ");
            parts.push(format!("added fields: {joined}"));
        }
        if !remove_fields.is_empty() {
            let joined = remove_fields.join(", ");
            parts.push(format!("removed fields: {joined}"));
        }

        let summary = parts.join("; ");
        Ok(ToolResult::new(format!(
            "Schema for '{name}' updated: {summary}"
        )))
    }
}

#[async_trait]
impl Tool for CollectionsTool {
    fn name(&self) -> &'static str {
        "collections"
    }

    fn description(&self) -> &'static str {
        "Manage structured data collections. Create collections with typed \
         schemas, then use per-collection tools (discovered via tool_search) \
         for CRUD operations. Actions: create, list, describe, delete, \
         alter_schema."
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            subagent_access: SubagentAccess::ReadOnly,
            actions: actions![create, list: ro, describe: ro, delete, alter_schema,],
            category: ToolCategory::Productivity,
            ..Default::default()
        }
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "list", "describe", "delete", "alter_schema"],
                    "description": "Action to perform"
                },
                "name": {
                    "type": "string",
                    "description": "Collection name (alphanumeric + underscore, \
                                    1-64 chars, starts with letter)"
                },
                "description": {
                    "type": "string",
                    "description": "Human-readable description of the collection \
                                    (required for create)"
                },
                "fields": {
                    "type": "array",
                    "description": "Field definitions for create. Each: \
                                    {name, type, required?, values?}",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "type": {
                                "type": "string",
                                "enum": ["text", "number", "bool", "enum",
                                         "date", "datetime"]
                            },
                            "required": { "type": "boolean" },
                            "values": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Allowed values (for enum type)"
                            }
                        },
                        "required": ["name", "type"]
                    }
                },
                "add_fields": {
                    "type": "array",
                    "description": "Fields to add (for alter_schema)",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "type": {
                                "type": "string",
                                "enum": ["text", "number", "bool", "enum",
                                         "date", "datetime"]
                            },
                            "required": { "type": "boolean" },
                            "values": {
                                "type": "array",
                                "items": { "type": "string" }
                            }
                        },
                        "required": ["name", "type"]
                    }
                },
                "remove_fields": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Field names to remove (for alter_schema)"
                }
            },
            "required": ["action"]
        })
    }

    fn usage_examples(&self) -> Vec<crate::agent::tools::base::ToolExample> {
        vec![
            crate::agent::tools::base::ToolExample {
                user_request: "create a grocery list".into(),
                params: serde_json::json!({
                    "action": "create",
                    "name": "grocery_list",
                    "description": "Weekly grocery shopping list",
                    "fields": [
                        {"name": "item", "type": "text", "required": true},
                        {"name": "quantity", "type": "number"},
                        {"name": "category", "type": "enum", "values": [
                            "produce", "dairy", "meat", "pantry", "other"
                        ]}
                    ]
                }),
            },
            crate::agent::tools::base::ToolExample {
                user_request: "what collections do I have".into(),
                params: serde_json::json!({"action": "list"}),
            },
        ]
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let action = require_param!(params, "action");

        match action {
            "create" => self.handle_create(&params),
            "list" => self.handle_list(),
            "describe" => self.handle_describe(&params),
            "delete" => self.handle_delete(&params),
            "alter_schema" => self.handle_alter_schema(&params),
            other => Ok(ToolResult::error(format!(
                "unknown action '{other}'. Expected: create, list, \
                 describe, delete, alter_schema"
            ))),
        }
    }
}

/// Parse field definitions from a JSON array.
fn parse_field_defs(value: &Value) -> std::result::Result<Vec<FieldDef>, String> {
    let arr = value.as_array().ok_or("'fields' must be an array")?;

    let mut fields = Vec::with_capacity(arr.len());
    for item in arr {
        let name = item["name"]
            .as_str()
            .ok_or("each field must have a 'name' string")?
            .to_string();

        if name.is_empty() || name.len() > 64 {
            return Err(format!("field name must be 1-64 chars, got '{name}'"));
        }

        let type_str = item["type"]
            .as_str()
            .ok_or("each field must have a 'type' string")?;

        let field_type = match type_str {
            "text" => FieldType::Text,
            "number" => FieldType::Number,
            "bool" => FieldType::Bool,
            "enum" => FieldType::Enum,
            "date" => FieldType::Date,
            "datetime" => FieldType::Datetime,
            other => {
                return Err(format!(
                    "unknown field type '{other}'. Expected: text, number, \
                     bool, enum, date, datetime"
                ));
            }
        };

        let required = item["required"].as_bool().unwrap_or(false);

        let values: Vec<String> = item["values"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();

        if matches!(field_type, FieldType::Enum) && values.is_empty() {
            return Err(format!(
                "enum field '{name}' requires a non-empty 'values' array"
            ));
        }

        fields.push(FieldDef {
            name,
            field_type,
            required,
            values,
        });
    }

    Ok(fields)
}

fn format_field_type(ft: &FieldType) -> &'static str {
    match ft {
        FieldType::Text => "text",
        FieldType::Number => "number",
        FieldType::Bool => "bool",
        FieldType::Enum => "enum",
        FieldType::Date => "date",
        FieldType::Datetime => "datetime",
    }
}

#[cfg(test)]
mod tests;
