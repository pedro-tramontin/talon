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
    ///
    /// Concurrency: the check+open+insert is atomic under a single write
    /// lock. Without the write lock, two callers could each see "not
    /// present", each `bk_store::open` the same file (creating two
    /// distinct `DbPool`s), and each receive a different `Arc` — the
    /// caller and `self.pools` would hold handles to different pools
    /// pointing at the same DB file. `bk_store::open` is cheap (it just
    /// returns a pool handle to the cached connection), so holding the
    /// write lock across it is fine.
    pub fn open(&self, project: &Project) -> Result<Arc<bk_store::DbPool>> {
        let path = project_path(&self.dir, project.info.id, &project.info.db_filename)?;
        let mut guard = self.pools.write().unwrap();
        if let Some(existing) = guard.get(&project.info.id) {
            return Ok(existing.clone());
        }
        let pool = Arc::new(bk_store::open(&path)?);
        guard.insert(project.info.id, pool.clone());
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

#[cfg(test)]
mod tests {
    use super::*;
    use bk_core::Project;
    use std::sync::{Arc, Barrier};
    use std::thread;

    /// Regression: `open` was previously check-then-insert under a
    /// read lock, then a write lock — a TOCTOU race. Two threads
    /// could each see "not present", each call `bk_store::open` (two
    /// distinct `DbPool`s to the same file), and each receive a
    /// different `Arc`. After the fix, all concurrent opens for the
    /// same project return the *same* `Arc` (pointer equality).
    ///
    /// The threads share the `Projects` instance via `Arc<Projects>`.
    /// `Projects` itself isn't `Sync` (it holds an `RwLock`, which
    /// is, but the wrapper is `!Sync` by default), so we wrap it in
    /// `Arc` and clone the handle — the spawned threads then take
    /// `&Projects` through the `Arc`. With the old check-then-insert
    /// pattern, this test would have failed with a non-matching
    /// `Arc::ptr_eq`; with the write-lock-around-whole-sequence
    /// pattern, all 8 threads converge on the same `DbPool`.
    #[test]
    fn concurrent_open_returns_same_pool_handle() {
        let tmp = tempfile::TempDir::new().unwrap();
        let projects = Arc::new(Projects::new(tmp.path()));
        let project = Arc::new(Project::new("acme.bb", "acme.bb", "0.1.0"));

        let n = 8usize;
        let barrier = Arc::new(Barrier::new(n));
        let mut handles = Vec::new();
        for _ in 0..n {
            let projects = Arc::clone(&projects);
            let project = Arc::clone(&project);
            let b = Arc::clone(&barrier);
            let handle = thread::spawn(move || {
                b.wait();
                projects.open(&project)
            });
            handles.push(handle);
        }

        let mut iter = handles.into_iter();
        let first: Arc<bk_store::DbPool> =
            iter.next().unwrap().join().expect("join").expect("open");
        for h in iter {
            let pool = h.join().expect("join").expect("open");
            assert!(
                Arc::ptr_eq(&first, &pool),
                "concurrent opens for the same project must return the same DbPool handle"
            );
        }
        assert_eq!(projects.open_count(), 1);
    }
}
