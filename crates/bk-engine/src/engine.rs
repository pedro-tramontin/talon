//! The `Engine` is the long-lived orchestrator. One per Tauri app /
//! per axum process. Holds the `Projects` map, the global config
//! dir, and the two event buses (the full UI bus in `events` and the
//! narrower MCP bus in `mcp_events`). Everything the rest of Talon
//! needs goes through here.

#![allow(missing_docs)]

use crate::events::{channel as events_channel, EventReceiver, EventSender};
use crate::mcp_events::{channel as mcp_channel, McpEvent, McpEventReceiver, McpEventSender};
use crate::projects::Projects;
use bk_core::{ExchangeId, HttpExchange, Project, ProjectId, ProjectSettings, Tag, TagId};
use std::path::PathBuf;
use std::sync::Arc;

pub struct Engine {
    /// The Talon user-config dir. `~/.config/talon` on Linux,
    /// `~/Library/Application Support/talon` on macOS,
    /// `%APPDATA%\talon` on Windows.
    pub config_dir: PathBuf,
    /// Subdir for project files. Always `<config_dir>/projects`.
    pub projects_dir: PathBuf,
    pub(crate) projects: Projects,
    /// Sender for the full event bus (12 variants, the Tauri UI
    /// subscribes to this one). Cloning the sender is cheap; the
    /// engine holds the canonical copy. Subscribers call
    /// `subscribe_events()` to get a fresh `EventReceiver`.
    pub(crate) event_tx: EventSender,
    /// Sender for the MCP-narrowed event bus (5 variants, the MCP
    /// server and the internal agent subscribe to this one). Same
    /// shape as `event_tx`; separate so a slow MCP client doesn't
    /// block UI updates.
    pub(crate) mcp_event_tx: McpEventSender,
}

impl Engine {
    /// Initialize the engine at the given config dir, creating it if
    /// it doesn't exist. The projects subdir is also created.
    /// Both event buses are initialized here; subscribers register
    /// via `subscribe_events()` and `subscribe_mcp_events()` after
    /// construction.
    pub fn new(config_dir: impl Into<PathBuf>) -> std::io::Result<Self> {
        let config_dir = config_dir.into();
        std::fs::create_dir_all(&config_dir)?;
        let projects_dir = config_dir.join("projects");
        std::fs::create_dir_all(&projects_dir)?;
        // `Projects::new` takes ownership of the PathBuf. Clone first
        // so we can keep a copy in `self.projects_dir` for the UI to
        // display in the about dialog.
        let projects_dir_for_self = projects_dir.clone();
        let (event_tx, _event_rx) = events_channel();
        let (mcp_event_tx, _mcp_event_rx) = mcp_channel();
        Ok(Self {
            config_dir,
            projects_dir: projects_dir_for_self,
            projects: Projects::new(projects_dir),
            event_tx,
            mcp_event_tx,
        })
    }

    /// Subscribe to the full event bus. Each call returns a fresh,
    /// independent receiver — broadcast channels support any number
    /// of concurrent subscribers, and each one observes every event
    /// sent from that point forward. To fan out to N consumers
    /// (e.g., the Tauri UI + the MCP server + the agent), call
    /// this method N times.
    pub fn subscribe_events(&self) -> EventReceiver {
        self.event_tx.subscribe()
    }

    /// Subscribe to the MCP-narrowed event bus. Same semantics as
    /// `subscribe_events()`. The MCP server (§3.5b) and the
    /// internal agent (§3.5c) call this.
    pub fn subscribe_mcp_events(&self) -> McpEventReceiver {
        self.mcp_event_tx.subscribe()
    }

    /// Open a project. Creates the DB file on first open, and
    /// upserts the `projects` row so `exchanges` inserts (which have
    /// a FK to `projects(id)`) succeed. The upsert is idempotent —
    /// re-opening a project just refreshes its row.
    pub fn open_project(&self, project: &bk_core::Project) -> crate::Result<Arc<bk_store::DbPool>> {
        let pool = self.projects.open(project)?;
        bk_store::projects::upsert(&pool, &project.info)?;
        // Emit on the full bus (UI shows the project in the dropdown).
        // The MCP bus deliberately does NOT carry ProjectOpened — the
        // LLM doesn't need to know which projects are open in the
        // engine (it works on one at a time, by name). The bus
        // surfaces event with `send`; the `let _ =` swallows the
        // `SendError` if there are zero subscribers (which is normal
        // during tests or before the UI attaches).
        let _ = self
            .event_tx
            .send(crate::events::EngineEvent::ProjectOpened {
                project_id: project.info.id,
                db_filename: project.info.db_filename.clone(),
            });
        Ok(pool)
    }

