# Design: Structured Collections

## Problem

LLMs are bad at mutating structured data stored as free-text. IronClaw's benchmarks
(PR #1937) show 70-76% task success for typed CRUD collections vs 26-37% for raw
memory documents on identical tasks (grocery lists, todos, time tracking, transactions).

The failure mode: "add milk to the grocery list" causes the LLM to either create a
duplicate document (fragmenting data) or attempt read-modify-write on text (losing items,
duplicating entries, mangling formatting). Append-only text storage doesn't support mutation.

Oxicrab's memory system stores everything as `memory_entries` rows keyed by `source_key`.
There is no structured, queryable, mutable data store. Users who want to track lists,
inventories, budgets, or any tabular data hit the same failure mode.

## Proposal

Add **typed CRUD collections** — user-defined schemas with auto-generated per-collection
tools that provide add/query/update/delete/count operations on structured records.

## Architecture

### Storage: Two new SQLite tables in MemoryDB

```sql
CREATE TABLE IF NOT EXISTS collections (
    name         TEXT PRIMARY KEY,
    description  TEXT NOT NULL DEFAULT '',
    schema_json  TEXT NOT NULL,         -- JSON: field definitions
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS collection_records (
    id              TEXT PRIMARY KEY,   -- uuid
    collection_name TEXT NOT NULL REFERENCES collections(name) ON DELETE CASCADE,
    data_json       TEXT NOT NULL,      -- JSON object matching schema
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_collection_records_name ON collection_records(collection_name);
```

### Schema format

```json
{
  "fields": [
    { "name": "item", "type": "text", "required": true },
    { "name": "quantity", "type": "number", "required": false },
    { "name": "bought", "type": "bool", "required": false },
    { "name": "category", "type": "enum", "values": ["produce", "dairy", "meat", "other"] }
  ]
}
```

Four field types: `text`, `number`, `bool`, `enum`. Enum requires a `values` array.
All fields nullable unless `required: true`. No date/datetime initially — LLM date
parsing is a rabbit hole; add it in Phase 2 if needed.

### Tool architecture

**Two tools:**

1. **`collections`** — management tool (always registered)
   - `create`: name, description, fields → creates collection + registers data tool
   - `list`: → lists all collections with field summaries
   - `delete`: name → drops collection and all records
   - `describe`: name → shows schema and record count

2. **`{collection_name}`** — one per collection (registered as deferred)
   - `add`: record data → insert, returns ID
   - `query`: filters, limit, offset → matching records
   - `update`: id, fields to update → partial update
   - `delete`: id → remove record
   - `count`: optional filters → count matching records

Per-collection tools are registered as deferred via `register_deferred()` and
discovered through `tool_search`. This avoids polluting the LLM's tool list for
users who don't use collections, while ensuring the tool is found when relevant.

IronClaw's benchmarks show per-collection tools (76%) significantly outperform a
single generic CRUD tool (41%). The `tool_search` mechanism bridges the discovery
gap — when a user says "add milk to my grocery list," the LLM searches for
"grocery" and finds the `grocery_list` tool.

### Filter system

Filters are JSON objects in query/count/update/delete params:

```json
{
  "filters": [
    { "field": "category", "op": "eq", "value": "produce" },
    { "field": "quantity", "op": "gt", "value": 5 }
  ]
}
```

Operators: `eq`, `neq`, `gt`, `gte`, `lt`, `lte`, `contains` (text substring).
Filters are AND-combined. Each filter is validated against the schema (field exists,
op is valid for the type). Translated to SQL WHERE clauses using `json_extract()`.

### Dynamic tool registration

Collections are loaded from SQLite at startup. For each collection, a
`CollectionDataTool` is constructed and registered as deferred.

When a collection is created at runtime:
1. Insert into `collections` table
2. Construct `CollectionDataTool` with the schema
3. Register as deferred in `ToolRegistry`
4. Add to `tool_search` index

This requires `ToolRegistry` to support post-startup registration. Today,
`register_deferred()` is called during `register_all_tools()`. We need to add
a `register_deferred_runtime()` method that also updates the `tool_search` index.
The `ToolSearchTool` holds a `Vec<ToolIndexEntry>` — this needs to become a
shared `Arc<Mutex<Vec<ToolIndexEntry>>>` so runtime additions are visible.

### Type coercion

Reuse `coerce_params_to_schema()` from `src/agent/tools/registry/mod.rs` for
record data. Additionally, collection-specific coercion:
- `"true"`/`"false"` → bool
- `"5"` → number
- Enum values: case-insensitive match against allowed values

### Validation

Before insert/update:
- Required fields present (for insert; updates are partial)
- Values match declared types
- Enum values in the allowed set
- Collection name: alphanumeric + underscore, 1-64 chars (must be valid tool name)
- Max 50 collections (prevent unbounded tool registration)
- Max 10 fields per collection
- Max 10,000 records per collection

## What it replaces

### Partially replaces: memory-based list tracking

Today, users who say "remember my grocery list: milk, eggs, bread" get this stored
as a `daily:` memory entry. Retrieval via `memory_search` works, but mutation
("remove milk") fails because the LLM has to rewrite the text entry.

With collections, the LLM would instead:
1. Create a `grocery_list` collection with fields `item: text, bought: bool`
2. Add records for each item
3. Handle "remove milk" with a `delete` operation

**Memory entries are NOT removed.** Collections complement memory, they don't replace it.
Memory remains the right store for unstructured notes, facts, preferences, and context.
Collections are for structured, mutable, queryable data.

### Does NOT replace: workspace files

Workspace files (`WorkspaceManager`) handle file I/O — code, documents, data files.
Collections handle in-database structured records. Different use cases, no overlap.

### Does NOT replace: knowledge entries

Knowledge entries (`knowledge:` prefix) are curated reference content exempt from
purge. Collections are user-generated mutable data. Different lifecycle.

## Pros

1. **Dramatic accuracy improvement**: 70-76% vs 26-37% on structured data tasks
   (IronClaw's benchmarks, validated across 28 scenarios and 2 models)
2. **Clean CRUD semantics**: LLMs understand add/query/update/delete far better
   than read-modify-write on text
3. **Queryable**: Filters enable "show me all items over $50" without LLM
   text parsing
4. **Builds on existing infrastructure**: SQLite (MemoryDB), deferred tool
   registration (tool_search), param coercion, action-based tool pattern
5. **Single-user simplicity**: No user_id scoping, no RBAC, no multi-tenant
   complexity (unlike IronClaw's 14K-line implementation)
6. **Discoverable**: Deferred registration + tool_search means zero token cost
   for users who don't use collections

## Cons

1. **Dynamic tool registration is new territory**: `ToolRegistry` currently only
   registers tools at startup. Runtime registration needs a shared mutable index,
   which adds a lock. Low risk but new pattern.
2. **Schema rigidity**: No schema migration in Phase 1 (delete and recreate).
   Users who want to add a field to an existing collection lose all records.
3. **No full-text search on records**: Queries use exact/comparison filters, not
   fuzzy search. For "find that thing I bought last week," memory_search is still
   better.
4. **Persistence across restarts**: Collection tools need to be re-registered
   from SQLite on every startup. Adds to startup time (negligible for <50
   collections).
5. **LLM must decide when to use collections vs memory**: Ambiguous requests
   like "remember that I need milk" could go either way. The system prompt
   will need guidance on when to prefer collections over memory.
6. **No embeddings/semantic search on records**: Collection queries are
   structural (field-based filters). Can't do "find similar items."

## Estimated scope

~2,000 lines across:
- `crates/oxicrab-memory/src/memory_db/collections.rs` (~400 lines) — SQLite CRUD
- `src/agent/tools/collections/mod.rs` (~300 lines) — `CollectionsTool` (management)
- `src/agent/tools/collections/data_tool.rs` (~500 lines) — `CollectionDataTool`
  (per-collection CRUD, dynamically instantiated)
- `src/agent/tools/setup/mod.rs` (~50 lines) — registration, startup loading
- `src/agent/tools/registry/mod.rs` (~50 lines) — `register_deferred_runtime()`
- `src/agent/tools/tool_search/mod.rs` (~30 lines) — shared mutable index
- Tests (~500 lines)
- Migration SQL (~20 lines)

## Phase 2 (future, if Phase 1 proves useful)

- Date/datetime fields with LLM-friendly parsing ("tomorrow", "next Friday")
- Aggregations: sum, avg, min, max, group_by
- Auto-generated skill docs per collection (IronClaw showed +8% accuracy)
- Schema alteration: add/remove fields without data loss
- Import/export: CSV, JSON
- Collection templates: pre-built schemas for common use cases (todo list,
  expense tracker, reading list)
