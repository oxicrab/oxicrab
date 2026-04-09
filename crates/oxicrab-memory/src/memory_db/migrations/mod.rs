use anyhow::{Result, bail};
use rusqlite::Connection;

const MIGRATION_0001_BASE: &str = include_str!("0001_base.sql");

/// Run a migration step inside a transaction. Rolls back on failure so the
/// database is never left in a partially-migrated state.
fn run_migration(conn: &Connection, version: u32, f: impl FnOnce() -> Result<()>) -> Result<()> {
    conn.execute_batch("BEGIN;")?;
    match f() {
        Ok(()) => {
            conn.execute_batch(&format!("PRAGMA user_version = {version}; COMMIT;"))?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK;");
            Err(e)
        }
    }
}

pub fn apply_migrations(conn: &Connection) -> Result<()> {
    if user_version(conn)? < 1 {
        run_migration(conn, 1, || {
            conn.execute_batch(MIGRATION_0001_BASE)?;
            Ok(())
        })?;
    }

    if user_version(conn)? < 2 {
        run_migration(conn, 2, || {
            add_column_if_missing(conn, "llm_cost_log", "request_id", "TEXT")?;
            add_column_if_missing(conn, "intent_metrics", "request_id", "TEXT")?;
            add_column_if_missing(conn, "memory_access_log", "request_id", "TEXT")?;
            Ok(())
        })?;
    }

    if user_version(conn)? < 3 {
        run_migration(conn, 3, || {
            conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_memory_entries_source_key \
                 ON memory_entries(source_key, created_at);",
            )?;
            Ok(())
        })?;
    }

    if user_version(conn)? < 4 {
        run_migration(conn, 4, || {
            conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_sessions_updated ON sessions(updated_at);",
            )?;
            Ok(())
        })?;
    }

    if user_version(conn)? < 5 {
        run_migration(conn, 5, || {
            conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS rss_feeds (
                id TEXT PRIMARY KEY,
                url TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                site_url TEXT,
                last_fetched_at_ms INTEGER,
                last_error TEXT,
                consecutive_failures INTEGER NOT NULL DEFAULT 0,
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at_ms INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS rss_articles (
                id TEXT PRIMARY KEY,
                feed_id TEXT NOT NULL REFERENCES rss_feeds(id) ON DELETE CASCADE,
                url TEXT NOT NULL UNIQUE,
                title TEXT NOT NULL,
                author TEXT,
                published_at_ms INTEGER,
                fetched_at_ms INTEGER NOT NULL,
                description TEXT,
                full_content TEXT,
                summary TEXT,
                status TEXT NOT NULL DEFAULT 'new',
                read INTEGER NOT NULL DEFAULT 0,
                created_at_ms INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS rss_article_tags (
                article_id TEXT NOT NULL REFERENCES rss_articles(id) ON DELETE CASCADE,
                tag TEXT NOT NULL,
                PRIMARY KEY (article_id, tag)
            );

            CREATE TABLE IF NOT EXISTS rss_profile (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                interests TEXT NOT NULL,
                onboarding_state TEXT NOT NULL DEFAULT 'needs_profile',
                cron_job_id TEXT,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS rss_model (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                feature_index TEXT NOT NULL,
                mu BLOB NOT NULL,
                sigma BLOB NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_rss_articles_feed ON rss_articles(feed_id, status);
            CREATE INDEX IF NOT EXISTS idx_rss_articles_status ON rss_articles(status, created_at_ms);
            CREATE INDEX IF NOT EXISTS idx_rss_articles_published ON rss_articles(published_at_ms);
            CREATE INDEX IF NOT EXISTS idx_rss_article_tags_tag ON rss_article_tags(tag);",
        )?;
            Ok(())
        })?;
    }

    if user_version(conn)? < 6 {
        run_migration(conn, 6, || {
            conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cron_execution_traces (
                id TEXT PRIMARY KEY,
                job_id TEXT NOT NULL,
                job_name TEXT NOT NULL,
                started_at TEXT NOT NULL,
                completed_at TEXT,
                status TEXT NOT NULL DEFAULT 'running',
                events TEXT NOT NULL DEFAULT '[]',
                summary TEXT,
                token_count INTEGER DEFAULT 0,
                tool_call_count INTEGER DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_cron_traces_job ON cron_execution_traces(job_id);
            CREATE INDEX IF NOT EXISTS idx_cron_traces_started ON cron_execution_traces(started_at);",
        )?;
            Ok(())
        })?;
    }

    if user_version(conn)? < 7 {
        run_migration(conn, 7, || {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS collections (
                name         TEXT PRIMARY KEY,
                description  TEXT NOT NULL DEFAULT '',
                schema_json  TEXT NOT NULL,
                created_at   TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS collection_records (
                id              TEXT PRIMARY KEY,
                collection_name TEXT NOT NULL REFERENCES collections(name) ON DELETE CASCADE,
                data_json       TEXT NOT NULL,
                created_at      TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_collection_records_name
                ON collection_records(collection_name);",
            )?;
            Ok(())
        })?;
    }

    Ok(())
}

/// Ensure optional FTS5 objects exist. Returns `true` when FTS5 is available.
pub fn ensure_fts_objects(conn: &Connection) -> Result<bool> {
    if conn
        .execute(
            "CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts
             USING fts5(
                content,
                source_key,
                content='memory_entries',
                content_rowid='id'
             )",
            [],
        )
        .is_err()
    {
        return Ok(false);
    }

    conn.execute(
        "CREATE TRIGGER IF NOT EXISTS mem_ai AFTER INSERT ON memory_entries BEGIN
            INSERT INTO memory_fts(rowid, content, source_key)
            VALUES (new.id, new.content, new.source_key);
        END",
        [],
    )?;

    conn.execute(
        "CREATE TRIGGER IF NOT EXISTS mem_ad AFTER DELETE ON memory_entries BEGIN
            INSERT INTO memory_fts(memory_fts, rowid, content, source_key)
            VALUES ('delete', old.id, old.content, old.source_key);
        END",
        [],
    )?;

    conn.execute(
        "CREATE TRIGGER IF NOT EXISTS mem_au AFTER UPDATE ON memory_entries BEGIN
            INSERT INTO memory_fts(memory_fts, rowid, content, source_key)
            VALUES ('delete', old.id, old.content, old.source_key);
            INSERT INTO memory_fts(rowid, content, source_key)
            VALUES (new.id, new.content, new.source_key);
        END",
        [],
    )?;

    Ok(true)
}

fn add_column_if_missing(
    conn: &Connection,
    table: &'static str,
    column: &'static str,
    definition: &'static str,
) -> Result<()> {
    ensure_allowed_column_addition(table, column, definition)?;

    let mut stmt = conn.prepare("SELECT name FROM pragma_table_info(?1)")?;
    let columns = stmt.query_map([table], |row| row.get::<_, String>(0))?;
    for c in columns {
        if c? == column {
            return Ok(());
        }
    }

    let alter = format!("ALTER TABLE \"{table}\" ADD COLUMN \"{column}\" {definition}");
    conn.execute(&alter, [])?;
    Ok(())
}

fn user_version(conn: &Connection) -> Result<u32> {
    let current: u32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    Ok(current)
}

fn ensure_allowed_column_addition(
    table: &'static str,
    column: &'static str,
    definition: &'static str,
) -> Result<()> {
    if matches!(
        (table, column, definition),
        (
            "llm_cost_log" | "intent_metrics" | "memory_access_log",
            "request_id",
            "TEXT"
        )
    ) {
        return Ok(());
    }
    bail!("Unsupported migration column addition: {table}.{column} {definition}")
}

#[cfg(test)]
mod tests;
