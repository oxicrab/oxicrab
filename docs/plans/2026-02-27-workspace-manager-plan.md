# Workspace Manager Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add an organized file management layer to the workspace with auto-routing, manifest tracking, lifecycle policies, and an agent-facing workspace tool.

**Architecture:** A `WorkspaceManager` in `src/agent/workspace/` provides category inference (by extension), date-based subdirectory routing, and SQLite manifest tracking in the existing `memory.sqlite3`. A new `workspace` action-based tool exposes list/search/manage operations. Hygiene system is extended to clean expired files. WriteFileTool auto-routes files through the manager.

**Tech Stack:** Rust, SQLite (rusqlite), chrono, serde, existing MemoryDB

**Design doc:** `docs/plans/2026-02-27-workspace-manager-design.md`

---

### Task 1: WorkspaceManager core module — category inference + path resolution

**Files:**
- Create: `src/agent/workspace/mod.rs`
- Create: `src/agent/workspace/tests.rs`
- Modify: `src/agent/mod.rs` — add `pub mod workspace;`

This is the foundation. The `WorkspaceManager` struct holds the workspace root path and a reference to `MemoryDB`. Category inference maps file extensions to one of 6 categories. Path resolution builds `{category}/{YYYY-MM-DD}/{filename}`.

**Step 1: Write the failing tests**

Create `src/agent/workspace/tests.rs`:

```rust
use super::*;
use std::path::Path;

#[test]
fn test_infer_category_code() {
    assert_eq!(infer_category(Path::new("script.py")), FileCategory::Code);
    assert_eq!(infer_category(Path::new("main.rs")), FileCategory::Code);
    assert_eq!(infer_category(Path::new("app.js")), FileCategory::Code);
    assert_eq!(infer_category(Path::new("style.css")), FileCategory::Code);
}

#[test]
fn test_infer_category_documents() {
    assert_eq!(infer_category(Path::new("report.md")), FileCategory::Documents);
    assert_eq!(infer_category(Path::new("notes.txt")), FileCategory::Documents);
}

#[test]
fn test_infer_category_data() {
    assert_eq!(infer_category(Path::new("users.csv")), FileCategory::Data);
    assert_eq!(infer_category(Path::new("config.json")), FileCategory::Data);
    assert_eq!(infer_category(Path::new("data.yaml")), FileCategory::Data);
}

#[test]
fn test_infer_category_images() {
    assert_eq!(infer_category(Path::new("photo.png")), FileCategory::Images);
    assert_eq!(infer_category(Path::new("diagram.svg")), FileCategory::Images);
}

#[test]
fn test_infer_category_downloads() {
    assert_eq!(infer_category(Path::new("manual.pdf")), FileCategory::Downloads);
    assert_eq!(infer_category(Path::new("archive.zip")), FileCategory::Downloads);
}

#[test]
fn test_infer_category_temp_for_unknown() {
    assert_eq!(infer_category(Path::new("noext")), FileCategory::Temp);
    assert_eq!(infer_category(Path::new("file.xyz")), FileCategory::Temp);
}

#[test]
fn test_category_as_str_round_trip() {
    for cat in &[
        FileCategory::Code,
        FileCategory::Documents,
        FileCategory::Data,
        FileCategory::Images,
        FileCategory::Downloads,
        FileCategory::Temp,
    ] {
        assert_eq!(FileCategory::from_str(cat.as_str()), Some(*cat));
    }
    assert_eq!(FileCategory::from_str("bogus"), None);
}

#[test]
fn test_resolve_path_creates_date_subdir() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = WorkspaceManager::new(tmp.path().to_path_buf(), None);
    let resolved = mgr.resolve_path("script.py", None);

    // Should be {workspace}/code/{today}/script.py
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let expected = tmp.path().join("code").join(&today).join("script.py");
    assert_eq!(resolved, expected);
}

#[test]
fn test_resolve_path_respects_category_hint() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = WorkspaceManager::new(tmp.path().to_path_buf(), None);
    let resolved = mgr.resolve_path("output.json", Some(FileCategory::Temp));

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let expected = tmp.path().join("temp").join(&today).join("output.json");
    assert_eq!(resolved, expected);
}

#[test]
fn test_is_managed_category_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = WorkspaceManager::new(tmp.path().to_path_buf(), None);

    // Category dirs are managed
    assert!(mgr.is_managed_path(&tmp.path().join("code/2026-02-27/foo.py")));
    assert!(mgr.is_managed_path(&tmp.path().join("temp/2026-02-27/bar")));

    // Existing system dirs are NOT managed by workspace manager
    assert!(!mgr.is_managed_path(&tmp.path().join("memory/MEMORY.md")));
    assert!(!mgr.is_managed_path(&tmp.path().join("sessions/key.jsonl")));
    assert!(!mgr.is_managed_path(&tmp.path().join("knowledge/faq.md")));
    assert!(!mgr.is_managed_path(&tmp.path().join("skills/foo/SKILL.md")));

    // Root-level bootstrap files are NOT managed
    assert!(!mgr.is_managed_path(&tmp.path().join("AGENTS.md")));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib test_infer_category`
Expected: Compilation error — `workspace` module doesn't exist yet

**Step 3: Write minimal implementation**

Create `src/agent/workspace/mod.rs`:

