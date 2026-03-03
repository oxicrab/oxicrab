use super::MemoryDB;
use anyhow::Result;
use rusqlite::params;
use std::fmt::Write as _;

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

impl MemoryDB {
    /// Register a workspace file in the manifest (upsert).
    #[allow(clippy::too_many_arguments)]
    pub fn register_workspace_file(
        &self,
        path: &str,
        category: &str,
        original_name: Option<&str>,
        size_bytes: i64,
        source_tool: Option<&str>,
        session_key: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "INSERT INTO workspace_files
               (path, category, original_name, size_bytes, source_tool, session_key, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
             ON CONFLICT(path) DO UPDATE SET
               category = excluded.category,
               original_name = excluded.original_name,
               size_bytes = excluded.size_bytes,
               source_tool = excluded.source_tool,
               session_key = excluded.session_key",
            params![
                path,
                category,
                original_name,
                size_bytes,
                source_tool,
                session_key
            ],
        )?;
        Ok(())
    }

    /// Register a workspace file with an explicit `created_at` timestamp (for testing).
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn register_workspace_file_with_date(
        &self,
        path: &str,
        category: &str,
        original_name: Option<&str>,
        size_bytes: i64,
        source_tool: Option<&str>,
        session_key: Option<&str>,
        created_at: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "INSERT INTO workspace_files
               (path, category, original_name, size_bytes, source_tool, session_key, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(path) DO UPDATE SET
               category = excluded.category,
               original_name = excluded.original_name,
               size_bytes = excluded.size_bytes,
               source_tool = excluded.source_tool,
               session_key = excluded.session_key,
               created_at = excluded.created_at",
            params![
                path,
                category,
                original_name,
                size_bytes,
                source_tool,
                session_key,
                created_at
            ],
        )?;
        Ok(())
    }

    /// Remove a workspace file from the manifest.
    pub fn unregister_workspace_file(&self, path: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute("DELETE FROM workspace_files WHERE path = ?1", params![path])?;
        Ok(())
    }

    /// List workspace files with optional filters.
    pub fn list_workspace_files(
        &self,
        category: Option<&str>,
        date: Option<&str>,
        tag: Option<&str>,
    ) -> Result<Vec<WorkspaceFileEntry>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;

        let mut sql = String::from(
            "SELECT id, path, category, original_name, size_bytes, source_tool, tags, created_at, accessed_at, session_key
             FROM workspace_files WHERE 1=1",
        );
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(cat) = category {
            let _ = write!(sql, " AND category = ?{}", param_values.len() + 1);
            param_values.push(Box::new(cat.to_string()));
        }
        if let Some(d) = date {
            let _ = write!(sql, " AND created_at LIKE ?{}", param_values.len() + 1);
            param_values.push(Box::new(format!("{d}%")));
        }
        if let Some(t) = tag {
            let _ = write!(
                sql,
                " AND (',' || tags || ',' LIKE '%,' || ?{} || ',%')",
                param_values.len() + 1
            );
            param_values.push(Box::new(t.to_string()));
        }
        sql.push_str(" ORDER BY created_at DESC");

        let mut stmt = conn.prepare(&sql)?;
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(AsRef::as_ref).collect();
        let rows = stmt
            .query_map(params_ref.as_slice(), |row| {
                Ok(WorkspaceFileEntry {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    category: row.get(2)?,
                    original_name: row.get(3)?,
                    size_bytes: row.get(4)?,
                    source_tool: row.get(5)?,
                    tags: row.get(6)?,
                    created_at: row.get(7)?,
                    accessed_at: row.get(8)?,
                    session_key: row.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    /// Search workspace files by path or original name.
    pub fn search_workspace_files(&self, query: &str) -> Result<Vec<WorkspaceFileEntry>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let escaped: String = query
            .chars()
            .flat_map(|c| match c {
                '\\' => vec!['\\', '\\'],
                '%' => vec!['\\', '%'],
                '_' => vec!['\\', '_'],
                other => vec![other],
            })
            .collect();
        let pattern = format!("%{escaped}%");
        let mut stmt = conn.prepare(
            "SELECT id, path, category, original_name, size_bytes, source_tool, tags, created_at, accessed_at, session_key
             FROM workspace_files
             WHERE path LIKE ?1 ESCAPE '\\' OR original_name LIKE ?1 ESCAPE '\\'
             ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![pattern], |row| {
                Ok(WorkspaceFileEntry {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    category: row.get(2)?,
                    original_name: row.get(3)?,
                    size_bytes: row.get(4)?,
                    source_tool: row.get(5)?,
                    tags: row.get(6)?,
                    created_at: row.get(7)?,
                    accessed_at: row.get(8)?,
                    session_key: row.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    /// Update `accessed_at` timestamp for a workspace file.
    pub fn touch_workspace_file(&self, path: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "UPDATE workspace_files SET accessed_at = datetime('now') WHERE path = ?1",
            params![path],
        )?;
        Ok(())
    }

    /// Set tags on a workspace file.
    pub fn set_workspace_file_tags(&self, path: &str, tags: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "UPDATE workspace_files SET tags = ?1 WHERE path = ?2",
            params![tags, path],
        )?;
        Ok(())
    }

    /// List workspace files that have exceeded their TTL.
    pub fn list_expired_workspace_files(
        &self,
        category: &str,
        ttl_days: u32,
    ) -> Result<Vec<WorkspaceFileEntry>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let modifier = format!("-{ttl_days} days");
        let mut stmt = conn.prepare(
            "SELECT id, path, category, original_name, size_bytes, source_tool, tags, created_at, accessed_at, session_key
             FROM workspace_files
             WHERE category = ?1 AND created_at < datetime('now', ?2)
             ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![category, modifier], |row| {
                Ok(WorkspaceFileEntry {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    category: row.get(2)?,
                    original_name: row.get(3)?,
                    size_bytes: row.get(4)?,
                    source_tool: row.get(5)?,
                    tags: row.get(6)?,
                    created_at: row.get(7)?,
                    accessed_at: row.get(8)?,
                    session_key: row.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    /// Move a workspace file to a new path and category.
    pub fn move_workspace_file(
        &self,
        old_path: &str,
        new_path: &str,
        new_category: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "UPDATE workspace_files SET path = ?1, category = ?2 WHERE path = ?3",
            params![new_path, new_category, old_path],
        )?;
        Ok(())
    }
}
