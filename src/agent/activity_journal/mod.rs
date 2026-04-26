//! Append-only NDJSON timeline of every conversation turn.
//!
//! Each line is a self-contained JSON object with a UTC timestamp,
//! session key, role (`user`/`agent`/`system`), and content. The journal
//! is never auto-truncated; operators rotate or archive it manually.
//! The file's directory is created on first write.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

pub mod query;
mod time_parser;

#[cfg(test)]
mod tests;

pub use query::{ActivityWindow, query_window, render_records};
pub use time_parser::{ResolvedAnchor, parse_time_expression};

/// One record on disk. `content` is truncated to the configured
/// `max_content_chars` at write time to keep the journal compact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivityRecord {
    pub timestamp: DateTime<Utc>,
    pub session_key: String,
    pub role: String,
    pub content: String,
}

pub struct ActivityJournal {
    path: PathBuf,
    max_content_chars: usize,
    /// Serialises writers so concurrent `append()` calls produce well-formed
    /// lines even though `tokio::fs::File` doesn't guarantee atomic appends
    /// across threads.
    write_lock: Mutex<()>,
}

impl ActivityJournal {
    /// Open (or create) the NDJSON journal at `path`. The parent
    /// directory is created if missing.
    pub fn new(path: PathBuf, max_content_chars: usize) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create activity journal directory: {}",
                    parent.display()
                )
            })?;
        }
        Ok(Self {
            path,
            max_content_chars: max_content_chars.max(64),
            write_lock: Mutex::new(()),
        })
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Append one record. `content` is truncated at a UTF-8 boundary
    /// before serialisation. Errors are surfaced to the caller — callers
    /// that journal in the hot path should `let _ =` the result so a
    /// disk failure never breaks user-facing replies.
    pub async fn append(&self, session_key: &str, role: &str, content: &str) -> Result<()> {
        let record = ActivityRecord {
            timestamp: Utc::now(),
            session_key: session_key.to_string(),
            role: role.to_string(),
            content: truncate_utf8(content, self.max_content_chars),
        };
        let mut line =
            serde_json::to_string(&record).context("activity journal serialisation failed")?;
        line.push('\n');

        // Single-line writes hold the lock briefly; `OpenOptions.append`
        // gives O_APPEND semantics on POSIX so the kernel handles
        // atomicity for writes < PIPE_BUF, but we serialise at the
        // process level too because we may write longer payloads.
        // `tokio::sync::Mutex` so the guard is `Send` across the
        // `OpenOptions.open` await.
        let _guard = self.write_lock.lock().await;
        let mut f = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
            .with_context(|| format!("opening {}", self.path.display()))?;
        f.write_all(line.as_bytes())
            .await
            .with_context(|| format!("writing to {}", self.path.display()))?;
        // Flush + close explicitly. Without this, a subsequent
        // `read_all` on the same file inside one test run can race
        // the implicit drop and observe a partial line.
        f.flush()
            .await
            .with_context(|| format!("flushing {}", self.path.display()))?;
        // sync_all() is the difference between "flushed to kernel
        // page cache" (lost on power-fail) and "fsynced to disk"
        // (durable). The journal is best-effort and append-only, so
        // a sync per record is acceptable overhead; without it, an
        // OS crash drops every record since the last writeback.
        f.sync_all()
            .await
            .with_context(|| format!("syncing {}", self.path.display()))?;
        Ok(())
    }

    /// Iterate records on disk. Skips malformed lines silently (a
    /// half-written line during a crash shouldn't poison subsequent
    /// queries). For very large journals this is O(n) — fine for the
    /// `query_activity` use case which always filters by time anyway.
    pub fn read_all(&self) -> Result<Vec<ActivityRecord>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let raw = std::fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        let mut out = Vec::new();
        for line in raw.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(rec) = serde_json::from_str::<ActivityRecord>(line) {
                out.push(rec);
            }
        }
        Ok(out)
    }
}

/// Truncate `s` to at most `max` chars, respecting UTF-8 boundaries.
/// Appends an ellipsis when truncation occurs.
fn truncate_utf8(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out = String::with_capacity(max);
    for (i, ch) in s.chars().enumerate() {
        if i >= max {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}
