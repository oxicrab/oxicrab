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
            // intent_metrics no longer exists on fresh DBs; on
            // upgrades from v1, migration 12 drops the whole table.
            // The add-column step is harmless on either path.
            if conn
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE type='table' AND name='intent_metrics'",
                    [],
                    |_| Ok(()),
                )
                .is_ok()
            {
                add_column_if_missing(conn, "intent_metrics", "request_id", "TEXT")?;
            }
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

    if user_version(conn)? < 8 {
        run_migration(conn, 8, || {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS tool_reflections (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                request_id      TEXT NOT NULL,
                tool_name       TEXT NOT NULL,
                action          TEXT,
                attempt_number  INTEGER NOT NULL,
                error_excerpt   TEXT NOT NULL,
                hypothesis      TEXT NOT NULL,
                retry_strategy  TEXT NOT NULL,
                next_outcome    TEXT,
                created_at_ms   INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_tool_reflections_tool
                ON tool_reflections(tool_name, action);
            CREATE INDEX IF NOT EXISTS idx_tool_reflections_request
                ON tool_reflections(request_id);

            CREATE TABLE IF NOT EXISTS skills_index (
                path             TEXT PRIMARY KEY,
                name             TEXT NOT NULL,
                description      TEXT NOT NULL,
                embedding        BLOB NOT NULL,
                file_sha256      TEXT NOT NULL,
                use_count        INTEGER NOT NULL DEFAULT 0,
                last_used_ms     INTEGER,
                created_at_ms    INTEGER NOT NULL,
                last_indexed_ms  INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_skills_index_name ON skills_index(name);
            CREATE INDEX IF NOT EXISTS idx_skills_index_use ON skills_index(use_count, last_used_ms);",
            )?;
            Ok(())
        })?;
    }

    if user_version(conn)? < 9 {
        run_migration(conn, 9, || {
            // Track which embedding model produced each row's embedding.
            // When the operator changes models, all rows with a different
            // model id are bulk-invalidated and re-embedded by the next
            // `SkillIndex::rebuild`. Empty default ('') matches "unknown
            // model" — pre-existing rows from migration 8 are eligible
            // for invalidation when the operator first sets the
            // configured model id.
            add_column_if_missing(
                conn,
                "skills_index",
                "embedding_model_id",
                "TEXT NOT NULL DEFAULT ''",
            )?;
            Ok(())
        })?;
    }

    if user_version(conn)? < 10 {
        run_migration(conn, 10, || {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS trajectory_events (
                    id            INTEGER PRIMARY KEY AUTOINCREMENT,
                    session_id    TEXT NOT NULL,
                    turn_index    INTEGER NOT NULL,
                    event_type    TEXT NOT NULL,
                    tool_name     TEXT,
                    action        TEXT,
                    latency_ms    INTEGER,
                    is_error      INTEGER,
                    created_at_ms INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_traj_events_session_turn
                    ON trajectory_events(session_id, turn_index);
                CREATE INDEX IF NOT EXISTS idx_traj_events_created
                    ON trajectory_events(created_at_ms);

                CREATE TABLE IF NOT EXISTS trajectory_summaries (
                    session_id     TEXT PRIMARY KEY,
                    summary        TEXT NOT NULL,
                    fingerprint    TEXT,
                    occurrences    INTEGER NOT NULL DEFAULT 0,
                    candidate_name TEXT,
                    candidate_desc TEXT,
                    candidate_conf REAL,
                    created_at_ms  INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS skill_refinements (
                    id            INTEGER PRIMARY KEY AUTOINCREMENT,
                    skill_name    TEXT NOT NULL,
                    confidence    REAL NOT NULL,
                    reason        TEXT NOT NULL,
                    bytes_before  INTEGER NOT NULL,
                    bytes_after   INTEGER NOT NULL,
                    version_after TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_skill_refinements_name
                    ON skill_refinements(skill_name, created_at_ms);",
            )?;
            Ok(())
        })?;
    }

    if user_version(conn)? < 11 {
        run_migration(conn, 11, || {
            // Structured claims with confidence + status + evidence.
            // Two tables — `claims` for the head record, `claim_evidence`
            // for the 1:N pointer list (different evidence kinds need
            // their own schemas; storing as JSON would block proper
            // querying).
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS claims (
                    id              INTEGER PRIMARY KEY AUTOINCREMENT,
                    text            TEXT NOT NULL,
                    confidence      REAL NOT NULL DEFAULT 0.5,
                    status          TEXT NOT NULL DEFAULT 'open',
                    last_seen_ms    INTEGER NOT NULL,
                    created_at_ms   INTEGER NOT NULL,
                    updated_at_ms   INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_claims_status
                    ON claims(status, last_seen_ms);
                CREATE INDEX IF NOT EXISTS idx_claims_confidence
                    ON claims(confidence);

                CREATE TABLE IF NOT EXISTS claim_evidence (
                    id            INTEGER PRIMARY KEY AUTOINCREMENT,
                    claim_id      INTEGER NOT NULL REFERENCES claims(id) ON DELETE CASCADE,
                    pointer_kind  TEXT NOT NULL,
                    pointer_value TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_claim_evidence_claim
                    ON claim_evidence(claim_id);",
            )?;
            Ok(())
        })?;
    }

    if user_version(conn)? < 12 {
        run_migration(conn, 12, || {
            // Drop the dead `cost_cents` column from llm_cost_log
            // (token counts are the ground truth) and drop the
            // never-written `intent_metrics` table. Both are
            // conditional so the migration is idempotent on fresh
            // databases that were created without those objects.
            let has_cost_cents = conn
                .prepare(
                    "SELECT 1 FROM pragma_table_info('llm_cost_log') WHERE name = 'cost_cents'",
                )?
                .exists([])?;
            if has_cost_cents {
                conn.execute_batch("ALTER TABLE llm_cost_log DROP COLUMN cost_cents;")?;
            }
            conn.execute_batch("DROP TABLE IF EXISTS intent_metrics;")?;
            Ok(())
        })?;
    }

    if user_version(conn)? < 13 {
        run_migration(conn, 13, || {
            // Dedup tool_reflections on (request_id, tool_name,
            // action, attempt_number). Two code paths logging the
            // same reflection would otherwise leave duplicate rows,
            // and the natural key is what lookups use.
            conn.execute_batch(
                "CREATE UNIQUE INDEX IF NOT EXISTS uq_tool_reflections_natkey
                    ON tool_reflections(
                        request_id, tool_name,
                        IFNULL(action, ''),
                        attempt_number
                    );",
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
        ("llm_cost_log" | "memory_access_log", "request_id", "TEXT")
            | ("intent_metrics", "request_id", "TEXT")
            | (
                "skills_index",
                "embedding_model_id",
                "TEXT NOT NULL DEFAULT ''"
            )
    ) {
        return Ok(());
    }
    bail!("Unsupported migration column addition: {table}.{column} {definition}")
}

#[cfg(test)]
mod tests;