```rust
use crate::agent::memory::memory_db::MemoryDB;
use chrono::Utc;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(test)]
mod tests;

/// File categories for workspace organization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileCategory {
    Code,
    Documents,
    Data,
    Images,
    Downloads,
    Temp,
}

impl FileCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Documents => "documents",
            Self::Data => "data",
            Self::Images => "images",
            Self::Downloads => "downloads",
            Self::Temp => "temp",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "code" => Some(Self::Code),
            "documents" => Some(Self::Documents),
            "data" => Some(Self::Data),
            "images" => Some(Self::Images),
            "downloads" => Some(Self::Downloads),
            "temp" => Some(Self::Temp),
            _ => None,
        }
    }

    pub const ALL: &[FileCategory] = &[
        Self::Code,
        Self::Documents,
        Self::Data,
        Self::Images,
        Self::Downloads,
        Self::Temp,
    ];
}

/// Directories that are managed by other systems and should NOT be touched.
const RESERVED_DIRS: &[&str] = &["memory", "knowledge", "skills", "sessions"];

/// Infer file category from extension.
pub fn infer_category(path: &Path) -> FileCategory {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "py" | "rs" | "js" | "ts" | "tsx" | "jsx" | "sh" | "bash" | "rb" | "go" | "java"
        | "c" | "cpp" | "h" | "hpp" | "html" | "css" | "sql" | "lua" | "php" | "swift"
        | "kt" | "scala" | "r" | "pl" | "zig" | "nim" | "ex" | "exs" | "erl" => {
            FileCategory::Code
        }
        "md" | "txt" | "doc" | "docx" | "rtf" | "org" | "rst" | "adoc" | "tex" | "log" => {
            FileCategory::Documents
        }
        "csv" | "json" | "yaml" | "yml" | "xml" | "toml" | "parquet" | "tsv" | "ndjson"
        | "jsonl" | "sqlite" | "sqlite3" | "db" => FileCategory::Data,
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "bmp" | "ico" | "tiff" | "tif"
        | "avif" | "heic" => FileCategory::Images,
        "pdf" | "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "epub" | "mobi"
        | "whl" | "deb" | "rpm" | "dmg" | "iso" | "apk" => FileCategory::Downloads,
        _ => FileCategory::Temp,
    }
}

/// Manages workspace file organization, manifest tracking, and lifecycle.
pub struct WorkspaceManager {
    workspace_root: PathBuf,
    db: Option<Arc<MemoryDB>>,
}

impl WorkspaceManager {
    pub fn new(workspace_root: PathBuf, db: Option<Arc<MemoryDB>>) -> Self {
        Self {
            workspace_root,
            db,
        }
    }

    /// Resolve a filename to a full organized path: `{workspace}/{category}/{YYYY-MM-DD}/{filename}`
    ///
    /// If `category_hint` is provided, uses that instead of inferring from extension.
    pub fn resolve_path(&self, filename: &str, category_hint: Option<FileCategory>) -> PathBuf {
        let path = Path::new(filename);
        let category = category_hint.unwrap_or_else(|| infer_category(path));
        let date = Utc::now().format("%Y-%m-%d").to_string();
        self.workspace_root
            .join(category.as_str())
            .join(&date)
            .join(filename)
    }

    /// Check whether a path falls under a workspace-manager-managed category directory.
    /// Returns false for reserved system directories (memory, knowledge, etc.) and root files.
    pub fn is_managed_path(&self, path: &Path) -> bool {
        let relative = match path.strip_prefix(&self.workspace_root) {
            Ok(r) => r,
            Err(_) => return false,
        };

        let first_component = relative
            .components()
            .next()
            .and_then(|c| c.as_os_str().to_str());

        match first_component {
            Some(dir) => {
                // Must not be a reserved system directory
                if RESERVED_DIRS.contains(&dir) {
                    return false;
                }
                // Must be a known category directory
                FileCategory::from_str(dir).is_some()
            }
            None => false,
        }
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }
}
```

Add `pub mod workspace;` to `src/agent/mod.rs`.

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib test_infer_category test_category_as_str test_resolve_path test_is_managed`
Expected: All PASS

**Step 5: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: No warnings

**Step 6: Commit**

```bash
git add src/agent/workspace/ src/agent/mod.rs
git commit -m "feat(workspace): add WorkspaceManager with category inference and path resolution"
```

---

### Task 2: SQLite manifest table + register/query methods on MemoryDB

**Files:**
- Modify: `src/agent/memory/memory_db/mod.rs` — add `workspace_files` table + CRUD methods
- Create or modify: test file for the new methods

Add the manifest table to `ensure_schema()` and implement the data access methods on `MemoryDB`.

**Step 1: Write the failing tests**

Add to the MemoryDB test file (or inline if small enough):

```rust
#[test]
fn test_workspace_file_register_and_list() {
    let tmp = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(tmp.path().join("test.sqlite3")).unwrap();

    db.register_workspace_file(
        "code/2026-02-27/script.py",
        "code",
        Some("script.py"),
        1234,
        Some("write_file"),
        None, // session_key
    )
    .unwrap();

    let files = db.list_workspace_files(Some("code"), None, None).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "code/2026-02-27/script.py");
    assert_eq!(files[0].category, "code");
    assert_eq!(files[0].size_bytes, 1234);
}

