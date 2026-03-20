CREATE TABLE IF NOT EXISTS memory_sources (
    source_key TEXT PRIMARY KEY,
    mtime_ns INTEGER NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS memory_entries (
    id INTEGER PRIMARY KEY,
    source_key TEXT NOT NULL,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (source_key, content_hash)
);

CREATE TABLE IF NOT EXISTS memory_embeddings (
    entry_id INTEGER PRIMARY KEY REFERENCES memory_entries(id) ON DELETE CASCADE,
    embedding BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS memory_access_log (
    id INTEGER PRIMARY KEY,
    query TEXT NOT NULL,
    search_type TEXT NOT NULL,
    result_count INTEGER NOT NULL,
    top_score REAL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    request_id TEXT
);

CREATE TABLE IF NOT EXISTS memory_search_hits (
    id INTEGER PRIMARY KEY,
    access_log_id INTEGER NOT NULL REFERENCES memory_access_log(id) ON DELETE CASCADE,
    source_key TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_search_hits_source ON memory_search_hits(source_key);
CREATE INDEX IF NOT EXISTS idx_search_hits_log_id ON memory_search_hits(access_log_id);

CREATE TABLE IF NOT EXISTS llm_cost_log (
    id INTEGER PRIMARY KEY,
    timestamp TEXT NOT NULL DEFAULT (datetime('now')),
    model TEXT NOT NULL,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cost_cents REAL NOT NULL,
    caller TEXT NOT NULL DEFAULT 'main',
    request_id TEXT
);
CREATE INDEX IF NOT EXISTS idx_cost_log_date ON llm_cost_log(timestamp);
CREATE INDEX IF NOT EXISTS idx_cost_log_model ON llm_cost_log(model);

CREATE TABLE IF NOT EXISTS scheduled_task_dlq (
    id INTEGER PRIMARY KEY,
    job_id TEXT NOT NULL,
    job_name TEXT NOT NULL,
    payload TEXT NOT NULL,
    error_message TEXT NOT NULL,
    failed_at TEXT NOT NULL DEFAULT (datetime('now')),
    retry_count INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'pending_retry'
);

-- Vestigial: this table has no INSERT statements; only CREATE + purge remain.
-- Kept for backward compatibility with existing databases.
CREATE TABLE IF NOT EXISTS intent_metrics (
    id INTEGER PRIMARY KEY,
    timestamp TEXT NOT NULL DEFAULT (datetime('now')),
    event_type TEXT NOT NULL,
    intent_method TEXT,
    semantic_score REAL,
    detection_layer TEXT,
    message_preview TEXT,
    request_id TEXT
);
CREATE INDEX IF NOT EXISTS idx_intent_metrics_date ON intent_metrics(timestamp);
CREATE INDEX IF NOT EXISTS idx_intent_metrics_type ON intent_metrics(event_type);

CREATE TABLE IF NOT EXISTS workspace_files (
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
);
CREATE INDEX IF NOT EXISTS idx_workspace_files_category ON workspace_files(category);
CREATE INDEX IF NOT EXISTS idx_workspace_files_created ON workspace_files(created_at);

CREATE TABLE IF NOT EXISTS complexity_routing_log (
    id INTEGER PRIMARY KEY,
    request_id TEXT NOT NULL,
    timestamp TEXT NOT NULL DEFAULT (datetime('now')),
    composite_score REAL NOT NULL,
    resolved_tier TEXT NOT NULL,
    resolved_model TEXT,
    forced TEXT,
    channel TEXT,
    message_preview TEXT
);
CREATE INDEX IF NOT EXISTS idx_complexity_log_ts ON complexity_routing_log(timestamp);
CREATE INDEX IF NOT EXISTS idx_complexity_log_req ON complexity_routing_log(request_id);

CREATE TABLE IF NOT EXISTS cron_jobs (
    id               TEXT PRIMARY KEY,
    name             TEXT NOT NULL,
    enabled          INTEGER NOT NULL DEFAULT 1,
    schedule_type    TEXT NOT NULL,
    at_ms            INTEGER,
    every_ms         INTEGER,
    cron_expr        TEXT,
    cron_tz          TEXT,
    event_pattern    TEXT,
    event_channel    TEXT,
    payload_kind     TEXT NOT NULL DEFAULT 'agent_turn',
    payload_message  TEXT NOT NULL DEFAULT '',
    agent_echo       INTEGER NOT NULL DEFAULT 1,
    next_run_at_ms   INTEGER,
    last_run_at_ms   INTEGER,
    last_status      TEXT,
    last_error       TEXT,
    run_count        INTEGER NOT NULL DEFAULT 0,
    last_fired_at_ms INTEGER,
    created_at_ms    INTEGER NOT NULL,
    updated_at_ms    INTEGER NOT NULL,
    delete_after_run INTEGER NOT NULL DEFAULT 0,
    expires_at_ms    INTEGER,
    max_runs         INTEGER,
    cooldown_secs    INTEGER,
    max_concurrent   INTEGER
);
CREATE INDEX IF NOT EXISTS idx_cron_jobs_enabled_next ON cron_jobs(enabled, next_run_at_ms);

CREATE TABLE IF NOT EXISTS pairing_allowlist (
    channel    TEXT NOT NULL,
    sender_id  TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (channel, sender_id)
);

CREATE TABLE IF NOT EXISTS pairing_pending (
    channel    TEXT NOT NULL,
    sender_id  TEXT NOT NULL,
    code       TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS pairing_failed_attempts (
    client_id    TEXT NOT NULL,
    attempted_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_pairing_failed_client ON pairing_failed_attempts(client_id);

CREATE TABLE IF NOT EXISTS cron_job_targets (
    job_id   TEXT NOT NULL REFERENCES cron_jobs(id) ON DELETE CASCADE,
    channel  TEXT NOT NULL,
    target   TEXT NOT NULL,
    PRIMARY KEY (job_id, channel, target)
);

CREATE TABLE IF NOT EXISTS sessions (
    key TEXT PRIMARY KEY,
    data TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS oauth_tokens (
    provider      TEXT PRIMARY KEY,
    access_token  TEXT NOT NULL,
    refresh_token TEXT,
    expires_at    INTEGER NOT NULL,
    extra_json    TEXT,
    updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS obsidian_sync_state (
    vault_name     TEXT NOT NULL,
    file_path      TEXT NOT NULL,
    content_hash   TEXT NOT NULL,
    last_synced_at INTEGER NOT NULL,
    size           INTEGER NOT NULL,
    PRIMARY KEY (vault_name, file_path)
);

CREATE TABLE IF NOT EXISTS obsidian_write_queue (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    vault_name     TEXT NOT NULL,
    path           TEXT NOT NULL,
    content        TEXT NOT NULL,
    operation      TEXT NOT NULL,
    queued_at      INTEGER NOT NULL,
    pre_write_hash TEXT
);
CREATE INDEX IF NOT EXISTS idx_obsidian_queue_vault ON obsidian_write_queue(vault_name);

CREATE TABLE IF NOT EXISTS subagent_logs (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id    TEXT NOT NULL,
    timestamp  TEXT NOT NULL DEFAULT (datetime('now')),
    event_type TEXT NOT NULL,
    content    TEXT NOT NULL,
    metadata   TEXT
);
CREATE INDEX IF NOT EXISTS idx_subagent_logs_task ON subagent_logs(task_id);
