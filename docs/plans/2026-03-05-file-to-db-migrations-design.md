# File-to-DB Migrations Design

## Problem

5 subsystems use file-based JSON storage with fs2 locks while the rest of the system uses MemoryDB (SQLite). Same class of bug as the cron migration: ENOENT on fresh installs, ad-hoc locking, dual source of truth, no transactional safety.

## Migrations

### 1. Pairing State

Tables: `pairing_allowlist(channel, sender_id, created_at)`, `pairing_pending(channel, sender_id, code, created_at)`, `pairing_failed_attempts(client_id, attempted_at)`. PairingStore takes `Arc<MemoryDB>`. `is_sender_paired()` reads DB. Remove `~/.oxicrab/pairing/`.

### 2. OAuth Tokens

Table: `oauth_tokens(provider PK, access_token, refresh_token, expires_at, extra_json, updated_at)`. Both providers take `Option<Arc<MemoryDB>>`. `extra_json` for Google's client_id/secret/uri/scopes. Remove `.oauth-cache.json`, `google_tokens.json`.

### 3. Obsidian Cache

Tables: `obsidian_sync_state(vault_name, file_path, content_hash, last_synced_at, size)`, `obsidian_write_queue(id, vault_name, path, content, operation, queued_at, pre_write_hash)`. Cached markdown files stay on disk. Remove `sync_state.json`, `write_queue.json`.

### 4. Media File Tracking

No new tables. Register media in existing `workspace_files` with category `media`. Change `cleanup_old_media()` to query DB instead of scanning filesystem.

### 5. Subagent Activity Logs

Table: `subagent_logs(id, task_id, timestamp, event_type, content, metadata)`. Auto-purge old task runs. Remove `~/.oxicrab/logs/`.