#[test]
fn test_workspace_file_search_by_name() {
    let tmp = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(tmp.path().join("test.sqlite3")).unwrap();

    db.register_workspace_file("code/2026-02-27/scraper.py", "code", Some("scraper.py"), 100, Some("write_file"), None).unwrap();
    db.register_workspace_file("data/2026-02-27/users.csv", "data", Some("users.csv"), 200, Some("write_file"), None).unwrap();

    let results = db.search_workspace_files("scraper").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].path, "code/2026-02-27/scraper.py");
}

#[test]
fn test_workspace_file_unregister() {
    let tmp = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(tmp.path().join("test.sqlite3")).unwrap();

    db.register_workspace_file("temp/2026-02-27/scratch.txt", "temp", Some("scratch.txt"), 50, None, None).unwrap();
    assert_eq!(db.list_workspace_files(None, None, None).unwrap().len(), 1);

    db.unregister_workspace_file("temp/2026-02-27/scratch.txt").unwrap();
    assert_eq!(db.list_workspace_files(None, None, None).unwrap().len(), 0);
}

#[test]
fn test_workspace_file_update_accessed_at() {
    let tmp = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(tmp.path().join("test.sqlite3")).unwrap();

    db.register_workspace_file("code/2026-02-27/foo.rs", "code", Some("foo.rs"), 100, None, None).unwrap();

    let before = db.list_workspace_files(None, None, None).unwrap();
    assert!(before[0].accessed_at.is_none());

    db.touch_workspace_file("code/2026-02-27/foo.rs").unwrap();

    let after = db.list_workspace_files(None, None, None).unwrap();
    assert!(after[0].accessed_at.is_some());
}

#[test]
fn test_workspace_file_update_tags() {
    let tmp = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(tmp.path().join("test.sqlite3")).unwrap();

    db.register_workspace_file("code/2026-02-27/foo.rs", "code", Some("foo.rs"), 100, None, None).unwrap();
    db.set_workspace_file_tags("code/2026-02-27/foo.rs", "important,review").unwrap();

    let files = db.list_workspace_files(None, None, Some("important")).unwrap();
    assert_eq!(files.len(), 1);
}

#[test]
fn test_workspace_cleanup_expired_files() {
    let tmp = tempfile::tempdir().unwrap();
    let db = MemoryDB::new(tmp.path().join("test.sqlite3")).unwrap();

    // Insert a file with a created_at 10 days ago
    let old_date = (chrono::Utc::now() - chrono::Duration::days(10))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    db.register_workspace_file_with_date("temp/2026-02-17/old.txt", "temp", Some("old.txt"), 50, None, None, &old_date).unwrap();

    let expired = db.list_expired_workspace_files("temp", 7).unwrap();
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].path, "temp/2026-02-17/old.txt");
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib test_workspace_file`
Expected: Compilation error — methods don't exist yet

**Step 3: Implement the manifest table and methods**

In `src/agent/memory/memory_db/mod.rs`:

1. Add the `workspace_files` table to `ensure_schema()` (after the `intent_metrics` table, around line 286):

```rust
conn.execute(
    "CREATE TABLE IF NOT EXISTS workspace_files (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        path TEXT NOT NULL UNIQUE,
        category TEXT NOT NULL,
        original_name TEXT,
        size_bytes INTEGER NOT NULL,
        source_tool TEXT,
        tags TEXT NOT NULL DEFAULT '',
        created_at TEXT NOT NULL,
        accessed_at TEXT,
        session_key TEXT
    )",
    [],
)?;

conn.execute_batch(
    "CREATE INDEX IF NOT EXISTS idx_workspace_files_category ON workspace_files(category);
     CREATE INDEX IF NOT EXISTS idx_workspace_files_created ON workspace_files(created_at);",
)?;
```

2. Add a `WorkspaceFileEntry` struct (near the other data structs at the top of the file):

```rust
#[derive(Debug, Clone)]
pub struct WorkspaceFileEntry {
    pub id: i64,
    pub path: String,
    pub category: String,
    pub original_name: Option<String>,
    pub size_bytes: i64,
    pub source_tool: Option<String>,
    pub tags: String,
    pub created_at: String,
    pub accessed_at: Option<String>,
    pub session_key: Option<String>,
}
```

3. Implement methods on `MemoryDB` (add a new impl block or extend the existing one):

- `register_workspace_file(path, category, original_name, size_bytes, source_tool, session_key) -> Result<()>` — INSERT OR REPLACE
- `register_workspace_file_with_date(...)` — same but with explicit `created_at` (for testing)
- `unregister_workspace_file(path) -> Result<()>` — DELETE
- `list_workspace_files(category: Option<&str>, date: Option<&str>, tag: Option<&str>) -> Result<Vec<WorkspaceFileEntry>>` — SELECT with optional WHERE clauses
- `search_workspace_files(query: &str) -> Result<Vec<WorkspaceFileEntry>>` — WHERE path LIKE or original_name LIKE
- `touch_workspace_file(path: &str) -> Result<()>` — UPDATE accessed_at = datetime('now')
- `set_workspace_file_tags(path: &str, tags: &str) -> Result<()>` — UPDATE tags
- `list_expired_workspace_files(category: &str, ttl_days: u32) -> Result<Vec<WorkspaceFileEntry>>` — WHERE category = ? AND created_at < datetime('now', '-N days')
- `move_workspace_file(old_path: &str, new_path: &str, new_category: &str) -> Result<()>` — UPDATE path and category

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib test_workspace_file`
Expected: All PASS

