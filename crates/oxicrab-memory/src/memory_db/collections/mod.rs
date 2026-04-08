mod date_parse;

use super::MemoryDB;
use super::escape_like;
use anyhow::{Result, bail};
use date_parse::{parse_natural_date, parse_natural_datetime};
// Re-export chrono types used by tests
#[cfg(test)]
use chrono::{Datelike, Duration, Local, NaiveDate, Weekday};
#[cfg(test)]
fn today() -> NaiveDate {
    Local::now().date_naive()
}
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tracing::debug;

const MAX_COLLECTIONS: usize = 50;
const MAX_FIELDS: usize = 10;
const MAX_RECORDS: u64 = 10_000;
const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;
const MAX_NAME_LEN: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionSchema {
    pub fields: Vec<FieldDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDef {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: FieldType,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    Text,
    Number,
    Bool,
    Enum,
    Date,
    Datetime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionInfo {
    pub name: String,
    pub description: String,
    pub schema: CollectionSchema,
    pub record_count: u64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionRecord {
    pub id: String,
    pub data: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordFilter {
    pub field: String,
    pub op: FilterOp,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FilterOp {
    Eq,
    Neq,
    Gt,
    Gte,
    Lt,
    Lte,
    Contains,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregationRequest {
    pub function: AggFunction,
    pub field: String,
    pub group_by: Option<String>,
    pub filters: Option<Vec<RecordFilter>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AggFunction {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregationResult {
    pub value: serde_json::Value,
    pub group: Option<String>,
}

// --- Validation helpers ---

fn validate_collection_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        bail!(
            "collection name must be 1-{MAX_NAME_LEN} characters, got {}",
            name.len()
        );
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        bail!("collection name must contain only alphanumeric characters and underscores");
    }
    Ok(())
}

fn validate_schema(schema: &CollectionSchema) -> Result<()> {
    if schema.fields.is_empty() {
        bail!("schema must have at least one field");
    }
    if schema.fields.len() > MAX_FIELDS {
        bail!(
            "schema can have at most {MAX_FIELDS} fields, got {}",
            schema.fields.len()
        );
    }
    for field in &schema.fields {
        if field.name.is_empty()
            || !field
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
            || !field.name.starts_with(|c: char| c.is_ascii_alphabetic())
        {
            bail!(
                "field name '{}' must start with a letter and contain only \
                 alphanumeric characters and underscores",
                field.name
            );
        }
        if field.field_type == FieldType::Enum && field.values.is_empty() {
            bail!("enum field '{}' must have at least one value", field.name);
        }
    }
    Ok(())
}

fn validate_field_value(value: &serde_json::Value, field: &FieldDef) -> Result<serde_json::Value> {
    match field.field_type {
        FieldType::Text => {
            if value.is_string() {
                Ok(value.clone())
            } else {
                bail!(
                    "field '{}' expects text, got {}",
                    field.name,
                    value_type_name(value)
                );
            }
        }
        FieldType::Number => {
            if value.is_number() {
                Ok(value.clone())
            } else if let Some(s) = value.as_str() {
                if let Ok(n) = s.parse::<f64>() {
                    Ok(serde_json::json!(n))
                } else {
                    bail!("field '{}' expects number, got text '{s}'", field.name);
                }
            } else {
                bail!(
                    "field '{}' expects number, got {}",
                    field.name,
                    value_type_name(value)
                );
            }
        }
        FieldType::Bool => {
            if value.is_boolean() {
                Ok(value.clone())
            } else if let Some(s) = value.as_str() {
                match s.to_lowercase().as_str() {
                    "true" | "yes" | "1" => Ok(serde_json::json!(true)),
                    "false" | "no" | "0" => Ok(serde_json::json!(false)),
                    _ => bail!("field '{}' expects bool, got text '{s}'", field.name),
                }
            } else {
                bail!(
                    "field '{}' expects bool, got {}",
                    field.name,
                    value_type_name(value)
                );
            }
        }
        FieldType::Enum => {
            let s = value.as_str().ok_or_else(|| {
                anyhow::anyhow!(
                    "field '{}' expects enum string, got {}",
                    field.name,
                    value_type_name(value)
                )
            })?;
            if !field.values.iter().any(|v| v == s) {
                bail!(
                    "field '{}' value '{s}' not in allowed values: {:?}",
                    field.name,
                    field.values
                );
            }
            Ok(value.clone())
        }
        FieldType::Date => {
            let s = value.as_str().ok_or_else(|| {
                anyhow::anyhow!(
                    "field '{}' expects date string, got {}",
                    field.name,
                    value_type_name(value)
                )
            })?;
            let parsed = parse_natural_date(s)?;
            Ok(serde_json::json!(parsed))
        }
        FieldType::Datetime => {
            let s = value.as_str().ok_or_else(|| {
                anyhow::anyhow!(
                    "field '{}' expects datetime string, got {}",
                    field.name,
                    value_type_name(value)
                )
            })?;
            let parsed = parse_natural_datetime(s)?;
            Ok(serde_json::json!(parsed))
        }
    }
}

fn value_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

fn validate_record_data(
    data: &serde_json::Value,
    schema: &CollectionSchema,
    partial: bool,
) -> Result<serde_json::Value> {
    let obj = data
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("record data must be a JSON object"))?;

    // Check for unknown fields
    for key in obj.keys() {
        if !schema.fields.iter().any(|f| f.name == *key) {
            bail!("unknown field '{key}' not in schema");
        }
    }

    let mut coerced = serde_json::Map::new();

    for field in &schema.fields {
        if let Some(value) = obj.get(&field.name) {
            if value.is_null() {
                if field.required && !partial {
                    bail!("required field '{}' cannot be null", field.name);
                }
                // skip null values
                continue;
            }
            let validated = validate_field_value(value, field)?;
            coerced.insert(field.name.clone(), validated);
        } else if field.required && !partial {
            bail!("required field '{}' is missing", field.name);
        }
    }

    Ok(serde_json::Value::Object(coerced))
}

fn build_filter_clause(
    filters: &[RecordFilter],
    schema: &CollectionSchema,
) -> Result<(String, Vec<Box<dyn rusqlite::types::ToSql>>)> {
    if filters.is_empty() {
        return Ok((String::new(), vec![]));
    }

    let mut clauses = Vec::new();
    let mut bind_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    for filter in filters {
        let field_def = schema
            .fields
            .iter()
            .find(|f| f.name == filter.field)
            .ok_or_else(|| anyhow::anyhow!("filter field '{}' not in schema", filter.field))?;

        let json_path = format!("$.{}", filter.field);
        let idx = bind_values.len() + 1;

        match filter.op {
            FilterOp::Contains => {
                if field_def.field_type != FieldType::Text {
                    bail!(
                        "'contains' filter only works on text fields, '{}' is {:?}",
                        filter.field,
                        field_def.field_type
                    );
                }
                let s = filter
                    .value
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("'contains' filter value must be a string"))?;
                let escaped = escape_like(s);
                clauses.push(format!(
                    "json_extract(data_json, '{json_path}') LIKE ?{idx} ESCAPE '\\'"
                ));
                bind_values.push(Box::new(format!("%{escaped}%")));
            }
            ref op => {
                let sql_op = match op {
                    FilterOp::Eq => "=",
                    FilterOp::Neq => "!=",
                    FilterOp::Gt => ">",
                    FilterOp::Gte => ">=",
                    FilterOp::Lt => "<",
                    FilterOp::Lte => "<=",
                    FilterOp::Contains => unreachable!(),
                };
                clauses.push(format!(
                    "json_extract(data_json, '{json_path}') {sql_op} ?{idx}"
                ));
                match &filter.value {
                    serde_json::Value::String(s) => {
                        bind_values.push(Box::new(s.clone()));
                    }
                    serde_json::Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            bind_values.push(Box::new(i));
                        } else if let Some(f) = n.as_f64() {
                            bind_values.push(Box::new(f));
                        } else {
                            bail!("unsupported number in filter");
                        }
                    }
                    serde_json::Value::Bool(b) => {
                        bind_values.push(Box::new(*b));
                    }
                    serde_json::Value::Null => {
                        bind_values.push(Box::new(rusqlite::types::Null));
                    }
                    _ => bail!("unsupported filter value type"),
                }
            }
        }
    }

    let where_clause = format!(" WHERE {}", clauses.join(" AND "));
    Ok((where_clause, bind_values))
}

// --- MemoryDB collection methods ---

impl MemoryDB {
    pub fn create_collection(
        &self,
        name: &str,
        description: &str,
        schema: &CollectionSchema,
    ) -> Result<()> {
        validate_collection_name(name)?;
        validate_schema(schema)?;

        let conn = self.lock_conn()?;

        // Check collection limit
        let count: u64 =
            conn.query_row("SELECT COUNT(*) FROM collections", [], |row| row.get(0))?;
        if count >= MAX_COLLECTIONS as u64 {
            bail!("maximum of {MAX_COLLECTIONS} collections reached");
        }

        // Check for duplicate name
        let exists: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM collections WHERE name = ?1",
            params![name],
            |row| row.get(0),
        )?;
        if exists {
            bail!("collection '{name}' already exists");
        }

        let schema_json = serde_json::to_string(schema)?;
        conn.execute(
            "INSERT INTO collections (name, description, schema_json) \
             VALUES (?1, ?2, ?3)",
            params![name, description, schema_json],
        )?;

        debug!(
            "created collection '{name}' with {} fields",
            schema.fields.len()
        );
        Ok(())
    }

    pub fn get_collection(&self, name: &str) -> Result<Option<CollectionInfo>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT c.name, c.description, c.schema_json, c.created_at, c.updated_at,
                    (SELECT COUNT(*) FROM collection_records r WHERE r.collection_name = c.name)
             FROM collections c WHERE c.name = ?1",
        )?;

        let mut rows = stmt.query(params![name])?;
        if let Some(row) = rows.next()? {
            let schema_json: String = row.get(2)?;
            let schema: CollectionSchema = serde_json::from_str(&schema_json)?;
            Ok(Some(CollectionInfo {
                name: row.get(0)?,
                description: row.get(1)?,
                schema,
                record_count: row.get(5)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn list_collections(&self) -> Result<Vec<CollectionInfo>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT c.name, c.description, c.schema_json, c.created_at, c.updated_at,
                    (SELECT COUNT(*) FROM collection_records r WHERE r.collection_name = c.name)
             FROM collections c ORDER BY c.name",
        )?;

        let rows = stmt.query_map([], |row| {
            let schema_json: String = row.get(2)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                schema_json,
                row.get::<_, u64>(5)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;

        let mut result = Vec::new();
        for row in rows {
            let (name, description, schema_json, record_count, created_at, updated_at) = row?;
            let schema: CollectionSchema = serde_json::from_str(&schema_json)?;
            result.push(CollectionInfo {
                name,
                description,
                schema,
                record_count,
                created_at,
                updated_at,
            });
        }

        Ok(result)
    }

    pub fn delete_collection(&self, name: &str) -> Result<bool> {
        let conn = self.lock_conn()?;
        let deleted = conn.execute("DELETE FROM collections WHERE name = ?1", params![name])?;
        Ok(deleted > 0)
    }

    pub fn alter_collection_schema(
        &self,
        name: &str,
        add_fields: &[FieldDef],
        remove_fields: &[String],
    ) -> Result<()> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction()?;

        // Load current schema
        let schema_json: String = tx
            .query_row(
                "SELECT schema_json FROM collections WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .map_err(|_| anyhow::anyhow!("collection '{name}' not found"))?;

        let mut schema: CollectionSchema = serde_json::from_str(&schema_json)?;

        // Remove fields
        for field_name in remove_fields {
            let idx = schema
                .fields
                .iter()
                .position(|f| f.name == *field_name)
                .ok_or_else(|| anyhow::anyhow!("field '{field_name}' not found in schema"))?;
            schema.fields.remove(idx);
        }

        // Add fields
        for field in add_fields {
            if schema.fields.iter().any(|f| f.name == field.name) {
                bail!("field '{}' already exists in schema", field.name);
            }
            if field.field_type == FieldType::Enum && field.values.is_empty() {
                bail!("enum field '{}' must have at least one value", field.name);
            }
            schema.fields.push(field.clone());
        }

        // Validate total field count
        if schema.fields.is_empty() {
            bail!("schema must have at least one field after alteration");
        }
        if schema.fields.len() > MAX_FIELDS {
            bail!("schema would exceed {MAX_FIELDS} fields after alteration");
        }

        // Strip removed fields from existing records
        if !remove_fields.is_empty() {
            let mut stmt = tx.prepare(
                "SELECT id, data_json FROM collection_records \
                 WHERE collection_name = ?1",
            )?;
            let records: Vec<(String, String)> = stmt
                .query_map(params![name], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;

            for (id, data_str) in &records {
                let mut data: serde_json::Value = serde_json::from_str(data_str)?;
                if let Some(obj) = data.as_object_mut() {
                    let mut changed = false;
                    for field_name in remove_fields {
                        if obj.remove(field_name).is_some() {
                            changed = true;
                        }
                    }
                    if changed {
                        let new_json = serde_json::to_string(&data)?;
                        tx.execute(
                            "UPDATE collection_records SET data_json = ?1, \
                             updated_at = datetime('now') WHERE id = ?2",
                            params![new_json, id],
                        )?;
                    }
                }
            }
        }

        // Save updated schema
        let new_schema_json = serde_json::to_string(&schema)?;
        tx.execute(
            "UPDATE collections SET schema_json = ?1, updated_at = datetime('now') \
             WHERE name = ?2",
            params![new_schema_json, name],
        )?;

        tx.commit()?;
        Ok(())
    }

    pub fn insert_record(&self, collection: &str, data: serde_json::Value) -> Result<String> {
        let conn = self.lock_conn()?;

        // Load schema
        let schema_json: String = conn
            .query_row(
                "SELECT schema_json FROM collections WHERE name = ?1",
                params![collection],
                |row| row.get(0),
            )
            .map_err(|_| anyhow::anyhow!("collection '{collection}' not found"))?;
        let schema: CollectionSchema = serde_json::from_str(&schema_json)?;

        // Check record limit
        let count: u64 = conn.query_row(
            "SELECT COUNT(*) FROM collection_records WHERE collection_name = ?1",
            params![collection],
            |row| row.get(0),
        )?;
        if count >= MAX_RECORDS {
            bail!("collection '{collection}' has reached the maximum of {MAX_RECORDS} records");
        }

        let validated = validate_record_data(&data, &schema, false)?;
        let id = uuid_v4();
        let data_json = serde_json::to_string(&validated)?;

        conn.execute(
            "INSERT INTO collection_records (id, collection_name, data_json) \
             VALUES (?1, ?2, ?3)",
            params![id, collection, data_json],
        )?;

        Ok(id)
    }

    pub fn query_records(
        &self,
        collection: &str,
        filters: &[RecordFilter],
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<CollectionRecord>> {
        let conn = self.lock_conn()?;

        // Load schema for filter validation
        let schema_json: String = conn
            .query_row(
                "SELECT schema_json FROM collections WHERE name = ?1",
                params![collection],
                |row| row.get(0),
            )
            .map_err(|_| anyhow::anyhow!("collection '{collection}' not found"))?;
        let schema: CollectionSchema = serde_json::from_str(&schema_json)?;

        let (where_clause, mut bind_values) = build_filter_clause(filters, &schema)?;

        let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
        let offset = offset.unwrap_or(0);

        let base_idx = bind_values.len() + 1;

        let sql = if where_clause.is_empty() {
            format!(
                "SELECT id, data_json, created_at, updated_at FROM collection_records \
                 WHERE collection_name = ?{base_idx} \
                 ORDER BY created_at DESC LIMIT ?{} OFFSET ?{}",
                base_idx + 1,
                base_idx + 2
            )
        } else {
            format!(
                "SELECT id, data_json, created_at, updated_at FROM collection_records \
                 WHERE collection_name = ?{base_idx} AND {} \
                 ORDER BY created_at DESC LIMIT ?{} OFFSET ?{}",
                where_clause.trim_start_matches(" WHERE "),
                base_idx + 1,
                base_idx + 2
            )
        };

        bind_values.push(Box::new(collection.to_string()));
        bind_values.push(Box::new(limit));
        bind_values.push(Box::new(offset));

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            bind_values.iter().map(AsRef::as_ref).collect();

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params_ref.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut result = Vec::new();
        for (id, data_json, created_at, updated_at) in rows {
            let data: serde_json::Value = serde_json::from_str(&data_json)?;
            result.push(CollectionRecord {
                id,
                data,
                created_at,
                updated_at,
            });
        }

        Ok(result)
    }

    pub fn update_record(
        &self,
        collection: &str,
        id: &str,
        data: serde_json::Value,
    ) -> Result<bool> {
        let conn = self.lock_conn()?;

        // Load schema
        let schema_json: String = conn
            .query_row(
                "SELECT schema_json FROM collections WHERE name = ?1",
                params![collection],
                |row| row.get(0),
            )
            .map_err(|_| anyhow::anyhow!("collection '{collection}' not found"))?;
        let schema: CollectionSchema = serde_json::from_str(&schema_json)?;

        // Load existing record
        let existing_json: Option<String> = conn
            .query_row(
                "SELECT data_json FROM collection_records \
                 WHERE id = ?1 AND collection_name = ?2",
                params![id, collection],
                |row| row.get(0),
            )
            .ok();

        let Some(existing_json) = existing_json else {
            return Ok(false);
        };

        // Validate the incoming partial data
        let validated_partial = validate_record_data(&data, &schema, true)?;

        // Merge: existing + update
        let mut existing: serde_json::Value = serde_json::from_str(&existing_json)?;
        if let (Some(base), Some(update)) =
            (existing.as_object_mut(), validated_partial.as_object())
        {
            for (k, v) in update {
                base.insert(k.clone(), v.clone());
            }
        }

        let merged_json = serde_json::to_string(&existing)?;
        conn.execute(
            "UPDATE collection_records SET data_json = ?1, \
             updated_at = datetime('now') \
             WHERE id = ?2 AND collection_name = ?3",
            params![merged_json, id, collection],
        )?;

        Ok(true)
    }

    pub fn delete_record(&self, collection: &str, id: &str) -> Result<bool> {
        let conn = self.lock_conn()?;
        let deleted = conn.execute(
            "DELETE FROM collection_records WHERE id = ?1 AND collection_name = ?2",
            params![id, collection],
        )?;
        Ok(deleted > 0)
    }

    pub fn count_records(&self, collection: &str, filters: &[RecordFilter]) -> Result<u64> {
        let conn = self.lock_conn()?;

        // Load schema for filter validation
        let schema_json: String = conn
            .query_row(
                "SELECT schema_json FROM collections WHERE name = ?1",
                params![collection],
                |row| row.get(0),
            )
            .map_err(|_| anyhow::anyhow!("collection '{collection}' not found"))?;
        let schema: CollectionSchema = serde_json::from_str(&schema_json)?;

        let (where_clause, mut bind_values) = build_filter_clause(filters, &schema)?;

        let base_idx = bind_values.len() + 1;

        let sql = if where_clause.is_empty() {
            format!(
                "SELECT COUNT(*) FROM collection_records \
                 WHERE collection_name = ?{base_idx}"
            )
        } else {
            format!(
                "SELECT COUNT(*) FROM collection_records \
                 WHERE collection_name = ?{base_idx} AND {}",
                where_clause.trim_start_matches(" WHERE ")
            )
        };

        bind_values.push(Box::new(collection.to_string()));

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            bind_values.iter().map(AsRef::as_ref).collect();

        let count: u64 = conn.query_row(&sql, params_ref.as_slice(), |row| row.get(0))?;

        Ok(count)
    }

    pub fn aggregate_records(
        &self,
        collection: &str,
        request: &AggregationRequest,
    ) -> Result<Vec<AggregationResult>> {
        let conn = self.lock_conn()?;

        // Load schema
        let schema_json: String = conn
            .query_row(
                "SELECT schema_json FROM collections WHERE name = ?1",
                params![collection],
                |row| row.get(0),
            )
            .map_err(|_| anyhow::anyhow!("collection '{collection}' not found"))?;
        let schema: CollectionSchema = serde_json::from_str(&schema_json)?;

        // Validate field exists
        let field_def = schema
            .fields
            .iter()
            .find(|f| f.name == request.field)
            .ok_or_else(|| {
                anyhow::anyhow!("aggregation field '{}' not in schema", request.field)
            })?;

        // sum/avg only on number fields
        if matches!(request.function, AggFunction::Sum | AggFunction::Avg)
            && field_def.field_type != FieldType::Number
        {
            bail!(
                "{:?} aggregation only works on number fields, '{}' is {:?}",
                request.function,
                request.field,
                field_def.field_type
            );
        }

        // Validate group_by field
        if let Some(ref group_by) = request.group_by
            && !schema.fields.iter().any(|f| f.name == *group_by)
        {
            bail!("group_by field '{group_by}' not in schema");
        }

        let filters = request.filters.as_deref().unwrap_or(&[]);
        let (where_clause, mut bind_values) = build_filter_clause(filters, &schema)?;

        let base_idx = bind_values.len() + 1;
        let field_path = format!("$.{}", request.field);

        let agg_expr = match request.function {
            AggFunction::Count => format!("COUNT(json_extract(data_json, '{field_path}'))"),
            AggFunction::Sum => {
                format!("SUM(CAST(json_extract(data_json, '{field_path}') AS REAL))")
            }
            AggFunction::Avg => {
                format!("AVG(CAST(json_extract(data_json, '{field_path}') AS REAL))")
            }
            AggFunction::Min => format!("MIN(json_extract(data_json, '{field_path}'))"),
            AggFunction::Max => format!("MAX(json_extract(data_json, '{field_path}'))"),
        };

        let (group_select, group_by_clause) = if let Some(ref group_by) = request.group_by {
            let gp = format!("$.{group_by}");
            (
                format!(", json_extract(data_json, '{gp}') AS group_val"),
                format!(" GROUP BY json_extract(data_json, '{gp}')"),
            )
        } else {
            (String::new(), String::new())
        };

        let filter_part = if where_clause.is_empty() {
            format!("WHERE collection_name = ?{base_idx}")
        } else {
            format!(
                "WHERE collection_name = ?{base_idx} AND {}",
                where_clause.trim_start_matches(" WHERE ")
            )
        };

        let sql = format!(
            "SELECT {agg_expr}{group_select} FROM collection_records \
             {filter_part}{group_by_clause}"
        );

        bind_values.push(Box::new(collection.to_string()));

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            bind_values.iter().map(AsRef::as_ref).collect();

        let mut stmt = conn.prepare(&sql)?;

        let mut results = Vec::new();

        if request.group_by.is_some() {
            let rows = stmt.query_map(params_ref.as_slice(), |row| {
                let val: rusqlite::types::Value = row.get(0)?;
                let group: Option<String> = row.get(1)?;
                Ok((val, group))
            })?;

            for row in rows {
                let (val, group) = row?;
                results.push(AggregationResult {
                    value: sqlite_value_to_json(val),
                    group,
                });
            }
        } else {
            let val: rusqlite::types::Value =
                stmt.query_row(params_ref.as_slice(), |row| row.get(0))?;
            results.push(AggregationResult {
                value: sqlite_value_to_json(val),
                group: None,
            });
        }

        Ok(results)
    }
}

fn sqlite_value_to_json(val: rusqlite::types::Value) -> serde_json::Value {
    match val {
        rusqlite::types::Value::Null => serde_json::Value::Null,
        rusqlite::types::Value::Integer(i) => serde_json::json!(i),
        rusqlite::types::Value::Real(f) => serde_json::json!(f),
        rusqlite::types::Value::Text(s) => serde_json::json!(s),
        rusqlite::types::Value::Blob(_) => serde_json::Value::Null,
    }
}

fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let nanos = now.as_nanos();

    // Simple pseudo-random UUID v4 using time + thread id
    let thread_id = std::thread::current().id();
    let thread_hash = format!("{thread_id:?}");
    let mut hasher = sha2::Sha256::new();
    use sha2::Digest;
    hasher.update(nanos.to_le_bytes());
    hasher.update(thread_hash.as_bytes());
    // Add some extra entropy from the stack pointer
    let stack_var = 0u8;
    let ptr = std::ptr::addr_of!(stack_var) as usize;
    hasher.update(ptr.to_le_bytes());
    let hash = hasher.finalize();

    // Format as UUID v4
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        u32::from_le_bytes([hash[0], hash[1], hash[2], hash[3]]),
        u16::from_le_bytes([hash[4], hash[5]]),
        u16::from_le_bytes([hash[6], hash[7]]) & 0x0FFF,
        (u16::from_le_bytes([hash[8], hash[9]]) & 0x3FFF) | 0x8000,
        u64::from_le_bytes([
            hash[10], hash[11], hash[12], hash[13], hash[14], hash[15], 0, 0
        ]) & 0xFFFF_FFFF_FFFF,
    )
}

#[cfg(test)]
mod tests;
