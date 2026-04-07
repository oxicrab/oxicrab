use crate::agent::memory::memory_db::{
    AggFunction, AggregationRequest, CollectionSchema, FieldType, FilterOp, MemoryDB, RecordFilter,
};
use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities, ToolCategory};
use crate::agent::tools::{Tool, ToolResult};
use crate::require_param;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

use super::format_field_type;

/// Per-collection data tool providing CRUD and aggregation operations.
/// One instance exists per collection, registered as a deferred tool
/// whose name matches the collection name.
pub struct CollectionDataTool {
    db: Arc<MemoryDB>,
    collection_name: String,
    collection_description: String,
    schema: CollectionSchema,
}

impl CollectionDataTool {
    pub fn new(
        db: Arc<MemoryDB>,
        collection_name: String,
        collection_description: String,
        schema: CollectionSchema,
    ) -> Self {
        Self {
            db,
            collection_name,
            collection_description,
            schema,
        }
    }

    fn handle_add(&self, params: &Value) -> Result<ToolResult> {
        let data = params
            .get("data")
            .cloned()
            .unwrap_or(Value::Object(serde_json::Map::new()));

        if !data.is_object() {
            return Ok(ToolResult::error("'data' must be a JSON object"));
        }

        let id = self.db.insert_record(&self.collection_name, data)?;
        Ok(ToolResult::new(format!(
            "Record added to '{}' with id: {}",
            self.collection_name, id,
        )))
    }

    fn handle_query(&self, params: &Value) -> Result<ToolResult> {
        let filters = parse_filters(params.get("filters"))?;
        let limit = params["limit"].as_u64().map(|v| v.min(100) as u32);
        let offset = params["offset"].as_u64().map(|v| v as u32);

        let records = self
            .db
            .query_records(&self.collection_name, &filters, limit, offset)?;

        if records.is_empty() {
            return Ok(ToolResult::new(format!(
                "No records found in '{}'.",
                self.collection_name,
            )));
        }

        let mut lines = Vec::with_capacity(records.len());
        for r in &records {
            lines.push(format!(
                "- [{}] {} (created: {}, updated: {})",
                r.id,
                serde_json::to_string(&r.data).unwrap_or_default(),
                r.created_at,
                r.updated_at,
            ));
        }

        Ok(ToolResult::new(format!(
            "Records in '{}' ({} shown, offset {}):\n{}",
            self.collection_name,
            records.len(),
            offset.unwrap_or(0),
            lines.join("\n"),
        )))
    }

    fn handle_update(&self, params: &Value) -> Result<ToolResult> {
        let id = params["id"].as_str().unwrap_or("");
        if id.is_empty() {
            return Ok(ToolResult::error("missing 'id' parameter"));
        }

        let data = params
            .get("data")
            .cloned()
            .unwrap_or(Value::Object(serde_json::Map::new()));
        if !data.is_object() {
            return Ok(ToolResult::error("'data' must be a JSON object"));
        }

        if self.db.update_record(&self.collection_name, id, data)? {
            Ok(ToolResult::new(format!(
                "Record '{}' updated in '{}'.",
                id, self.collection_name,
            )))
        } else {
            Ok(ToolResult::error(format!(
                "record '{}' not found in '{}'",
                id, self.collection_name,
            )))
        }
    }

    fn handle_delete(&self, params: &Value) -> Result<ToolResult> {
        let id = params["id"].as_str().unwrap_or("");
        if id.is_empty() {
            return Ok(ToolResult::error("missing 'id' parameter"));
        }

        if self.db.delete_record(&self.collection_name, id)? {
            Ok(ToolResult::new(format!(
                "Record '{}' deleted from '{}'.",
                id, self.collection_name,
            )))
        } else {
            Ok(ToolResult::error(format!(
                "record '{}' not found in '{}'",
                id, self.collection_name,
            )))
        }
    }

