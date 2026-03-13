use anyhow::{Result, bail};
use rusqlite::Connection;

const MIGRATION_0001_BASE: &str = include_str!("migrations/0001_base.sql");

pub fn apply_migrations(conn: &Connection) -> Result<()> {
    if user_version(conn)? < 1 {
        conn.execute_batch(MIGRATION_0001_BASE)?;
        conn.execute("PRAGMA user_version = 1", [])?;
    }

    if user_version(conn)? < 2 {
        add_column_if_missing(conn, "llm_cost_log", "request_id", "TEXT")?;
        add_column_if_missing(conn, "intent_metrics", "request_id", "TEXT")?;
        add_column_if_missing(conn, "memory_access_log", "request_id", "TEXT")?;
        conn.execute("PRAGMA user_version = 2", [])?;
    }

    if user_version(conn)? < 3 {
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_memory_entries_source_key \
             ON memory_entries(source_key, created_at);",
        )?;
        conn.execute("PRAGMA user_version = 3", [])?;
    }

    if user_version(conn)? < 4 {
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_sessions_updated ON sessions(updated_at);",
        )?;
        conn.execute("PRAGMA user_version = 4", [])?;
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
mod tests {
    use super::*;

    #[test]
    fn applies_latest_user_version() {
        let conn = Connection::open_in_memory().unwrap();
        apply_migrations(&conn).unwrap();
        let v: u32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(v, 4);
    }

    #[test]
    fn adds_request_id_columns_when_missing() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE llm_cost_log (
                id INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL DEFAULT (datetime('now')),
                model TEXT NOT NULL,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                cost_cents REAL NOT NULL,
                caller TEXT NOT NULL DEFAULT 'main'
            );
             CREATE TABLE intent_metrics (
                id INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL DEFAULT (datetime('now')),
                event_type TEXT NOT NULL,
                intent_method TEXT,
                semantic_score REAL,
                detection_layer TEXT,
                message_preview TEXT
            );
             CREATE TABLE memory_access_log (
                id INTEGER PRIMARY KEY,
                query TEXT NOT NULL,
                search_type TEXT NOT NULL,
                result_count INTEGER NOT NULL,
                top_score REAL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );",
        )
        .unwrap();
        apply_migrations(&conn).unwrap();

        for table in ["llm_cost_log", "intent_metrics", "memory_access_log"] {
            let mut stmt = conn
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap();
            let cols: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap();
            assert!(cols.iter().any(|c| c == "request_id"));
        }
    }

    #[test]
    fn test_migration_v3_adds_source_key_index() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATION_0001_BASE).unwrap();
        conn.execute("PRAGMA user_version = 2", []).unwrap();
        apply_migrations(&conn).unwrap();
        assert!(user_version(&conn).unwrap() >= 3);
        // Verify index exists
        let mut stmt = conn.prepare("PRAGMA index_list('memory_entries')").unwrap();
        let indexes: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(
            indexes.iter().any(|n| n.contains("source_key")),
            "source_key index should exist, found: {indexes:?}",
        );
    }
}
