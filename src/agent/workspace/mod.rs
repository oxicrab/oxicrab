use crate::agent::memory::memory_db::MemoryDB;
pub use crate::agent::memory::memory_db::WorkspaceFileEntry;
use anyhow::Result;
use chrono::Utc;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use tracing::warn;
use walkdir::WalkDir;

#[cfg(test)]
mod tests;

/// Directories reserved for existing workspace subsystems (not managed by `WorkspaceManager`).
const RESERVED_DIRS: &[&str] = &["memory", "knowledge", "skills", "sessions"];

/// File categories for workspace organization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileCategory {
    Code,
    Documents,
    Data,
    Images,
    Downloads,
    Temp,
}

impl FileCategory {
    /// All category variants.
    pub const ALL: [FileCategory; 6] = [
        FileCategory::Code,
        FileCategory::Documents,
        FileCategory::Data,
        FileCategory::Images,
        FileCategory::Downloads,
        FileCategory::Temp,
    ];

    /// Returns the directory name for this category.
    pub fn as_str(&self) -> &'static str {
        match self {
            FileCategory::Code => "code",
            FileCategory::Documents => "documents",
            FileCategory::Data => "data",
            FileCategory::Images => "images",
            FileCategory::Downloads => "downloads",
            FileCategory::Temp => "temp",
        }
    }
}

impl std::fmt::Display for FileCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for FileCategory {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "code" => Ok(FileCategory::Code),
            "documents" => Ok(FileCategory::Documents),
            "data" => Ok(FileCategory::Data),
            "images" => Ok(FileCategory::Images),
            "downloads" => Ok(FileCategory::Downloads),
            "temp" => Ok(FileCategory::Temp),
            _ => Err(format!("unknown file category: {s}")),
        }
    }
}

/// Infers a file category from its extension.
pub fn infer_category(path: &Path) -> FileCategory {
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => e.to_lowercase(),
        None => return FileCategory::Temp,
    };

    match ext.as_str() {
        // Code
        "py" | "rs" | "js" | "ts" | "tsx" | "jsx" | "sh" | "bash" | "rb" | "go" | "java" | "c"
        | "cpp" | "h" | "hpp" | "html" | "css" | "sql" | "lua" | "php" | "swift" | "kt"
        | "scala" | "r" | "pl" | "zig" | "nim" | "ex" | "exs" | "erl" => FileCategory::Code,
        // Documents
        "md" | "txt" | "doc" | "docx" | "rtf" | "org" | "rst" | "adoc" | "tex" | "log" => {
            FileCategory::Documents
        }
        // Data
        "csv" | "json" | "yaml" | "yml" | "xml" | "toml" | "parquet" | "tsv" | "ndjson"
        | "jsonl" | "sqlite" | "sqlite3" | "db" => FileCategory::Data,
        // Images
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "bmp" | "ico" | "tiff" | "tif"
        | "avif" | "heic" => FileCategory::Images,
        // Downloads
        "pdf" | "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "epub" | "mobi" | "whl"
        | "deb" | "rpm" | "dmg" | "iso" | "apk" => FileCategory::Downloads,
        // Default
        _ => FileCategory::Temp,
    }
}

/// Manages workspace file organization with category-based directory structure.
pub struct WorkspaceManager {
    workspace_root: PathBuf,
    db: Option<Arc<MemoryDB>>,
}

impl WorkspaceManager {
    /// Creates a new `WorkspaceManager`.
    ///
    /// The workspace root is canonicalized to ensure consistent path matching
    /// with `tokio::fs::canonicalize()` used by filesystem tools.
    pub fn new(workspace_root: PathBuf, db: Option<Arc<MemoryDB>>) -> Self {
        let workspace_root = workspace_root.canonicalize().unwrap_or(workspace_root);
        Self { workspace_root, db }
    }

    /// Returns the workspace root path.
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    /// Returns a reference to the memory database, if configured.
    pub fn db(&self) -> Option<&Arc<MemoryDB>> {
        self.db.as_ref()
    }

    /// Resolves a filename to a full path under the appropriate category directory.
    ///
    /// Path format: `{workspace}/{category}/{YYYY-MM-DD}/{filename}`
    ///
    /// If `category_hint` is provided, it overrides the inferred category.
    /// Path traversal components are stripped — only the final filename component is used.
    pub fn resolve_path(&self, filename: &str, category_hint: Option<FileCategory>) -> PathBuf {
        let path = Path::new(filename);
        let sanitized = path.file_name().unwrap_or(path.as_os_str());
        let category = category_hint.unwrap_or_else(|| infer_category(path));
        let date = Utc::now().format("%Y-%m-%d").to_string();
        self.workspace_root
            .join(category.as_str())
            .join(&date)
            .join(sanitized)
    }

