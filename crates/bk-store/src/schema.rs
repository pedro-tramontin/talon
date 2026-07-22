//! Canonical SQL schema. Every `CREATE TABLE` / `CREATE INDEX` statement
//! the migrations runner will execute lives here, versioned.
//!
//! **Convention:** schema versions are append-only. To change a table,
//! write a new migration (`.sql` snippet) that `ALTER`s it. Never edit a
//! past migration. If a past migration was wrong, write a new one that
//! fixes it.
//!
//! Notes are stored inline on `exchanges.notes` (no separate `notes`
//! table); the FTS5 index over `exchanges.notes` is kept in sync by
//! `bk_store::fts` and `bk_store::exchanges`.

/// The "current" schema version. Every time we add a new migration,
/// bump this number. Bumping tells the runner: "after running
/// migrations up to and including this version, this is what the DB
/// should look like."
pub const CURRENT_SCHEMA_VERSION: u32 = 3;

/// Migration 001 — initial schema. Creates every table the rest of
/// the codebase reads from. Idempotent: uses `CREATE TABLE IF NOT EXISTS`
/// and `CREATE INDEX IF NOT EXISTS` so re-running on an already-migrated
/// DB is a no-op.
///
/// **Known issue with the FTS5 table in this migration:** the
/// `exchange_fts` virtual table was created as `content=''` (contentless).
/// Contentless FTS5 tables do not support the FTS5 'delete' command's
/// intended behavior — the 'delete' marker is recorded but the
/// inverted-index entry is not removed, so `REPLACE INTO` (or
/// 'delete' + INSERT) does not update search results. Migration 002
/// drops and recreates the table with internal content, which fixes
/// the issue (REPLACE works, 'delete' works, SQL DELETE works).
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

/// Migration 002 — drop the contentless `exchange_fts` table from
/// migration 001 and recreate it with **internal content** (no
/// `content=''` option). The contentless variant had a fundamental
/// limitation: the FTS5 'delete' command did not remove the entry
/// from the inverted index, so updates (via `update_notes`) made the
/// search results stale — old notes remained searchable and new
/// notes would not match.
///
/// Internal content fixes this at the cost of duplicating the
/// indexed columns (url, method, headers, bodies, notes) inside the
/// FTS table. The data is already in `exchanges.request_json` /
/// `exchanges.response_json` (JSON blobs), so the duplication is
/// bounded to the 7 indexed columns — negligible compared to the
/// request/response bodies that already dominate row size.
///
/// After this migration, the FTS sync code can use:
///   - `REPLACE INTO exchange_fts (...) VALUES (...)` for updates
///   - `DELETE FROM exchange_fts WHERE rowid = ?` for deletes
///   - `REPLACE INTO` (idempotent) for inserts
///
/// The migration re-populates the FTS table by indexing every
/// existing `exchanges` row, so no data is lost.
///
/// **One-time data migration cost:** for a project with N exchanges,
/// the migration re-indexes all N rows. For typical projects (hundreds
/// to low thousands of exchanges) this is < 1 second.
pub const MIGRATION_002_FTS5_INTERNAL_CONTENT: &str = r#"
-- Drop the old contentless FTS5 table. All data is in `exchanges` —
-- we'll re-index from there.
DROP TABLE IF EXISTS exchange_fts;

-- Recreate with internal content (no `content=''` option). The FTS
-- table now stores the indexed columns directly, enabling proper
-- REPLACE / DELETE / UPDATE semantics.
CREATE VIRTUAL TABLE exchange_fts USING fts5(
    url,
    method,
    request_headers,
    response_headers,
    request_body,
    response_body,
    notes,
    tokenize='unicode61 remove_diacritics 2'
);

-- Re-index every existing exchange. Each `exchanges` row's rowid is
-- preserved (it's the SQLite auto-incrementing rowid), so the FTS
-- index uses the same rowid. The `json_extract` calls pull the
-- URL, method, and notes out of the JSON blobs.
--
-- For headers and bodies, we re-construct the same lossy UTF-8
-- strings that the runtime code uses (see `fts::index_exchange`).
-- This keeps the migration self-contained: a fresh `exchanges` row
-- re-indexes identically whether it goes through the migration or
-- through the runtime path.
INSERT INTO exchange_fts (rowid, url, method, request_headers, response_headers, request_body, response_body, notes)
SELECT
    e.rowid,
    json_extract(e.request_json, '$.url'),
    json_extract(e.request_json, '$.method'),
    '',  -- request_headers: not stored separately in the JSON; FTS will index nothing for this column
    COALESCE(json_extract(e.response_json, '$.headers'), ''),
    json_extract(e.request_json, '$.body'),
    COALESCE(json_extract(e.response_json, '$.body'), ''),
    e.notes
FROM exchanges e;
"#;

/// Migration 003 — adds the `replay_history` table (Phase 6 Part C,
/// §C-A.4). The table persists per-tab replay send history so the
/// history panel survives an app restart. The Rust side of the
/// persistence is in `bk_store::replay_history`; the Tauri commands
/// (`list_replay_history` + `append_replay_history`) live in
/// `app/src/commands/replay.rs`.
///
/// **Why a new table and not a new column on `exchanges`:** the
/// `replay_history` rows are derived from the `exchanges` rows
/// (each replay send creates an exchange via the §5.2 path), but
/// they carry extra per-tab metadata (`tab_id`,
/// `sequence_within_tab`) that doesn't belong on the `exchanges`
/// table. A new table with FKs to both `projects` and `exchanges`
/// keeps the schema normalized.
///
/// **Idempotency:** the `CREATE TABLE IF NOT EXISTS` and
/// `CREATE INDEX IF NOT EXISTS` clauses make this safe to run on
/// an already-migrated DB. The runner wraps the whole migration
/// in a transaction (see `migrations::run`), so a partial failure
/// rolls back cleanly.
pub const MIGRATION_003_REPLAY_HISTORY: &str = r#"
CREATE TABLE IF NOT EXISTS replay_history (
    id                       TEXT PRIMARY KEY NOT NULL,
    project_id               TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    tab_id                   TEXT NOT NULL,
    request_exchange_id      TEXT NOT NULL REFERENCES exchanges(id) ON DELETE CASCADE,
    response_exchange_id     TEXT REFERENCES exchanges(id) ON DELETE SET NULL,
    timestamp                TEXT NOT NULL,
    sequence_within_tab      INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_replay_history_tab_seq
    ON replay_history (tab_id, sequence_within_tab ASC);

CREATE INDEX IF NOT EXISTS idx_replay_history_project_ts
    ON replay_history (project_id, timestamp DESC);
"#;