    fn handle_count(&self, params: &Value) -> Result<ToolResult> {
        let filters = parse_filters(params.get("filters"))?;
        let count = self.db.count_records(&self.collection_name, &filters)?;

        Ok(ToolResult::new(format!(
            "Count in '{}': {}",
            self.collection_name, count,
        )))
    }

    fn handle_aggregate(&self, params: &Value) -> Result<ToolResult> {
        let func_str = params["function"].as_str().unwrap_or("");
        let function = match func_str {
            "count" => AggFunction::Count,
            "sum" => AggFunction::Sum,
            "avg" => AggFunction::Avg,
            "min" => AggFunction::Min,
            "max" => AggFunction::Max,
            "" => return Ok(ToolResult::error("missing 'function' parameter")),
            other => {
                return Ok(ToolResult::error(format!(
                    "unknown aggregation function '{other}'. \
                     Expected: count, sum, avg, min, max"
                )));
            }
        };

        let field = params["field"].as_str().unwrap_or("").to_string();
        if field.is_empty() {
            return Ok(ToolResult::error("missing 'field' parameter"));
        }

        let group_by = params["group_by"].as_str().map(String::from);

        let filters = parse_filters(params.get("filters")).ok();

        let request = AggregationRequest {
            function,
            field,
            group_by,
            filters,
        };

        let results = self.db.aggregate_records(&self.collection_name, &request)?;

        if results.is_empty() {
            return Ok(ToolResult::new(format!(
                "No results for aggregation on '{}'.",
                self.collection_name,
            )));
        }

        let mut lines = Vec::with_capacity(results.len());
        for r in &results {
            if let Some(ref group) = r.group {
                lines.push(format!("  {}: {}", group, r.value));
            } else {
                lines.push(format!("  {}", r.value));
            }
        }

        Ok(ToolResult::new(format!(
            "Aggregation ({func_str}) on '{}':\n{}",
            self.collection_name,
            lines.join("\n"),
        )))
    }

    /// Build a JSON Schema for the `data` parameter based on the
    /// collection's field definitions.
    fn data_schema(&self) -> Value {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();

        for field in &self.schema.fields {
            let mut prop = serde_json::Map::new();
            match field.field_type {
                FieldType::Text => {
                    prop.insert("type".into(), Value::String("string".into()));
                }
                FieldType::Number => {
                    prop.insert("type".into(), Value::String("number".into()));
                }
                FieldType::Bool => {
                    prop.insert("type".into(), Value::String("boolean".into()));
                }
                FieldType::Enum => {
                    prop.insert("type".into(), Value::String("string".into()));
                    let vals: Vec<Value> = field
                        .values
                        .iter()
                        .map(|v| Value::String(v.clone()))
                        .collect();
                    prop.insert("enum".into(), Value::Array(vals));
                }
                FieldType::Date | FieldType::Datetime => {
                    prop.insert("type".into(), Value::String("string".into()));
                    let fmt = if matches!(field.field_type, FieldType::Date) {
                        "date"
                    } else {
                        "date-time"
                    };
                    prop.insert(
                        "description".into(),
                        Value::String(format!("{} ({})", field.name, fmt)),
                    );
                }
            }
            properties.insert(field.name.clone(), Value::Object(prop));
            if field.required {
                required.push(Value::String(field.name.clone()));
            }
        }

        let mut schema = serde_json::Map::new();
        schema.insert("type".into(), Value::String("object".into()));
        schema.insert("properties".into(), Value::Object(properties));
        if !required.is_empty() {
            schema.insert("required".into(), Value::Array(required));
        }
        Value::Object(schema)
    }
}

#[async_trait]
impl Tool for CollectionDataTool {
    fn name(&self) -> &str {
        &self.collection_name
    }