**Step 5: Commit**

```bash
git add src/agent/memory/memory_db/
git commit -m "feat(workspace): add workspace_files manifest table and CRUD methods to MemoryDB"
```

---

### Task 3: WorkspaceManager — manifest integration methods

**Files:**
- Modify: `src/agent/workspace/mod.rs` — add register_file, list_files, search_files, remove_file, cleanup_expired, sync_manifest
- Modify: `src/agent/workspace/tests.rs` — add tests

Now wire the `WorkspaceManager` methods to the `MemoryDB` methods. This is where the manager becomes useful — it handles the filesystem operations AND updates the manifest in one call.

**Step 1: Write the failing tests**

Add to `src/agent/workspace/tests.rs`:

```rust
#[test]
fn test_register_file_adds_to_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Arc::new(MemoryDB::new(tmp.path().join("memory/memory.sqlite3")).unwrap());
    let mgr = WorkspaceManager::new(tmp.path().to_path_buf(), Some(db.clone()));

    // Create a file in the workspace
    let file_path = tmp.path().join("code/2026-02-27/test.py");
    std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    std::fs::write(&file_path, "print('hello')").unwrap();

    mgr.register_file(&file_path, Some("write_file"), None).unwrap();

    let files = mgr.list_files(Some(FileCategory::Code), None, None).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].original_name.as_deref(), Some("test.py"));
}

#[test]
fn test_remove_file_deletes_file_and_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Arc::new(MemoryDB::new(tmp.path().join("memory/memory.sqlite3")).unwrap());
    let mgr = WorkspaceManager::new(tmp.path().to_path_buf(), Some(db.clone()));

    let file_path = tmp.path().join("temp/2026-02-27/scratch.txt");
    std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    std::fs::write(&file_path, "scratch").unwrap();
    mgr.register_file(&file_path, None, None).unwrap();

    mgr.remove_file(&file_path).unwrap();

    assert!(!file_path.exists());
    assert!(mgr.list_files(None, None, None).unwrap().is_empty());
}

#[test]
fn test_sync_manifest_removes_stale_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Arc::new(MemoryDB::new(tmp.path().join("memory/memory.sqlite3")).unwrap());
    let mgr = WorkspaceManager::new(tmp.path().to_path_buf(), Some(db.clone()));

    // Register a file that doesn't actually exist on disk
    db.register_workspace_file("code/2026-02-27/ghost.py", "code", Some("ghost.py"), 0, None, None).unwrap();

    let (removed, _discovered) = mgr.sync_manifest().unwrap();
    assert_eq!(removed, 1);
    assert!(mgr.list_files(None, None, None).unwrap().is_empty());
}

#[test]
fn test_sync_manifest_discovers_untracked_files() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Arc::new(MemoryDB::new(tmp.path().join("memory/memory.sqlite3")).unwrap());
    let mgr = WorkspaceManager::new(tmp.path().to_path_buf(), Some(db.clone()));

    // Create a file on disk that is NOT in the manifest
    let file_path = tmp.path().join("data/2026-02-27/found.csv");
    std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    std::fs::write(&file_path, "a,b,c").unwrap();

    let (_removed, discovered) = mgr.sync_manifest().unwrap();
    assert_eq!(discovered, 1);

    let files = mgr.list_files(Some(FileCategory::Data), None, None).unwrap();
    assert_eq!(files.len(), 1);
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib test_register_file test_remove_file test_sync_manifest`
Expected: Compilation error — methods not implemented

**Step 3: Implement manifest integration methods**

Add to `WorkspaceManager` in `src/agent/workspace/mod.rs`:

- `register_file(&self, abs_path: &Path, source_tool: Option<&str>, session_key: Option<&str>) -> Result<()>` — compute relative path from workspace root, get file size, extract filename, determine category, call `db.register_workspace_file()`
- `list_files(&self, category: Option<FileCategory>, date: Option<&str>, tag: Option<&str>) -> Result<Vec<WorkspaceFileEntry>>` — delegate to `db.list_workspace_files()`
- `search_files(&self, query: &str) -> Result<Vec<WorkspaceFileEntry>>` — delegate to `db.search_workspace_files()`
- `remove_file(&self, abs_path: &Path) -> Result<()>` — delete file from disk + `db.unregister_workspace_file()`
- `move_file(&self, abs_path: &Path, new_category: FileCategory) -> Result<PathBuf>` — physically move file, update manifest
- `tag_file(&self, abs_path: &Path, tags: &str) -> Result<()>` — delegate to `db.set_workspace_file_tags()`
- `touch_file(&self, abs_path: &Path) -> Result<()>` — delegate to `db.touch_workspace_file()`
- `cleanup_expired(&self, ttl_map: &HashMap<String, Option<u64>>) -> Result<u32>` — for each category with a TTL, list expired files, delete them from disk, unregister from manifest
- `sync_manifest(&self) -> Result<(u32, u32)>` — walk category dirs, compare with manifest. Remove stale entries (in manifest but not on disk). Register discovered files (on disk but not in manifest). Return `(removed, discovered)`.

