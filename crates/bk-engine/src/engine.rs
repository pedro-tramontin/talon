//! The `Engine` is the long-lived orchestrator. One per Tauri app /
//! per axum process. Holds the `Projects` map and the global config
//! dir. Everything the rest of Talon needs goes through here.

#![allow(missing_docs)]

use crate::projects::Projects;
use bk_core::{HttpExchange, ProjectId};
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
        id: bk_core::ExchangeId,
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
    ) -> crate::Result<Vec<bk_core::ExchangeId>> {
        let pool = self
            .projects
            .get(project_id)
            .ok_or_else(|| crate::EngineError::ProjectNotOpen(project_id.to_string()))?;
        bk_store::fts::search(&pool, project_id, query, limit).map_err(Into::into)
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
