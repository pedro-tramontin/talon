//! High-level project operations. The engine holds a pool per open
//! project; this module is the public API for opening, closing, and
//! listing them.
//!
//! ## In-memory settings (Phase 6, §6.2)
//!
//! As of Phase 6, the engine also keeps the full `Project` (info +
//! settings) in an `RwLock<HashMap<ProjectId, Project>>` so the
//! scope-rule and match & replace Tauri commands can mutate
//! `ProjectSettings` without a SQLite round-trip. **The settings
//! are in memory only** — there is no persistence on disk; the
//! `bk_store::projects::upsert` only persists `ProjectInfo`. When
//! the engine is restarted, the next `open_project` call rebuilds
//! the in-memory entry with `ProjectSettings::default()`. This is
//! the same deferral pattern Phase 5 used for `ReplayStore.history`
//! (in-memory only, persistence is a v0.5+ follow-up).

#![allow(missing_docs)]

use crate::error::{EngineError, Result};
use bk_core::{Project, ProjectId, ProjectInfo};
use bk_store::project_path;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// In-memory state of the engine: the projects dir, the open pools,
/// and (Phase 6) the cached `Project` (with settings) for each
/// open project. `RwLock` so reads (listing projects) don't block
/// reads; only opens and closes take a write lock.
pub struct Projects {
    pub dir: PathBuf,
    pools: RwLock<HashMap<ProjectId, Arc<bk_store::DbPool>>>,
    /// Cached `Project` (info + settings) per open project.
    /// Populated by `open_with_info`; read by `get_settings` /
    /// `update_settings`; cleared by `close`.
    cache: RwLock<HashMap<ProjectId, Project>>,
}

impl Projects {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            pools: RwLock::new(HashMap::new()),
            cache: RwLock::new(HashMap::new()),
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
    ///
    /// **Phase 6 Part C (§C-A.1):** the in-memory `ProjectSettings`
    /// cache is rehydrated from SQLite on every `open` call. The
    /// `ProjectSettings` are persisted to `projects.settings_json` by
    /// `bk_store::projects::update_settings`; the on-disk value is the
    /// source of truth. If the SQLite column is the schema's `'{}'`
    /// default (a pre-migration-001 DB), the cache is populated with
    /// `ProjectSettings::default()`. The fresh `Project` the caller
    /// passes in still owns `info` (name, target_host, etc.); the cache
    /// takes its `settings` from SQLite.
    pub fn open(&self, project: &Project) -> Result<Arc<bk_store::DbPool>> {
        let path = project_path(&self.dir, project.info.id, &project.info.db_filename)?;
        let mut pools_guard = self.pools.write().unwrap();
        if let Some(existing) = pools_guard.get(&project.info.id) {
            // Already open: rehydrate settings from SQLite (in case
            // another writer mutated them since the last open), then
            // merge the fresh `info` from the caller's `Project`.
            let pool = existing.clone();
            drop(pools_guard);
            let settings =
                bk_store::projects::read_settings(&pool, project.info.id).unwrap_or_default();
            let mut merged = project.clone();
            merged.settings = settings;
            self.cache.write().unwrap().insert(merged.info.id, merged);
            return Ok(pool);
        }
        let pool = Arc::new(bk_store::open(&path)?);
        let id = project.info.id;
        pools_guard.insert(id, pool.clone());
        drop(pools_guard);
        // Rehydrate settings from SQLite on the fresh open.
        let settings = bk_store::projects::read_settings(&pool, id).unwrap_or_default();
        let mut merged = project.clone();
        merged.settings = settings;
        self.cache.write().unwrap().insert(id, merged);
        Ok(pool)
    }

    /// Close a project's pool. Idempotent (closing a non-open project is OK).
    pub fn close(&self, id: ProjectId) {
        self.pools.write().unwrap().remove(&id);
        self.cache.write().unwrap().remove(&id);
    }

    /// Get an already-open pool, or None if it's not open. Used by
    /// the rest of the engine to look up the pool for a given project.
    pub fn get(&self, id: ProjectId) -> Option<Arc<bk_store::DbPool>> {
        self.pools.read().unwrap().get(&id).cloned()
    }