    /// Returns true if the given path is inside a managed category directory.
    ///
    /// Returns false for reserved directories (memory, knowledge, skills, sessions),
    /// root-level files, and paths outside the workspace.
    pub fn is_managed_path(&self, path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(&self.workspace_root) else {
            return false;
        };

        // Reject path traversal attempts
        if relative
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return false;
        }

        let mut comps = relative.components();
        let first_component = match comps.next() {
            Some(std::path::Component::Normal(name)) => name.to_str().unwrap_or(""),
            _ => return false,
        };

        // Must have more than just the first component (not a root-level file)
        if comps.next().is_none() {
            return false;
        }

        // Check it's a known category and not a reserved dir
        if RESERVED_DIRS.contains(&first_component) {
            return false;
        }

        first_component.parse::<FileCategory>().is_ok()
    }

    // ── Manifest integration methods ────────────────────────────

    /// Compute a relative path string from an absolute path under the workspace root.
    fn relative_path(&self, abs_path: &Path) -> Option<String> {
        abs_path
            .strip_prefix(&self.workspace_root)
            .ok()
            .and_then(|p| p.to_str())
            .map(ToString::to_string)
    }

    /// Register a file in the workspace manifest.
    ///
    /// Computes relative path, file size, category, and original name from the
    /// absolute path, then writes a manifest entry to the database.
    pub fn register_file(
        &self,
        abs_path: &Path,
        source_tool: Option<&str>,
        session_key: Option<&str>,
    ) -> Result<()> {
        let Some(db) = &self.db else {
            return Ok(());
        };

        let Some(rel) = self.relative_path(abs_path) else {
            warn!(
                "register_file: path is outside workspace: {}",
                abs_path.display()
            );
            return Ok(());
        };

        let size_bytes = std::fs::metadata(abs_path).map_or(0, |m| m.len() as i64);

        let original_name = abs_path
            .file_name()
            .and_then(|n| n.to_str())
            .map(ToString::to_string);

        // Determine category: first path component if it's a known category, else infer
        let category = category_from_relative(&rel);

        db.register_workspace_file(
            &rel,
            category.as_str(),
            original_name.as_deref(),
            size_bytes,
            source_tool,
            session_key,
        )
    }

    /// List workspace files with optional filters.
    pub fn list_files(
        &self,
        category: Option<FileCategory>,
        date: Option<&str>,
        tag: Option<&str>,
    ) -> Result<Vec<WorkspaceFileEntry>> {
        let Some(db) = &self.db else {
            return Ok(Vec::new());
        };

        db.list_workspace_files(category.map(|c| c.as_str()), date, tag)
    }

    /// Search workspace files by path or original name.
    pub fn search_files(&self, query: &str) -> Result<Vec<WorkspaceFileEntry>> {
        let Some(db) = &self.db else {
            return Ok(Vec::new());
        };

        db.search_workspace_files(query)
    }

    /// Remove a file from disk and from the manifest.
    pub fn remove_file(&self, abs_path: &Path) -> Result<()> {
        if !self.is_managed_path(abs_path) {
            anyhow::bail!(
                "path is not a managed workspace file: {}",
                abs_path.display()
            );
        }
        std::fs::remove_file(abs_path)?;

        let Some(db) = &self.db else {
            return Ok(());
        };

        if let Some(rel) = self.relative_path(abs_path) {
            db.unregister_workspace_file(&rel)?;
        }
        Ok(())
    }

    /// Move a file to a new category directory.
    ///
    /// The file keeps its date subdirectory (if any) and filename.
    /// Returns the new absolute path.
    pub fn move_file(&self, abs_path: &Path, new_category: FileCategory) -> Result<PathBuf> {
        if !self.is_managed_path(abs_path) {
            anyhow::bail!(
                "path is not a managed workspace file: {}",
                abs_path.display()
            );
        }
        let old_rel = self
            .relative_path(abs_path)
            .ok_or_else(|| anyhow::anyhow!("path is not under workspace root"))?;

        // Build new relative path: {new_category}/{remaining after first component}
        let rel_path = Path::new(&old_rel);
        let after_first: PathBuf = rel_path.components().skip(1).collect();
        let new_rel_path = Path::new(new_category.as_str()).join(&after_first);
        let new_rel = new_rel_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non-UTF-8 path"))?;

        let new_abs_path = self.workspace_root.join(&new_rel_path);

        if let Some(parent) = new_abs_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::rename(abs_path, &new_abs_path)?;

        if let Some(db) = &self.db {
            db.move_workspace_file(&old_rel, new_rel, new_category.as_str())?;
        }

        Ok(new_abs_path)
    }

    /// Set tags on a workspace file.
    pub fn tag_file(&self, abs_path: &Path, tags: &str) -> Result<()> {
        let Some(db) = &self.db else {
            return Ok(());
        };

        if let Some(rel) = self.relative_path(abs_path) {
            db.set_workspace_file_tags(&rel, tags)?;
        }
        Ok(())
    }

    /// Update `accessed_at` timestamp for a workspace file (fire-and-forget).
    pub fn touch_file(&self, abs_path: &Path) -> Result<()> {
        let Some(db) = &self.db else {
            return Ok(());
        };

        if let Some(rel) = self.relative_path(abs_path) {
            // Fire-and-forget: don't fail if file isn't in manifest
            let _ = db.touch_workspace_file(&rel);
        }
        Ok(())
    }

    /// Clean up expired files based on per-category TTLs.
    ///
    /// `ttl_map` maps category name to optional TTL in days (`None` = no expiry).
    /// Returns the total number of files removed.
    pub fn cleanup_expired(&self, ttl_map: &HashMap<String, Option<u64>>) -> Result<u32> {
        let Some(db) = &self.db else {
            return Ok(0);
        };

        let mut total_removed = 0u32;

        for (category, ttl) in ttl_map {
            let Some(days) = ttl else {
                continue;
            };

            let expired = db.list_expired_workspace_files(category, *days as u32)?;
            for entry in &expired {
                let abs = self.workspace_root.join(&entry.path);
                if abs.exists() && std::fs::remove_file(&abs).is_err() {
                    warn!("failed to remove expired file: {}", abs.display());
                    continue;
                }
                db.unregister_workspace_file(&entry.path)?;
                total_removed += 1;
            }
        }

        Ok(total_removed)
    }

    /// Synchronize the manifest with the filesystem.
    ///
    /// Removes stale manifest entries for files that no longer exist on disk, and
    /// discovers untracked files in category directories.
    ///
    /// Returns `(removed_stale, discovered_new)`.
    pub fn sync_manifest(&self) -> Result<(u32, u32)> {
        let Some(db) = &self.db else {
            return Ok((0, 0));
        };

        // Phase 1: remove stale entries
        let all_entries = db.list_workspace_files(None, None, None)?;
        let mut removed_stale = 0u32;
        for entry in &all_entries {
            let abs = self.workspace_root.join(&entry.path);
            if !abs.exists() {
                db.unregister_workspace_file(&entry.path)?;
                removed_stale += 1;
            }
        }

        // Phase 2: discover untracked files
        let remaining = db.list_workspace_files(None, None, None)?;
        let known_paths: std::collections::HashSet<String> =
            remaining.into_iter().map(|e| e.path).collect();

        let mut discovered_new = 0u32;
        for cat in FileCategory::ALL {
            let cat_dir = self.workspace_root.join(cat.as_str());
            if !cat_dir.is_dir() {
                continue;
            }

            for entry in WalkDir::new(&cat_dir)
                .max_depth(4)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|e| e.file_type().is_file())
            {
                let abs = entry.path();
                if let Some(rel) = self.relative_path(abs)
                    && !known_paths.contains(&rel)
                {
                    let size = std::fs::metadata(abs).map_or(0, |m| m.len() as i64);
                    let original_name = abs
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(ToString::to_string);
                    db.register_workspace_file(
                        &rel,
                        cat.as_str(),
                        original_name.as_deref(),
                        size,
                        None,
                        None,
                    )?;
                    discovered_new += 1;
                }
            }
        }

        Ok((removed_stale, discovered_new))
    }
}

/// Determine category from the first component of a relative path.
fn category_from_relative(rel: &str) -> FileCategory {
    let first = Path::new(rel)
        .components()
        .next()
        .and_then(|c| match c {
            std::path::Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .unwrap_or("");

    first
        .parse::<FileCategory>()
        .unwrap_or_else(|_| infer_category(Path::new(rel)))
}
