use crate::agent::memory::memory_db::MemoryDB;
use chrono::Utc;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

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
    pub fn new(workspace_root: PathBuf, db: Option<Arc<MemoryDB>>) -> Self {
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
}