    /// Get the cached `Project` (info + settings) for an open project.
    /// Returns `None` if the project is not open. Used by the Phase 6
    /// scope-rule and match & replace Tauri commands to read the
    /// current rules without a SQLite round-trip.
    pub fn get_settings(&self, id: ProjectId) -> Option<Project> {
        self.cache.read().unwrap().get(&id).cloned()
    }

    /// Update the cached `Project` (info + settings) for an open
    /// project. Replaces the whole entry. The caller (the engine)
    /// is responsible for the `Result<()>` semantics: this
    /// function returns `ProjectNotOpen` if the project is not
    /// currently open, and `Ok(())` on success.
    ///
    /// **No SQLite write happens here** — the persistence is a
    /// v0.5+ follow-up. See the module-level docs.
    pub fn update_settings(&self, project: Project) -> Result<()> {
        let mut guard = self.cache.write().unwrap();
        if !self.pools.read().unwrap().contains_key(&project.info.id) {
            return Err(EngineError::ProjectNotOpen(project.info.id.to_string()));
        }
        guard.insert(project.info.id, project);
        Ok(())
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

    /// List the `ProjectInfo` for all currently open projects,
    /// ordered newest-first by `created_at`. The v0.5+ post-batch
    /// P3 #9 gap-fix wires the UI's `setProjects` action to a
    /// `list_projects` Tauri command that calls this method.
    ///
    /// **Scope:** only currently-open projects are listed. Each
    /// project has its own DB file (`<config_dir>/projects/<id>.db`)
    /// with its own `projects` table; a global registry of
    /// "every project ever opened" would require either a shared
    /// global DB or a directory scan of the projects dir — both
    /// are out of scope for v0.5+ post-batch. The UI's project
    /// dropdown already shows the open-projects list (via the
    /// `Projects` event bus), so the new `setProjects` action
    /// just rehydrates that list on app startup.
    pub fn list_open_projects(&self) -> Vec<ProjectInfo> {
        let mut out: Vec<ProjectInfo> = self
            .cache
            .read()
            .unwrap()
            .values()
            .map(|p| p.info.clone())
            .collect();
        // Newest first (the spec's `ORDER BY created_at DESC`).
        out.sort_by_key(|p| std::cmp::Reverse(p.created_at));
        out
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

    /// `list_open_projects` returns the `ProjectInfo` for every
    /// currently-open project, ordered newest-first by `created_at`.
    /// The v0.5+ post-batch P3 #9 gap-fix wires the UI's
    /// `setProjects` action to a `list_projects` Tauri command
    /// that calls this method; this test pins the
    /// (a) only-open-projects filter, (b) newest-first ordering,
    /// (c) clone-isolation (the cache is unchanged).
    #[test]
    fn list_open_projects_returns_open_only_newest_first() {
        let tmp = tempfile::TempDir::new().unwrap();
        let projects = Projects::new(tmp.path());

        // Open 2 projects. The first-opened has the older
        // `created_at`; we want the list to surface the
        // second-opened project first.
        let p1 = Project::new("acme.bb", "acme.bb", "0.1.0");
        let p1_id = p1.info.id;
        let p2 = Project::new("globex.bb", "globex.bb", "0.1.0");
        let p2_id = p2.info.id;
        projects.open(&p1).expect("open p1");
        // Force a measurable time gap so `created_at` strictly
        // orders (chrono has sub-second resolution but a busy
        // test machine can collapse two `Utc::now()` calls into
        // the same tick).
        std::thread::sleep(std::time::Duration::from_millis(10));
        projects.open(&p2).expect("open p2");

        let list = projects.list_open_projects();
        assert_eq!(list.len(), 2, "both open projects must appear");
        // Newest first: p2 before p1.
        assert_eq!(list[0].id, p2_id, "newest project must be first");
        assert_eq!(list[1].id, p1_id, "oldest project must be last");
        assert!(
            list[0].created_at >= list[1].created_at,
            "list must be ordered by created_at DESC"
        );

        // Close p1: the next `list_open_projects` call returns
        // only p2.
        projects.close(p1_id);
        let list2 = projects.list_open_projects();
        assert_eq!(list2.len(), 1, "only the still-open project remains");
        assert_eq!(list2[0].id, p2_id);

        // Clone-isolation: the returned `Vec<ProjectInfo>` does
        // not pin the cache. Mutating the returned vec must
        // not affect a subsequent call.
        let list3 = projects.list_open_projects();
        assert_eq!(list3.len(), 1);
    }
}