    fn description(&self) -> &str {
        &self.collection_description
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            subagent_access: SubagentAccess::ReadOnly,
            actions: vec![
                crate::agent::tools::base::ActionDescriptor {
                    name: "add",
                    read_only: false,
                },
                crate::agent::tools::base::ActionDescriptor {
                    name: "query",
                    read_only: true,
                },
                crate::agent::tools::base::ActionDescriptor {
                    name: "update",
                    read_only: false,
                },
                crate::agent::tools::base::ActionDescriptor {
                    name: "delete",
                    read_only: false,
                },
                crate::agent::tools::base::ActionDescriptor {
                    name: "count",
                    read_only: true,
                },
                crate::agent::tools::base::ActionDescriptor {
                    name: "aggregate",
                    read_only: true,
                },
            ],
            category: ToolCategory::Productivity,
            ..Default::default()
        }
    }

    fn parameters(&self) -> Value {
        let field_names: Vec<String> = self
            .schema
            .fields
            .iter()
            .map(|f| {
                let ft = format_field_type(&f.field_type);
                if f.required {
                    format!("{} ({}, required)", f.name, ft)
                } else {
                    format!("{} ({})", f.name, ft)
                }
            })
            .collect();

        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["add", "query", "update", "delete",
                             "count", "aggregate"],
                    "description": format!(
                        "Action to perform on the '{}' collection. \
                         Fields: {}",
                        self.collection_name,
                        field_names.join(", "),
                    )
                },
                "data": self.data_schema(),
                "id": {
                    "type": "string",
                    "description": "Record ID (for update, delete)"
                },
                "filters": {
                    "type": "array",
                    "description": "Query filters: [{field, op, value}]. \
                                    Ops: eq, neq, gt, gte, lt, lte, contains",
                    "items": {
                        "type": "object",
                        "properties": {
                            "field": { "type": "string" },
                            "op": {
                                "type": "string",
                                "enum": ["eq", "neq", "gt", "gte",
                                         "lt", "lte", "contains"]
                            },
                            "value": {}
                        },
                        "required": ["field", "op", "value"]
                    }
                },
                "limit": {
                    "type": "integer",
                    "description": "Max records to return (default 20, max 100)"
                },
                "offset": {
                    "type": "integer",
                    "description": "Number of records to skip (pagination)"
                },
                "function": {
                    "type": "string",
                    "enum": ["count", "sum", "avg", "min", "max"],
                    "description": "Aggregation function"
                },
                "field": {
                    "type": "string",
                    "description": "Field to aggregate on"
                },
                "group_by": {
                    "type": "string",
                    "description": "Optional field to group results by"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let action = require_param!(params, "action");

        match action {
            "add" => self.handle_add(&params),
            "query" => self.handle_query(&params),
            "update" => self.handle_update(&params),
            "delete" => self.handle_delete(&params),
            "count" => self.handle_count(&params),
            "aggregate" => self.handle_aggregate(&params),
            other => Ok(ToolResult::error(format!(
                "unknown action '{other}'. Expected: add, query, update, \
                 delete, count, aggregate"
            ))),
        }
    }
}

/// Parse filter definitions from a JSON array.
fn parse_filters(value: Option<&Value>) -> Result<Vec<RecordFilter>> {
    let Some(arr) = value.and_then(Value::as_array) else {
        return Ok(Vec::new());
    };

    let mut filters = Vec::with_capacity(arr.len());
    for item in arr {
        let field = item["field"].as_str().unwrap_or("").to_string();
        if field.is_empty() {
            continue;
        }

        let op = match item["op"].as_str().unwrap_or("eq") {
            "eq" => FilterOp::Eq,
            "neq" => FilterOp::Neq,
            "gt" => FilterOp::Gt,
            "gte" => FilterOp::Gte,
            "lt" => FilterOp::Lt,
            "lte" => FilterOp::Lte,
            "contains" => FilterOp::Contains,
            other => {
                anyhow::bail!(
                    "unknown filter op '{other}'. Expected: eq, neq, gt, \
                     gte, lt, lte, contains"
                );
            }
        };

        let value = item.get("value").cloned().unwrap_or(Value::Null);

        filters.push(RecordFilter { field, op, value });
    }

    Ok(filters)
}
