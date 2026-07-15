#![allow(missing_docs)]
//! Project metadata. A project is one SQLite file, one target, one
//! "scope" set. Stored on disk as `~/.local/share/talon/projects/<uuid>.db`
//! on Linux, `~/Library/Application Support/talon/projects/<uuid>.db` on
//! macOS, and `%APPDATA%\talon\projects\<uuid>.db` on Windows. The
//! project file itself is portable across OSes — it's just SQLite.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub id: crate::ProjectId,
    pub name: String,        // e.g. "acme.bb"
    pub target_host: String, // e.g. "acme.bb" — convenience for UI
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Path to the SQLite file, relative to the projects dir. We store
    /// the relative path so projects can move between machines.
    pub db_filename: String, // e.g. "acme-bb-2026-07-01.db"
    /// Talon version that created this project. Used for forward-compat
    /// checks in `bk-store`'s migrations.
    pub talon_version: String,
    /// Optional: the fingerprint of the CA that was used to MITM this
    /// project's traffic. Recorded so we can flag a project as
    /// "stale" if the global CA was rotated out from under it.
    pub ca_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSettings {
    /// Default scope rules applied to all exchanges in this project.
    /// (Phase 6 lets the user edit these interactively.)
    pub scope_rules: Vec<crate::scope::ScopeRule>,
    /// The default theme for the UI when this project is open.
    pub theme: Theme,
    /// Persisted match & replace rules.
    pub match_replace_rules: Vec<crate::scope::MatchReplaceRule>,
    /// Are we proxying live traffic? When false, only replay/fuzz can add
    /// exchanges to this project. Lets the user pause a project to
    /// review without new traffic being captured.
    pub proxy_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    System,
    Dark,
    Light,
}

#[allow(clippy::derivable_impls)]
impl Default for Theme {
    fn default() -> Self {
        Theme::System
    }
}

/// The full Project record. `info` is what gets shown in the projects
/// dropdown; `settings` is what gets persisted alongside it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub info: ProjectInfo,
    pub settings: ProjectSettings,
}

impl Project {
    /// Build a fresh Project from a name + target host. The DB filename
    /// is derived from the name + creation date so it's human-readable
    /// on disk.
    pub fn new(
        name: impl Into<String>,
        target_host: impl Into<String>,
        talon_version: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        let name = name.into();
        let target_host = target_host.into();
        let slug: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let db_filename = format!("{}-{}.db", slug.trim_matches('-'), now.format("%Y-%m-%d"));
        Self {
            info: ProjectInfo {
                id: crate::ProjectId::new(),
                name,
                target_host,
                created_at: now,
                updated_at: now,
                db_filename,
                talon_version: talon_version.into(),
                ca_fingerprint: None,
            },
            settings: ProjectSettings {
                scope_rules: vec![],
                theme: Theme::default(),
                match_replace_rules: vec![],
                proxy_enabled: true,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_new_sets_sane_defaults() {
        let p = Project::new("acme.bb", "acme.bb", "0.1.0");
        assert_eq!(p.info.name, "acme.bb");
        assert_eq!(p.info.target_host, "acme.bb");
        assert_eq!(p.settings.theme, Theme::System);
        assert!(p.settings.proxy_enabled);
        assert!(p.info.db_filename.starts_with("acme-bb-"));
        assert!(p.info.db_filename.ends_with(".db"));
    }

    #[test]
    fn project_db_filename_strips_non_alphanumerics() {
        let p = Project::new("Acme BB / Stage 1!", "acme.bb", "0.1.0");
        // Only lowercase alphanumerics and hyphens survive.
        assert!(p
            .info
            .db_filename
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.'));
    }

    #[test]
    fn project_serialization_roundtrips() {
        let p = Project::new("acme.bb", "acme.bb", "0.1.0");
        let s = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&s).unwrap();
        assert_eq!(back.info.id, p.info.id);
        assert_eq!(back.info.name, "acme.bb");
    }
}