All methods that access the DB should check `self.db.is_some()` and return `Ok(())` / empty results if no DB is available (graceful degradation).

**Step 4: Run tests to verify they pass**

Run: `cargo test --lib test_register_file test_remove_file test_sync_manifest`
Expected: All PASS

**Step 5: Commit**

```bash
git add src/agent/workspace/
git commit -m "feat(workspace): add manifest integration methods to WorkspaceManager"
```

---

### Task 4: Config schema — WorkspaceTtlConfig

**Files:**
- Modify: `src/config/schema/agent.rs` — add `WorkspaceTtlConfig` struct + field on `AgentDefaults`
- Modify: `src/config/schema/agent.rs` — update `Default` impl for `AgentDefaults`
- Modify: `src/config/schema/tests.rs` — update `credential_overlays()` if needed, verify config example test

**Step 1: Write the failing test**

The existing `test_config_example_is_up_to_date` will fail once we add the new config field without updating `config.example.json`. But first, add a unit test:

```rust
#[test]
fn test_workspace_ttl_defaults() {
    let ttl = WorkspaceTtlConfig::default();
    assert_eq!(ttl.temp, Some(7));
    assert_eq!(ttl.downloads, Some(30));
    assert_eq!(ttl.images, Some(90));
    assert_eq!(ttl.code, None);
    assert_eq!(ttl.documents, None);
    assert_eq!(ttl.data, None);
}
```

**Step 2: Implement the config struct**

In `src/config/schema/agent.rs`, add:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceTtlConfig {
    #[serde(default = "default_ttl_temp")]
    pub temp: Option<u64>,
    #[serde(default = "default_ttl_downloads")]
    pub downloads: Option<u64>,
    #[serde(default = "default_ttl_images")]
    pub images: Option<u64>,
    #[serde(default)]
    pub code: Option<u64>,
    #[serde(default)]
    pub documents: Option<u64>,
    #[serde(default)]
    pub data: Option<u64>,
}

fn default_ttl_temp() -> Option<u64> { Some(7) }
fn default_ttl_downloads() -> Option<u64> { Some(30) }
fn default_ttl_images() -> Option<u64> { Some(90) }

impl Default for WorkspaceTtlConfig {
    fn default() -> Self {
        Self {
            temp: default_ttl_temp(),
            downloads: default_ttl_downloads(),
            images: default_ttl_images(),
            code: None,
            documents: None,
            data: None,
        }
    }
}

impl WorkspaceTtlConfig {
    /// Convert to a HashMap for use by WorkspaceManager.cleanup_expired()
    pub fn to_map(&self) -> std::collections::HashMap<String, Option<u64>> {
        let mut map = std::collections::HashMap::new();
        map.insert("temp".into(), self.temp);
        map.insert("downloads".into(), self.downloads);
        map.insert("images".into(), self.images);
        map.insert("code".into(), self.code);
        map.insert("documents".into(), self.documents);
        map.insert("data".into(), self.data);
        map
    }
}
```

Add to `AgentDefaults`:

```rust
#[serde(default, rename = "workspaceTtl")]
pub workspace_ttl: WorkspaceTtlConfig,
```

And update `Default::default()` for `AgentDefaults` to include `workspace_ttl: WorkspaceTtlConfig::default()`.

**Step 3: Update config.example.json**

Add `"workspaceTtl"` section under `agents.defaults` with the default values.

**Step 4: Run tests**

Run: `cargo test --lib test_workspace_ttl_defaults test_config_example_is_up_to_date`
Expected: All PASS

**Step 5: Commit**

```bash
git add src/config/schema/ config.example.json
git commit -m "feat(workspace): add WorkspaceTtlConfig to agent defaults"
```

---

### Task 5: Workspace tool — action-based tool for agent

**Files:**
- Create: `src/agent/tools/workspace_tool/mod.rs`
- Create: `src/agent/tools/workspace_tool/tests.rs`
- Modify: `src/agent/tools/mod.rs` — add `pub mod workspace_tool;`

This is the new `workspace` tool with 8 actions. Follow the action-based tool pattern from the GitHub tool.

**Step 1: Write the failing tests**

Create `src/agent/tools/workspace_tool/tests.rs` with tests for each action:

```rust
use super::*;
use crate::agent::tools::base::ExecutionContext;
use serde_json::json;

fn test_ctx() -> ExecutionContext {
    ExecutionContext::default()
}

#[tokio::test]
async fn test_workspace_tool_tree_action() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Arc::new(MemoryDB::new(tmp.path().join("memory/memory.sqlite3")).unwrap());
    let mgr = Arc::new(WorkspaceManager::new(tmp.path().to_path_buf(), Some(db)));

    // Create some category dirs
    std::fs::create_dir_all(tmp.path().join("code/2026-02-27")).unwrap();
    std::fs::write(tmp.path().join("code/2026-02-27/foo.py"), "x").unwrap();

    let tool = WorkspaceTool::new(mgr);
    let result = tool.execute(json!({"action": "tree"}), &test_ctx()).await.unwrap();
    assert!(!result.is_error);
    assert!(result.output.contains("code"));
}