    /// Close a project. Emits `ProjectClosed` on the full bus.
    pub fn close_project(&self, id: ProjectId) {
        self.projects.close(id);
        let _ = self
            .event_tx
            .send(crate::events::EngineEvent::ProjectClosed { project_id: id });
    }

    /// Insert an exchange into the given project. The project must be
    /// open (call `open_project` first).
    pub fn insert_exchange(
        &self,
        project_id: ProjectId,
        exchange: &HttpExchange,
    ) -> crate::Result<()> {
        let pool = self
            .projects
            .get(project_id)
            .ok_or_else(|| crate::EngineError::ProjectNotOpen(project_id.to_string()))?;
        bk_store::exchanges::insert(&pool, exchange)?;
        // Emit on the full bus (UI adds a row). v0.5: the
        // event carries the full `HttpExchange` so the UI
        // doesn't need a per-click `get_exchange` round-trip.
        // The wire payload is base64-encoded for bodies
        // (per `body_complete_data_serde` in `bk-core`).
        let id = exchange.meta.id;
        let _ = self
            .event_tx
            .send(crate::events::EngineEvent::ExchangeInserted {
                id,
                project_id,
                exchange: exchange.clone(),
            });
        // Demux to the MCP bus: smaller payload (no request/response
        // body — the LLM can call `talon_get_exchange` to fetch if
        // needed). The `status` is `None` for blocked exchanges
        // (response is None) and the actual status code otherwise.
        let method = exchange.request.method.as_str().to_owned();
        let url = exchange.request.url.as_str().to_owned();
        let status = exchange.response.as_ref().map(|r| r.status);
        let _ = self.mcp_event_tx.send(McpEvent::ExchangeCaptured {
            id,
            project_id,
            method,
            url,
            status,
        });
        Ok(())
    }

    /// Get an exchange by ID.
    pub fn get_exchange(
        &self,
        project_id: ProjectId,
        id: ExchangeId,
    ) -> crate::Result<Option<HttpExchange>> {
        let pool = self
            .projects
            .get(project_id)
            .ok_or_else(|| crate::EngineError::ProjectNotOpen(project_id.to_string()))?;
        bk_store::exchanges::get(&pool, id).map_err(Into::into)
    }

    /// List recent exchanges for a project.
    pub fn list_recent(
        &self,
        project_id: ProjectId,
        limit: u32,
    ) -> crate::Result<Vec<HttpExchange>> {
        let pool = self
            .projects
            .get(project_id)
            .ok_or_else(|| crate::EngineError::ProjectNotOpen(project_id.to_string()))?;
        bk_store::exchanges::list_recent(&pool, project_id, limit).map_err(Into::into)
    }

    /// Search exchanges by FTS5 query string. Returns matching
    /// exchange IDs ranked by FTS5's BM25 (best first).
    pub fn search(
        &self,
        project_id: ProjectId,
        query: &str,
        limit: u32,
    ) -> crate::Result<Vec<ExchangeId>> {
        let pool = self
            .projects
            .get(project_id)
            .ok_or_else(|| crate::EngineError::ProjectNotOpen(project_id.to_string()))?;
        bk_store::fts::search(&pool, project_id, query, limit).map_err(Into::into)
    }

    /// Delete an exchange by ID. Removes the FTS5 row, the
    /// `exchange_tags` join rows (CASCADE), and the exchange itself.
    /// Returns `ProjectNotOpen` if the project is not open.
    /// Emits `ExchangeDeleted` on the full bus (UI removes the row).
    pub fn delete_exchange(&self, project_id: ProjectId, id: ExchangeId) -> crate::Result<()> {
        let pool = self
            .projects
            .get(project_id)
            .ok_or_else(|| crate::EngineError::ProjectNotOpen(project_id.to_string()))?;
        bk_store::exchanges::delete(&pool, id)?;
        let _ = self
            .event_tx
            .send(crate::events::EngineEvent::ExchangeDeleted { id, project_id });
        Ok(())
    }

    /// Update the free-form notes on an exchange. The FTS5 row is
    /// re-indexed in the same transaction (the `update_notes`
    /// function in `bk_store::exchanges` is the one that was rewritten
    /// to fix the stale-FTS bug — see PR #11's Copilot review fix #4).
    pub fn update_notes(
        &self,
        project_id: ProjectId,
        id: ExchangeId,
        notes: &str,
    ) -> crate::Result<()> {
        let pool = self
            .projects
            .get(project_id)
            .ok_or_else(|| crate::EngineError::ProjectNotOpen(project_id.to_string()))?;
        bk_store::exchanges::update_notes(&pool, id, notes)?;
        let _ = self
            .event_tx
            .send(crate::events::EngineEvent::ExchangeNotesUpdated { id, project_id });
        Ok(())
    }

