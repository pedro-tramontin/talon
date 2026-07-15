//! Canonical SQL schema. Every `CREATE TABLE` / `CREATE INDEX` statement
//! the migrations runner will execute lives here, versioned.
//!
//! **Convention:** schema versions are append-only. To change a table,
//! write a new migration (`.sql` snippet) that `ALTER`s it. Never edit a
//! past migration. If a past migration was wrong, write a new one that
//! fixes it.
//!
//! The `notes` table is intentionally deferred to a future migration —
//! the Part A scope doesn't include it; Part B §2.8 adds it.

/// The "current" schema version. Every time we add a new migration,
/// bump this number. Bumping tells the runner: "after running
/// migrations up to and including this version, this is what the DB
/// should look like."
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Migration 001 — initial schema. Creates every table the rest of
/// the codebase reads from. Idempotent: uses `CREATE TABLE IF NOT EXISTS`
/// and `CREATE INDEX IF NOT EXISTS` so re-running on an already-migrated
/// DB is a no-op.
pub const MIGRATION_001_INITIAL: &str = r#"
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS projects (
    id              TEXT PRIMARY KEY NOT NULL,
    name            TEXT NOT NULL,
    target_host     TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    db_filename     TEXT NOT NULL,
    talon_version   TEXT NOT NULL,
    ca_fingerprint  TEXT,
    settings_json   TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS exchanges (
    id              TEXT PRIMARY KEY NOT NULL,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    timestamp       TEXT NOT NULL,
    duration_ns     INTEGER NOT NULL DEFAULT 0,
    summary         TEXT NOT NULL,
    scope_state     TEXT NOT NULL CHECK (scope_state IN ('in_scope','out_of_scope','blocked','unscoped')),
    notes           TEXT NOT NULL DEFAULT '',
    starred         INTEGER NOT NULL DEFAULT 0,
    blocked_reason  TEXT,
    request_json    TEXT NOT NULL,
    response_json   TEXT
);

CREATE INDEX IF NOT EXISTS idx_exchanges_project_ts
    ON exchanges (project_id, timestamp DESC);

CREATE INDEX IF NOT EXISTS idx_exchanges_project_starred
    ON exchanges (project_id, starred)
    WHERE starred = 1;

CREATE TABLE IF NOT EXISTS tags (
    id      TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name    TEXT NOT NULL,
    color   TEXT,
    UNIQUE (project_id, name)
);

CREATE TABLE IF NOT EXISTS exchange_tags (
    exchange_id  TEXT NOT NULL REFERENCES exchanges(id) ON DELETE CASCADE,
    tag_id       TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (exchange_id, tag_id)
);

CREATE INDEX IF NOT EXISTS idx_exchange_tags_tag
    ON exchange_tags (tag_id);

-- Full-text search over the URL, method, headers, and response body.
-- FTS5 is built into SQLite when the `bundled` feature is on
-- (which pulls in `modern_sqlite`); no extra rusqlite feature needed.
-- The contentless table is fed manually by §2.9.
CREATE VIRTUAL TABLE IF NOT EXISTS exchange_fts USING fts5(
    url,
    method,
    request_headers,
    response_headers,
    request_body,
    response_body,
    notes,
    content='',                              -- contentless table; we feed it manually
    tokenize='unicode61 remove_diacritics 2' -- good default for HTTP text
);
"#;