#[tokio::test]
async fn test_workspace_tool_list_action() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Arc::new(MemoryDB::new(tmp.path().join("memory/memory.sqlite3")).unwrap());
    let mgr = Arc::new(WorkspaceManager::new(tmp.path().to_path_buf(), Some(db.clone())));

    // Register a file
    let file_path = tmp.path().join("code/2026-02-27/test.py");
    std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    std::fs::write(&file_path, "print('hello')").unwrap();
    mgr.register_file(&file_path, Some("write_file"), None).unwrap();

    let tool = WorkspaceTool::new(mgr);
    let result = tool.execute(json!({"action": "list", "category": "code"}), &test_ctx()).await.unwrap();
    assert!(!result.is_error);
    assert!(result.output.contains("test.py"));
}

#[tokio::test]
async fn test_workspace_tool_delete_action() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Arc::new(MemoryDB::new(tmp.path().join("memory/memory.sqlite3")).unwrap());
    let mgr = Arc::new(WorkspaceManager::new(tmp.path().to_path_buf(), Some(db.clone())));

    let file_path = tmp.path().join("temp/2026-02-27/trash.txt");
    std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    std::fs::write(&file_path, "junk").unwrap();
    mgr.register_file(&file_path, None, None).unwrap();

    let tool = WorkspaceTool::new(mgr);
    let result = tool.execute(
        json!({"action": "delete", "path": file_path.to_str().unwrap()}),
        &test_ctx(),
    ).await.unwrap();
    assert!(!result.is_error);
    assert!(!file_path.exists());
}

#[tokio::test]
async fn test_workspace_tool_capabilities_actions() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = Arc::new(WorkspaceManager::new(tmp.path().to_path_buf(), None));
    let tool = WorkspaceTool::new(mgr);
    let caps = tool.capabilities();

    // Verify all 8 actions are declared
    assert_eq!(caps.actions.len(), 8);

    // Verify read-only flags
    let ro_actions: Vec<&str> = caps.actions.iter().filter(|a| a.read_only).map(|a| a.name).collect();
    assert!(ro_actions.contains(&"list"));
    assert!(ro_actions.contains(&"search"));
    assert!(ro_actions.contains(&"info"));
    assert!(ro_actions.contains(&"tree"));

    let rw_actions: Vec<&str> = caps.actions.iter().filter(|a| !a.read_only).map(|a| a.name).collect();
    assert!(rw_actions.contains(&"move"));
    assert!(rw_actions.contains(&"delete"));
    assert!(rw_actions.contains(&"tag"));
    assert!(rw_actions.contains(&"cleanup"));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib test_workspace_tool`
Expected: Compilation error

**Step 3: Implement the workspace tool**

Create `src/agent/tools/workspace_tool/mod.rs`:

```rust
use crate::agent::memory::memory_db::MemoryDB;
use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities};
use crate::agent::tools::{Tool, ToolResult, ToolVersion};
use crate::agent::workspace::WorkspaceManager;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

#[cfg(test)]
mod tests;

pub struct WorkspaceTool {
    manager: Arc<WorkspaceManager>,
}

impl WorkspaceTool {
    pub fn new(manager: Arc<WorkspaceManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for WorkspaceTool {
    fn name(&self) -> &'static str { "workspace" }
    fn description(&self) -> &'static str {
        "Manage workspace files: list, search, organize, and clean up files in the workspace."
    }
    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: false,
            subagent_access: SubagentAccess::ReadOnly,
            actions: actions![
                list: ro,
                search: ro,
                info: ro,
                tree: ro,
                r#move,
                delete,
                tag,
                cleanup,
            ],
        }
    }
    fn parameters(&self) -> Value { /* JSON schema with action enum and per-action params */ }
    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let action = params["action"].as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;
        match action {
            "list" => { /* delegate to manager.list_files(), format output */ }
            "search" => { /* delegate to manager.search_files() */ }
            "info" => { /* get single file info from manifest */ }
            "tree" => { /* walk category dirs, build tree string */ }
            "move" => { /* delegate to manager.move_file() */ }
            "delete" => { /* delegate to manager.remove_file() */ }
            "tag" => { /* delegate to manager.tag_file() */ }
            "cleanup" => { /* delegate to manager.cleanup_expired() */ }
            _ => Ok(ToolResult::error(format!("unknown action: {action}")))
        }
    }
}
```

Use the `actions!` macro from `src/agent/tools/base/mod.rs` (same as GitHub tool). Note: `move` is a keyword in Rust, so use `r#move` in the macro and `"move"` as the string match.

**Step 4: Run tests**

Run: `cargo test --lib test_workspace_tool`
Expected: All PASS

**Step 5: Commit**

```bash
git add src/agent/tools/workspace_tool/ src/agent/tools/mod.rs
git commit -m "feat(workspace): add workspace action-based tool with 8 actions"
```

---

### Task 6: Tool registration + wiring through ToolBuildContext

**Files:**
- Modify: `src/agent/tools/setup.rs` — add `register_workspace` function and call from `register_all_tools()`
- Modify: `src/agent/tools/setup.rs` — add `workspace_manager: Option<Arc<WorkspaceManager>>` to `ToolBuildContext`
- Modify: `src/agent/loop/mod.rs` — pass workspace manager through `AgentLoop::new()` to `ToolBuildContext`

**Step 1: Add workspace manager to ToolBuildContext**

In `src/agent/tools/setup.rs`, add field:

```rust
pub workspace_manager: Option<Arc<crate::agent::workspace::WorkspaceManager>>,
```