    /// Toggle the starred flag on an exchange. Used by the ⭐ button
    /// on each row in the UI's exchange list.
    pub fn set_starred(
        &self,
        project_id: ProjectId,
        id: ExchangeId,
        starred: bool,
    ) -> crate::Result<()> {
        let pool = self
            .projects
            .get(project_id)
            .ok_or_else(|| crate::EngineError::ProjectNotOpen(project_id.to_string()))?;
        bk_store::exchanges::set_starred(&pool, id, starred)?;
        let _ = self
            .event_tx
            .send(crate::events::EngineEvent::ExchangeStarredToggled {
                id,
                project_id,
                starred,
            });
        Ok(())
    }

    /// Create a tag (or return the existing one if the name is taken
    /// within the project). Idempotent. Returns the tag's ID.
    /// Emits `TagUpserted` on the full bus and demuxes to `TagAdded`
    /// on the MCP bus (the LLM learns about new tags via the
    /// narrower event).
    pub fn tag_upsert(
        &self,
        project_id: ProjectId,
        new: bk_store::tags::NewTag,
    ) -> crate::Result<TagId> {
        let pool = self
            .projects
            .get(project_id)
            .ok_or_else(|| crate::EngineError::ProjectNotOpen(project_id.to_string()))?;
        let id = bk_store::tags::upsert(&pool, project_id, &new)?;
        let name = new.name.clone();
        let _ = self.event_tx.send(crate::events::EngineEvent::TagUpserted {
            id,
            project_id,
            name: name.clone(),
        });
        let _ = self.mcp_event_tx.send(McpEvent::TagAdded {
            id,
            project_id,
            name,
        });
        Ok(id)
    }

    /// List all tags for a project, alphabetical by name. Used by
    /// the right-rail tag-picker to populate the available list.
    pub fn list_tags(&self, project_id: ProjectId) -> crate::Result<Vec<Tag>> {
        let pool = self
            .projects
            .get(project_id)
            .ok_or_else(|| crate::EngineError::ProjectNotOpen(project_id.to_string()))?;
        bk_store::tags::list(&pool, project_id).map_err(Into::into)
    }

    /// List the tags currently attached to a specific exchange.
    /// Used by the right-rail tag-picker to show the current state
    /// when the user has an exchange selected.
    pub fn list_tags_for_exchange(
        &self,
        project_id: ProjectId,
        exchange_id: ExchangeId,
    ) -> crate::Result<Vec<Tag>> {
        let pool = self
            .projects
            .get(project_id)
            .ok_or_else(|| crate::EngineError::ProjectNotOpen(project_id.to_string()))?;
        bk_store::tags::list_for_exchange(&pool, exchange_id).map_err(Into::into)
    }

    /// Attach a tag to an exchange. Idempotent (no-op if already attached).
    ///
    /// **No cross-project check at this layer.** `bk_store::tags::attach`
    /// is a plain `INSERT OR IGNORE INTO exchange_tags (exchange_id, tag_id)`
    /// — the engine passes the `tag_id` and `exchange_id` straight through
    /// without verifying they belong to the same project as the
    /// `project_id` we used to look up the pool. In normal usage the UI
    /// only ever passes a `tag_id` it got from `list_tags(project_id)`
    /// and an `exchange_id` from `list_recent(project_id, ...)`, so both
    /// will be from the same project and the invariant holds by
    /// construction. If we ever want to enforce it at the engine
    /// layer, the right place is a single SQL check before the
    /// `INSERT` in `bk_store::tags::attach` — e.g.
    /// `INSERT ... SELECT WHERE EXISTS (SELECT 1 FROM tags t, exchanges e
    /// WHERE t.id = ?1 AND e.id = ?2 AND t.project_id = e.project_id)`.
    /// That should be a separate PR; not bundled into this engine-wrapper
    /// change. (Copilot's review comment #5 on PR #15 flagged the
    /// earlier version of this comment for overpromising the invariant.)
    pub fn tag_attach(
        &self,
        project_id: ProjectId,
        tag_id: TagId,
        exchange_id: ExchangeId,
    ) -> crate::Result<()> {
        let pool = self
            .projects
            .get(project_id)
            .ok_or_else(|| crate::EngineError::ProjectNotOpen(project_id.to_string()))?;
        bk_store::tags::attach(&pool, tag_id, exchange_id)?;
        let _ = self.event_tx.send(crate::events::EngineEvent::TagAttached {
            tag_id,
            exchange_id,
            project_id,
        });
        Ok(())
    }

    /// Detach a tag from an exchange. No-op if not attached.
    pub fn tag_detach(
        &self,
        project_id: ProjectId,
        tag_id: TagId,
        exchange_id: ExchangeId,
    ) -> crate::Result<()> {
        let pool = self
            .projects
            .get(project_id)
            .ok_or_else(|| crate::EngineError::ProjectNotOpen(project_id.to_string()))?;
        bk_store::tags::detach(&pool, tag_id, exchange_id)?;
        let _ = self.event_tx.send(crate::events::EngineEvent::TagDetached {
            tag_id,
            exchange_id,
            project_id,
        });
        Ok(())
    }

