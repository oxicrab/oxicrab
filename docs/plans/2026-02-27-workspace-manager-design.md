# Workspace Manager Design

**Date**: 2026-02-27
**Status**: Approved

## Problem

The `~/.oxicrab/workspace/` directory accumulates files from tool execution, user output, and downloads without organization. Files land wherever the LLM decides, creating a flat mess that's hard to navigate, search, or clean up.

## Solution: Workspace Manager Layer

A `WorkspaceManager` struct that provides auto-routing of files into category/date directories, manifest tracking in SQLite, configurable lifecycle policies, and a workspace tool for the agent to list/search/manage files.

## Directory Structure

Existing directories (`memory/`, `knowledge/`, `skills/`, `sessions/`) are unchanged. New category directories are created on demand:

```
~/.oxicrab/workspace/
├── AGENTS.md, USER.md, TOOLS.md    (unchanged)
├── memory/                          (unchanged)
├── knowledge/                       (unchanged)
├── skills/                          (unchanged)
├── sessions/                        (unchanged)
│
├── code/                            scripts, source files
│   └── 2026-02-27/
│       └── scraper.py
├── documents/                       reports, markdown, text
│   └── 2026-02-27/
│       └── meeting-summary.md
├── data/                            CSVs, JSON, YAML exports
│   └── 2026-02-27/
│       └── users-export.csv
├── images/                          generated/downloaded images
│   └── 2026-02-27/
│       └── chart.png
├── downloads/                       fetched files, API responses
│   └── 2026-02-27/
│       └── report.pdf
└── temp/                            ephemeral tool artifacts
    └── 2026-02-27/
        └── intermediate.json
```

### Category Inference (by extension)

- **code**: `.py`, `.rs`, `.js`, `.ts`, `.sh`, `.rb`, `.go`, `.java`, `.c`, `.cpp`, `.html`, `.css`, `.sql`
- **documents**: `.md`, `.txt`, `.doc`, `.docx`, `.rtf`
- **data**: `.csv`, `.json`, `.yaml`, `.yml`, `.xml`, `.toml`, `.parquet`
- **images**: `.png`, `.jpg`, `.jpeg`, `.gif`, `.svg`, `.webp`, `.bmp`
- **downloads**: `.pdf`, `.zip`, `.tar`, `.gz`, `.epub`
- **temp**: no extension, unknown extension, or explicitly-marked temporary files

Date subdirectories use `YYYY-MM-DD` format (UTC).

## Manifest Tracking

New table in existing `memory.sqlite3`:

```sql
CREATE TABLE IF NOT EXISTS workspace_files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL UNIQUE,          -- relative to workspace root
    category TEXT NOT NULL,             -- code, documents, data, images, downloads, temp
    original_name TEXT,                 -- name as specified by agent/user
    size_bytes INTEGER NOT NULL,
    source_tool TEXT,                   -- which tool created it
    tags TEXT DEFAULT '',               -- comma-separated
    created_at TEXT NOT NULL,           -- ISO 8601
    accessed_at TEXT,                   -- last read via read_file
    session_key TEXT                    -- which conversation created this
);
```

Only files in the new category directories are tracked. Existing managed directories (`memory/`, `knowledge/`, `sessions/`, `skills/`) have their own lifecycle systems.

The manifest is an index — filesystem is source of truth. Stale entries (file deleted outside oxicrab) are cleaned up during manifest sync.

## WorkspaceManager Struct

```rust
pub struct WorkspaceManager {
    workspace_root: PathBuf,
    db: Arc<MemoryDB>,
}
```

### Methods

- `resolve_path(filename, category_hint) -> PathBuf` — determines full path from extension inference + optional category override; creates date subdirectory
- `register_file(path, source_tool, session_key)` — adds to manifest after write
- `unregister_file(path)` — removes manifest entry (called when file deleted)
- `list_files(category, date_range, tags) -> Vec<FileEntry>` — query manifest
- `search_files(query) -> Vec<FileEntry>` — search by name/tags
- `remove_file(path)` — delete file + manifest entry
- `cleanup_expired()` — enforces per-category TTL policies
- `sync_manifest()` — reconciles manifest with filesystem

## Integration Points

### WriteFileTool

Gets an optional `WorkspaceManager`. When writing to workspace without a specific subdirectory path, auto-routes via `resolve_path()`. Explicit paths are respected as-is. After every workspace write, calls `register_file()`.

### ReadFileTool

Updates `accessed_at` in manifest when reading a tracked file.

### Other file-producing tools

Browser screenshots, image generation, etc. — same pattern: route through workspace manager.

### Hygiene system

Extended to call `cleanup_expired()` alongside existing memory archive/purge.

## Workspace Tool

New action-based tool called `workspace`:

| Action    | Description                              | Read-only |
|-----------|------------------------------------------|-----------|
| `list`    | List files, filter by category/date/tags | Yes       |
| `search`  | Search files by name pattern             | Yes       |
| `info`    | Get details about a specific file        | Yes       |
| `move`    | Move a file to a different category      | No        |
| `delete`  | Delete a file and its manifest entry     | No        |
| `tag`     | Add/remove tags on a file                | No        |
| `cleanup` | Trigger manual cleanup of expired files  | No        |
| `tree`    | Show directory tree of workspace         | Yes       |

Capabilities: `built_in: true`, `network_outbound: false`, `subagent_access: ReadOnly`.

`ReadOnlyToolWrapper` exposes: `list`, `search`, `info`, `tree`.

## Lifecycle Policies

Configurable per-category TTLs in config:

```json
{
  "agents": {
    "defaults": {
      "workspace": {
        "ttl": {
          "temp": 7,
          "downloads": 30,
          "images": 90,
          "code": null,
          "documents": null,
          "data": null
        }
      }
    }
  }
}
```

`null` = never expire. Cleanup runs during hygiene cycle.

## Configuration

New fields in agent config:

```rust
// In workspace config (new struct)
pub struct WorkspaceConfig {
    pub path: PathBuf,              // existing field, moved here
    pub ttl: HashMap<String, Option<u64>>,  // category -> days (None = no expiry)
}
```

Defaults match the table above.

## Backward Compatibility

- Existing files in workspace root are not moved automatically
- The `sync_manifest()` method can discover and register existing files on first run
- Tools continue to work without the workspace manager (it's optional)
- No changes to memory/, knowledge/, sessions/, skills/ behavior