**Step 2: Add register_workspace function**

```rust
fn register_workspace(registry: &mut ToolRegistry, ctx: &ToolBuildContext) {
    use crate::agent::tools::workspace_tool::WorkspaceTool;

    if let Some(ref mgr) = ctx.workspace_manager {
        registry.register(Arc::new(WorkspaceTool::new(mgr.clone())));
    }
}
```

Call it from `register_all_tools()` (after `register_memory_search`).

**Step 3: Construct WorkspaceManager in AgentLoop::new()**

In `src/agent/loop/mod.rs`, where `ToolBuildContext` is built (inside `AgentLoop::new()`), construct the `WorkspaceManager`:

```rust
let workspace_manager = memory_store
    .db()
    .map(|db| Arc::new(WorkspaceManager::new(config.workspace.clone(), Some(db))));
```

Pass it into `ToolBuildContext`:

```rust
workspace_manager,
```

**Step 4: Update test_defaults**

In `AgentLoopConfig::test_defaults()`, set `workspace_manager: None` on the `ToolBuildContext` equivalent. And update `tests/common/mod.rs` `create_test_agent_with()` if it constructs `ToolBuildContext` directly.

**Step 5: Run all tests**

Run: `cargo test -- --test-threads=1`
Expected: All PASS

**Step 6: Commit**

```bash
git add src/agent/tools/setup.rs src/agent/loop/mod.rs tests/common/mod.rs
git commit -m "feat(workspace): wire WorkspaceManager through ToolBuildContext and register workspace tool"
```

---

### Task 7: WriteFileTool integration — auto-routing + manifest registration

**Files:**
- Modify: `src/agent/tools/filesystem/mod.rs` — add `WorkspaceManager` to `WriteFileTool`, auto-route workspace writes
- Update existing tests or add new ones