    /// Number of currently open projects.
    pub fn open_count(&self) -> usize {
        self.projects.open_count()
    }

    /// List the IDs of all currently open projects. The UI uses this
    /// to populate the project switcher dropdown.
    pub fn open_ids(&self) -> Vec<ProjectId> {
        self.projects.open_ids()
    }

    /// Get the full `Project` (info + settings) for an open project.
    /// Returns `ProjectNotOpen` if the project is not open. Used by
    /// the Phase 6 Tauri commands (`list_scope_rules`,
    /// `list_match_replace_rules`, etc.) to read the current rules
    /// without a SQLite round-trip.
    pub fn get_project(&self, id: ProjectId) -> crate::Result<Project> {
        self.projects
            .get_settings(id)
            .ok_or_else(|| crate::EngineError::ProjectNotOpen(id.to_string()))
    }

    /// Update the cached `Project` (info + settings) for an open
    /// project. Used by the Phase 6 scope-rule and match & replace
    /// Tauri commands to write new rules.
    ///
    /// **Phase 6 Part C (C-A.1):** the in-memory update is now
    /// paired with a SQLite write via `Engine::save_settings`.
    /// `update_project` itself stays in-memory only (a "dry
    /// update" path for tests / previews); the CRUD Tauri
    /// commands call `save_settings` after `update_project` to
    /// persist.
    pub fn update_project(&self, project: Project) -> crate::Result<()> {
        self.projects.update_settings(project)
    }

    /// Persist the current `ProjectSettings` for an open project
    /// to the project's SQLite `projects.settings_json` column
    /// (Phase 6 Part C, §C-A.1). The CRUD Tauri commands call
    /// this after the in-memory `update_project` to make the
    /// mutation durable across engine restarts.
    ///
    /// **In-memory + on-disk both updated:** this method writes
    /// to SQLite AND updates the in-memory `ProjectSettings`
    /// cache (the `ProjectSettings` field on the cached
    /// `Project`). The in-memory update is the "current session
    /// sees the change" guarantee; the on-disk write is the
    /// "next session sees the change" guarantee.
    ///
    /// **Defensive semantics:** if the SQLite write fails, the
    /// in-memory update is still rolled back (the in-memory
    /// update happens AFTER the SQLite write). The error is
    /// surfaced to the caller; the UI shows a "save failed"
    /// toast. v0.5+ followup: transactional update with
    /// automatic rollback.
    pub fn save_settings(&self, id: ProjectId, settings: &ProjectSettings) -> crate::Result<()> {
        let pool = self
            .projects
            .get(id)
            .ok_or_else(|| crate::EngineError::ProjectNotOpen(id.to_string()))?;
        bk_store::projects::update_settings(&pool, id, settings)?;
        // Update the in-memory cache too (the cached `Project`'s
        // `settings` field). Without this, a subsequent
        // `get_project(id)` returns the pre-save value.
        if let Some(mut cached) = self.projects.get_settings(id) {
            cached.settings = settings.clone();
            self.projects.update_settings(cached)?;
        }
        Ok(())
    }

    /// List the `replay_history` entries for a given tab
    /// (Phase 6 Part C, §C-A.4). Returns entries ordered by
    /// `sequence_within_tab` ASC. Used by the UI's
    /// `ReplayStore.openTab` action to rehydrate the tab's
    /// in-memory history.
    pub fn list_replay_history(
        &self,
        project_id: ProjectId,
        tab_id: &str,
    ) -> crate::Result<Vec<bk_store::replay_history::ReplayHistoryEntry>> {
        let pool = self
            .projects
            .get(project_id)
            .ok_or_else(|| crate::EngineError::ProjectNotOpen(project_id.to_string()))?;
        bk_store::replay_history::list_by_tab(&pool, tab_id).map_err(Into::into)
    }

    /// Append a `replay_history` entry (Phase 6 Part C,
    /// §C-A.4). Used by the UI's `ReplayStore.appendSend`
    /// action. The caller (the Tauri command) mints the `id`
    /// and `sequence_within_tab` (the tab's current sequence
    /// count).
    pub fn append_replay_history(
        &self,
        project_id: ProjectId,
        entry: &bk_store::replay_history::ReplayHistoryEntry,
    ) -> crate::Result<()> {
        let pool = self
            .projects
            .get(project_id)
            .ok_or_else(|| crate::EngineError::ProjectNotOpen(project_id.to_string()))?;
        bk_store::replay_history::insert(&pool, entry).map_err(Into::into)
    }
}
