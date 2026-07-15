//! The `Engine` is the long-lived orchestrator. One per Tauri app /
//! per axum process. Holds the `Projects` map and the global config
//! dir. Everything the rest of Talon needs goes through here.

#![allow(missing_docs)]

use crate::projects::Projects;
use bk_core::{ExchangeId, HttpExchange, ProjectId, Tag, TagId};
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
}

impl Engine {
    /// Initialize the engine at the given config dir, creating it if
    /// it doesn't exist. The projects subdir is also created.
    pub fn new(config_dir: impl Into<PathBuf>) -> std::io::Result<Self> {
        let config_dir = config_dir.into();
        std::fs::create_dir_all(&config_dir)?;
        let projects_dir = config_dir.join("projects");
        std::fs::create_dir_all(&projects_dir)?;
        // `Projects::new` takes ownership of the PathBuf. Clone first
        // so we can keep a copy in `self.projects_dir` for the UI to
        // display in the about dialog.
        let projects_dir_for_self = projects_dir.clone();
        Ok(Self {
            config_dir,
            projects_dir: projects_dir_for_self,
            projects: Projects::new(projects_dir),
        })
    }

    /// Open a project. Creates the DB file on first open, and
    /// upserts the `projects` row so `exchanges` inserts (which have
    /// a FK to `projects(id)`) succeed. The upsert is idempotent —
    /// re-opening a project just refreshes its row.
    pub fn open_project(&self, project: &bk_core::Project) -> crate::Result<Arc<bk_store::DbPool>> {
        let pool = self.projects.open(project)?;
        bk_store::projects::upsert(&pool, &project.info)?;
        Ok(pool)
    }

    /// Close a project.
    pub fn close_project(&self, id: ProjectId) {
        self.projects.close(id)
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
    pub fn delete_exchange(&self, project_id: ProjectId, id: ExchangeId) -> crate::Result<()> {
        let pool = self
            .projects
            .get(project_id)
            .ok_or_else(|| crate::EngineError::ProjectNotOpen(project_id.to_string()))?;
        bk_store::exchanges::delete(&pool, id).map_err(Into::into)
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
        bk_store::exchanges::update_notes(&pool, id, notes).map_err(Into::into)
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
        bk_store::exchanges::set_starred(&pool, id, starred).map_err(Into::into)
    }

    /// Create a tag (or return the existing one if the name is taken
    /// within the project). Idempotent. Returns the tag's ID.
    pub fn tag_upsert(
        &self,
        project_id: ProjectId,
        new: bk_store::tags::NewTag,
    ) -> crate::Result<TagId> {
        let pool = self
            .projects
            .get(project_id)
            .ok_or_else(|| crate::EngineError::ProjectNotOpen(project_id.to_string()))?;
        bk_store::tags::upsert(&pool, project_id, &new).map_err(Into::into)
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

    /// Attach a tag to an exchange. Idempotent. The `bk_store::tags::attach`
    /// function also enforces the cross-project invariant (the tag
    /// must belong to the same project as the exchange) -- this
    /// happens transparently because `bk_store::tags::attach` uses
    /// an `INSERT ... SELECT WHERE EXISTS` with a project_id JOIN.
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
        bk_store::tags::attach(&pool, tag_id, exchange_id).map_err(Into::into)
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
        bk_store::tags::detach(&pool, tag_id, exchange_id).map_err(Into::into)
    }

    /// Number of currently open projects.
    pub fn open_count(&self) -> usize {
        self.projects.open_count()
    }

    /// List the IDs of all currently open projects.
    pub fn open_ids(&self) -> Vec<ProjectId> {
        self.projects.open_ids()
    }
}