The key behavior: when `WriteFileTool` writes to the workspace and the target path is *directly under the workspace root* (not in an existing subdirectory), auto-route via `WorkspaceManager.resolve_path()`. When a specific subdirectory path is given, respect it. After any workspace write, call `register_file()`.

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn test_write_file_auto_routes_to_category() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Arc::new(MemoryDB::new(tmp.path().join("memory/memory.sqlite3")).unwrap());
    let mgr = Arc::new(WorkspaceManager::new(tmp.path().to_path_buf(), Some(db.clone())));

    let tool = WriteFileTool::new(
        None,
        None,
        Some(tmp.path().to_path_buf()),
    ).with_workspace_manager(mgr.clone());

    // Write to workspace root — should be auto-routed
    let workspace_path = tmp.path().join("script.py");
    let result = tool.execute(
        json!({"path": workspace_path.to_str().unwrap(), "content": "print('hi')"}),
        &ExecutionContext::default(),
    ).await.unwrap();
    assert!(!result.is_error);

    // File should be in code/{today}/script.py, NOT at workspace root
    assert!(!workspace_path.exists());
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let expected = tmp.path().join("code").join(&today).join("script.py");
    assert!(expected.exists());

    // Should be in manifest
    let files = mgr.list_files(Some(FileCategory::Code), None, None).unwrap();
    assert_eq!(files.len(), 1);
}
```

**Step 2: Implement**

Add `workspace_manager: Option<Arc<WorkspaceManager>>` field to `WriteFileTool`. Add a `with_workspace_manager()` builder method. In `execute()`:

1. After expanding the path, check if it falls directly under the workspace root (parent == workspace root, not in a reserved dir, not already in a category dir).
2. If so, call `workspace_manager.resolve_path(filename, None)` to get the organized path.
3. Create the date subdirectory and write to the organized path instead.
4. After successful write, call `workspace_manager.register_file()`.
5. For explicit paths that are already in a category dir, just register after write.

Update `register_filesystem()` in `setup.rs` to pass the workspace manager to `WriteFileTool`.

**Step 3: Run tests**

Run: `cargo test --lib test_write_file`
Expected: All PASS

**Step 4: Commit**

```bash
git add src/agent/tools/filesystem/ src/agent/tools/setup.rs
git commit -m "feat(workspace): auto-route workspace writes through WorkspaceManager"
```

---

### Task 8: ReadFileTool integration — accessed_at tracking

**Files:**
- Modify: `src/agent/tools/filesystem/mod.rs` — add `WorkspaceManager` to `ReadFileTool`, call `touch_file()` on reads

**Step 1: Implement**

Add `workspace_manager: Option<Arc<WorkspaceManager>>` to `ReadFileTool`. In `execute()`, after a successful read, if the path is a managed workspace file, call `workspace_manager.touch_file()`. This is fire-and-forget — don't fail the read if manifest update fails.

**Step 2: Update registration**

In `register_filesystem()`, pass the workspace manager to `ReadFileTool`.

**Step 3: Run tests**

Run: `cargo test --lib`
Expected: All PASS

**Step 4: Commit**

```bash
git add src/agent/tools/filesystem/ src/agent/tools/setup.rs
git commit -m "feat(workspace): track accessed_at in manifest on file reads"
```

---

### Task 9: Hygiene integration — cleanup expired workspace files

**Files:**
- Modify: `src/agent/memory/hygiene/mod.rs` — add `cleanup_workspace_files()` function
- Modify: `src/agent/memory/indexer.rs` — call workspace cleanup during hygiene cycle
- Add tests

**Step 1: Write the failing test**

```rust
#[test]
fn test_cleanup_workspace_files_removes_expired() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace_root = tmp.path().to_path_buf();
    let db = Arc::new(MemoryDB::new(tmp.path().join("memory/memory.sqlite3")).unwrap());

    // Create an old temp file
    let old_file = workspace_root.join("temp/2026-02-10/old.txt");
    std::fs::create_dir_all(old_file.parent().unwrap()).unwrap();
    std::fs::write(&old_file, "old").unwrap();

    // Register with old date
    let old_date = "2026-02-10T00:00:00Z";
    db.register_workspace_file_with_date(
        "temp/2026-02-10/old.txt", "temp", Some("old.txt"), 3, None, None, old_date,
    ).unwrap();

    let ttl = WorkspaceTtlConfig::default(); // temp = 7 days
    let removed = cleanup_workspace_files(&db, &workspace_root, &ttl.to_map()).unwrap();
    assert_eq!(removed, 1);
    assert!(!old_file.exists());
}
```

**Step 2: Implement**

In `src/agent/memory/hygiene/mod.rs`, add:

```rust
pub fn cleanup_workspace_files(
    db: &MemoryDB,
    workspace_root: &Path,
    ttl_map: &std::collections::HashMap<String, Option<u64>>,
) -> Result<u32> {
    let mut total_removed = 0;
    for (category, ttl) in ttl_map {
        let Some(days) = ttl else { continue };
        let expired = db.list_expired_workspace_files(category, *days as u32)?;
        for entry in expired {
            let abs_path = workspace_root.join(&entry.path);
            if abs_path.exists() {
                std::fs::remove_file(&abs_path)?;
            }
            db.unregister_workspace_file(&entry.path)?;
            total_removed += 1;
        }
    }
    Ok(total_removed)
}
```

In `run_hygiene()`, add a call (requires passing workspace_root and ttl_map). Extend the function signature or create a `run_workspace_hygiene()` that's called from the indexer alongside `run_hygiene()`.

In the indexer (`src/agent/memory/indexer.rs`), where `run_hygiene()` is called, also call `cleanup_workspace_files()` with the TTL config. The TTL config needs to be threaded through — add it to `MemoryIndexer` fields.

**Step 3: Run tests**

Run: `cargo test --lib test_cleanup_workspace`
Expected: All PASS

**Step 4: Commit**

```bash
git add src/agent/memory/hygiene/ src/agent/memory/indexer.rs
git commit -m "feat(workspace): integrate workspace file cleanup into hygiene cycle"
```

---

### Task 10: Documentation updates

**Files:**
- Modify: `docs/_pages/tools.html` — add workspace tool documentation
- Modify: `docs/_pages/config.html` — add workspaceTtl config documentation
- Modify: `README.md` — add workspace tool to tool list
- Run: `python3 docs/build.py`

**Step 1: Update docs**

In `docs/_pages/tools.html`, add a section for the `workspace` tool with all 8 actions documented.

In `docs/_pages/config.html`, add the `workspaceTtl` config fields.

In `README.md`, add `workspace` to the tool list with a one-line description: "Manage workspace files: list, search, organize, and clean up."

**Step 2: Build docs**

Run: `python3 docs/build.py`

**Step 3: Verify no diff**

Run: `git diff --quiet -- docs/` (should be clean after build)

**Step 4: Commit**

```bash
git add docs/ README.md
git commit -m "docs: add workspace tool and workspaceTtl config documentation"
```

---

### Task 11: Update CLAUDE.md

**Files:**
- Modify: `CLAUDE.md` — add workspace manager notes to Architecture, Common Pitfalls, etc.

Add a bullet point to the Common Pitfalls section about the workspace manager:

```
- **Workspace file routing**: Files written to workspace root are auto-routed to `{category}/{YYYY-MM-DD}/` directories by `WorkspaceManager`. Category inferred from extension. Manifest tracked in `workspace_files` SQLite table. Reserved dirs (`memory/`, `knowledge/`, `skills/`, `sessions/`) are NOT managed by workspace manager. TTL config in `agents.defaults.workspaceTtl`.
```

**Commit:**

```bash
git add CLAUDE.md
git commit -m "docs: add workspace manager notes to CLAUDE.md"
```

---

### Task 12: Integration test

**Files:**
- Create: `tests/workspace_management.rs`

Write an integration test that exercises the full flow: write a file through WriteFileTool, verify it lands in the right category/date dir, verify it appears in workspace tool's `list` action, verify cleanup removes expired files.

This test should use `MockLLMProvider` and `TempDir` following the pattern in `tests/common/mod.rs`.

**Step 1: Write the test**

```rust
#[tokio::test]
async fn test_workspace_file_lifecycle() {
    // Setup: create agent with workspace manager
    // Write a file via write_file tool
    // Verify auto-routing to code/{date}/
    // Query via workspace tool list action
    // Verify manifest entry exists
    // Delete via workspace tool delete action
    // Verify file and manifest entry are gone
}
```

**Step 2: Run**

Run: `cargo test --test workspace_management -- --test-threads=1`
Expected: PASS

**Step 3: Commit**

```bash
git add tests/workspace_management.rs
git commit -m "test: add workspace management integration test"
```
