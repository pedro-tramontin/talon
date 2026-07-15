//! High-level project operations. The engine holds a pool per open
//! project; this module is the public API for opening, closing, and
//! listing them.

#![allow(missing_docs)]

use crate::error::{EngineError, Result};
use bk_core::{Project, ProjectId};
use bk_store::project_path;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// In-memory state of the engine: the projects dir, and the open pools.
/// `RwLock` so reads (listing projects) don't block reads; only opens
/// and closes take a write lock.
pub struct Projects {
    pub dir: PathBuf,
    pools: RwLock<HashMap<ProjectId, Arc<bk_store::DbPool>>>,
}

impl Projects {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            pools: RwLock::new(HashMap::new()),
        }
    }

    /// Open (or re-open) a project's database. Returns the shared pool handle.
    pub fn open(&self, project: &Project) -> Result<Arc<bk_store::DbPool>> {
        let path = project_path(&self.dir, project.info.id, &project.info.db_filename)?;
        if let Some(existing) = self.pools.read().unwrap().get(&project.info.id) {
            return Ok(existing.clone());
        }
        let pool = Arc::new(bk_store::open(&path)?);
        self.pools
            .write()
            .unwrap()
            .insert(project.info.id, pool.clone());
        Ok(pool)
    }

    /// Close a project's pool. Idempotent (closing a non-open project is OK).
    pub fn close(&self, id: ProjectId) {
        self.pools.write().unwrap().remove(&id);
    }

    /// Get an already-open pool, or None if it's not open. Used by
    /// the rest of the engine to look up the pool for a given project.
    pub fn get(&self, id: ProjectId) -> Option<Arc<bk_store::DbPool>> {
        self.pools.read().unwrap().get(&id).cloned()
    }

    /// Number of currently open projects. Used by the UI's
    /// "open projects" count badge.
    pub fn open_count(&self) -> usize {
        self.pools.read().unwrap().len()
    }

    /// List the IDs of all currently open projects. The UI uses this
    /// to populate the project switcher dropdown.
    pub fn open_ids(&self) -> Vec<ProjectId> {
        self.pools.read().unwrap().keys().copied().collect()
    }

    /// Re-export the dir so the engine can find it without going
    /// through `Projects`.
    pub fn dir(&self) -> &PathBuf {
        &self.dir
    }
}

impl std::fmt::Display for Projects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Projects(dir={}, open={})",
            self.dir.display(),
            self.open_count()
        )
    }
}

// Quiet "unused" warning for EngineError when only the variants are
// referenced via `?`/`From` (the engine.rs file uses both).
#[allow(dead_code)]
fn _ensure_error_used(_: EngineError) {}
