use crate::agent::memory::MemoryDB;
use anyhow::Result;
use chrono::{Datelike, Utc};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub struct MemoryStore {
    _workspace: PathBuf,
    memory_dir: PathBuf,
    db: MemoryDB,
}

impl MemoryStore {
    pub fn new(workspace: impl AsRef<Path>) -> Result<Self> {
        let workspace = workspace.as_ref();
        let memory_dir = workspace.join("memory");
        std::fs::create_dir_all(&memory_dir)?;

        let db_path = memory_dir.join("memory.sqlite3");
        let db = MemoryDB::new(db_path)?;

        Ok(Self {
            _workspace: workspace.to_path_buf(),
            memory_dir,
            db,
        })
    }

    pub fn get_memory_context(&self, query: Option<&str>) -> Result<String> {
        // Index memory directory
        self.db.index_directory(&self.memory_dir)?;

        // Always include MEMORY.md
        let memory_file = self.memory_dir.join("MEMORY.md");
        if memory_file.exists() {
            self.db.index_file("MEMORY.md", &memory_file)?;
        }

        // Get today's date for daily notes
        let today = Utc::now();
        let today_key = format!(
            "{}-{:02}-{:02}.md",
            today.year(),
            today.month(),
            today.day()
        );
        let today_file = self.memory_dir.join(&today_key);

        // Index today's note
        if today_file.exists() {
            self.db.index_file(&today_key, &today_file)?;
        }

        // Search for relevant chunks if query provided
        let mut chunks = Vec::new();
        if let Some(query) = query {
            let exclude: HashSet<String> = [today_key.clone()].iter().cloned().collect();
            let hits = self.db.search(query, 8, Some(&exclude))?;
            for hit in hits {
                chunks.push(format!("**{}**: {}", hit.source_key, hit.content));
            }
        }

        // Always include MEMORY.md content (fallback when no query or no FTS results)
        if chunks.is_empty() || query.is_none() {
            if let Ok(long_term) = self.read_long_term() {
                if !long_term.trim().is_empty() {
                    chunks.insert(0, format!("## Long-term Memory\n{}", long_term));
                }
            }
        }

        // Include today's note
        if today_file.exists() {
            if let Ok(content) = std::fs::read_to_string(&today_file) {
                if !content.trim().is_empty() {
                    chunks.push(format!("**Today's Notes ({})**:\n{}", today_key, content));
                }
            }
        }

        Ok(chunks.join("\n\n---\n\n"))
    }

    pub fn get_today_file(&self) -> PathBuf {
        let today = Utc::now();
        self.memory_dir.join(format!(
            "{}-{:02}-{:02}.md",
            today.year(),
            today.month(),
            today.day()
        ))
    }

    #[allow(dead_code)] // May be used by tools or future features
    pub fn read_today(&self) -> Result<String> {
        let today_file = self.get_today_file();
        if today_file.exists() {
            Ok(std::fs::read_to_string(&today_file)?)
        } else {
            Ok(String::new())
        }
    }

    pub fn append_today(&self, content: &str) -> Result<()> {
        let today_file = self.get_today_file();
        let today = Utc::now();
        let date_str = format!("{}-{:02}-{:02}", today.year(), today.month(), today.day());

        if today_file.exists() {
            let existing = std::fs::read_to_string(&today_file)?;
            let new_content = format!("{}\n{}", existing, content);
            std::fs::write(&today_file, new_content)?;
        } else {
            let header = format!("# {}\n\n", date_str);
            std::fs::write(&today_file, format!("{}{}", header, content))?;
        }
        Ok(())
    }

    pub fn read_long_term(&self) -> Result<String> {
        let memory_file = self.memory_dir.join("MEMORY.md");
        if memory_file.exists() {
            Ok(std::fs::read_to_string(&memory_file)?)
        } else {
            Ok(String::new())
        }
    }

    #[allow(dead_code)] // May be used by tools or future features
    pub fn write_long_term(&self, content: &str) -> Result<()> {
        let memory_file = self.memory_dir.join("MEMORY.md");
        std::fs::write(&memory_file, content)?;
        Ok(())
    }

    #[allow(dead_code)] // May be used by tools or future features
    pub fn get_recent_memories(&self, days: usize) -> Result<String> {
        let mut memories = Vec::new();
        let today = Utc::now().date_naive();

        for i in 0..days {
            let date = today - chrono::Duration::days(i as i64);
            let date_str = date.format("%Y-%m-%d").to_string();
            let file_path = self.memory_dir.join(format!("{}.md", date_str));

            if file_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&file_path) {
                    memories.push(content);
                }
            }
        }

        Ok(memories.join("\n\n---\n\n"))
    }

    #[allow(dead_code)] // May be used by tools or future features
    pub fn list_memory_files(&self) -> Result<Vec<PathBuf>> {
        if !self.memory_dir.exists() {
            return Ok(vec![]);
        }

        let mut files = Vec::new();
        for entry in std::fs::read_dir(&self.memory_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension() == Some(std::ffi::OsStr::new("md")) {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    // Check if it matches YYYY-MM-DD.md pattern
                    if name.len() == 13 && name.chars().take(4).all(|c| c.is_ascii_digit()) {
                        files.push(path);
                    }
                }
            }
        }

        files.sort_by(|a, b| b.cmp(a)); // Newest first
        Ok(files)
    }
}
