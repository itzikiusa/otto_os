//! Named Multi-Repo IDE Projects store (`<data_dir>/ide-projects.json`) and
//! `<10ms` bulk child-repository scanner for 30+ repo GoLand/IntelliJ workspaces.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use chrono::Utc;
use otto_core::{new_id, Error, Result};

use crate::target::canonicalize_local_root_dir;
use crate::types::{IdeNamedProject, IdeScanReposResp, IdeScannedRepo, IdeUpsertProjectReq};

#[derive(Clone)]
pub struct ProjectStore {
    file_path: PathBuf,
    cache: Arc<RwLock<Vec<IdeNamedProject>>>,
}

impl ProjectStore {
    pub fn new(data_dir: PathBuf) -> Self {
        let file_path = data_dir.join("ide-projects.json");
        let initial = fs::read_to_string(&file_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Vec<IdeNamedProject>>(&raw).ok())
            .unwrap_or_default();
        Self {
            file_path,
            cache: Arc::new(RwLock::new(initial)),
        }
    }

    pub fn list(&self) -> Vec<IdeNamedProject> {
        let mut items = self.cache.read().unwrap_or_else(|e| e.into_inner()).clone();
        items.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then_with(|| b.updated_at.cmp(&a.updated_at))
                .then_with(|| a.name.cmp(&b.name))
        });
        items
    }

    /// Return attached sibling folders configured for any Named Project whose
    /// `primary_root` (or member `folders`) matches `root`.
    pub fn folders_for_root(&self, root: &str) -> Vec<String> {
        let clean = root.trim().trim_end_matches('/');
        if clean.is_empty() {
            return Vec::new();
        }
        let guard = self.cache.read().unwrap_or_else(|e| e.into_inner());
        for proj in guard.iter() {
            if proj.primary_root.trim().trim_end_matches('/') == clean {
                return proj.folders.clone();
            }
        }
        Vec::new()
    }

    pub fn upsert(&self, req: IdeUpsertProjectReq, is_root_user: bool) -> Result<IdeNamedProject> {
        let name = req.name.trim().to_string();
        if name.is_empty() {
            return Err(Error::Invalid("project name must not be empty".into()));
        }
        let raw_primary = if !req.primary_root.trim().is_empty() {
            req.primary_root.trim()
        } else {
            req.folders
                .first()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| Error::Invalid("primary_root or folders[0] is required".into()))?
        };
        let primary_canon = canonicalize_local_root_dir(raw_primary, is_root_user)?;
        let primary_str = primary_canon.to_string_lossy().into_owned();

        let mut folders = Vec::new();
        folders.push(primary_str.clone());
        for raw in &req.folders {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(canon) = canonicalize_local_root_dir(trimmed, is_root_user) {
                let s = canon.to_string_lossy().into_owned();
                if !folders.contains(&s) {
                    folders.push(s);
                }
            }
        }

        let now = Utc::now().to_rfc3339();
        let mut guard = self.cache.write().unwrap_or_else(|e| e.into_inner());
        let id = req
            .id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                guard
                    .iter()
                    .find(|p| p.primary_root == primary_str && p.name == name)
                    .map(|p| p.id.clone())
            })
            .unwrap_or_else(|| format!("ideproj_{}", new_id()));

        let project = IdeNamedProject {
            id: id.clone(),
            name,
            primary_root: primary_str,
            folders,
            pinned: req.pinned,
            updated_at: now,
        };

        if let Some(existing) = guard.iter_mut().find(|p| p.id == id) {
            *existing = project.clone();
        } else {
            guard.push(project.clone());
        }
        self.persist_locked(&guard)?;
        Ok(project)
    }

    pub fn delete(&self, id: &str) -> Result<bool> {
        let target_id = id.trim();
        let mut guard = self.cache.write().unwrap_or_else(|e| e.into_inner());
        let before = guard.len();
        guard.retain(|p| p.id != target_id);
        let removed = guard.len() != before;
        if removed {
            self.persist_locked(&guard)?;
        }
        Ok(removed)
    }

    fn persist_locked(&self, items: &[IdeNamedProject]) -> Result<()> {
        if let Some(parent) = self.file_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| Error::Internal(format!("create ide-projects dir: {e}")))?;
        }
        let tmp = self.file_path.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(items)
            .map_err(|e| Error::Internal(format!("serialize ide-projects: {e}")))?;
        fs::write(&tmp, json)
            .map_err(|e| Error::Internal(format!("write ide-projects.tmp: {e}")))?;
        fs::rename(&tmp, &self.file_path)
            .map_err(|e| Error::Internal(format!("commit ide-projects.json: {e}")))?;
        Ok(())
    }
}

/// Fast `<10ms` 1-level scan of `parent_path` returning child repositories/folders
/// so users can bulk-select 30+ Go or Java repos into one Named IDE Project.
#[allow(clippy::disallowed_methods)] // runs inside spawn_blocking
pub fn scan_parent_repos(parent_path: &str, is_root_user: bool) -> Result<IdeScanReposResp> {
    let parent = canonicalize_local_root_dir(parent_path, is_root_user)?;
    let rd = fs::read_dir(&parent).map_err(|e| {
        Error::NotFound(format!("cannot read directory '{}': {e}", parent.display()))
    })?;

    let mut repos = Vec::new();
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || matches!(name.as_str(), "node_modules" | "target" | "Library") {
            continue;
        }
        let path = entry.path();
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        let is_dir = ft.is_dir() || (ft.is_symlink() && path.is_dir());
        if !is_dir {
            continue;
        }
        let canon = fs::canonicalize(&path).unwrap_or(path);
        let has_go_mod = canon.join("go.mod").is_file() || canon.join("go.work").is_file();
        let has_pom_or_gradle = canon.join("pom.xml").is_file()
            || canon.join("build.gradle").is_file()
            || canon.join("build.gradle.kts").is_file();
        let has_cargo = canon.join("Cargo.toml").is_file();
        let has_node = canon.join("package.json").is_file();
        let has_py =
            canon.join("pyproject.toml").is_file() || canon.join("requirements.txt").is_file();
        let has_git = canon.join(".git").exists();

        let kind = if has_go_mod {
            "go"
        } else if has_pom_or_gradle {
            "java"
        } else if has_cargo {
            "rust"
        } else if has_node {
            "node"
        } else if has_py {
            "python"
        } else if has_git {
            "git"
        } else {
            "folder"
        };

        repos.push(IdeScannedRepo {
            name,
            path: canon.to_string_lossy().into_owned(),
            kind: kind.to_string(),
            has_go_mod,
            has_pom_or_gradle,
            has_git,
        });
    }
    repos.sort_by(|a, b| {
        let a_is_repo = a.kind != "folder";
        let b_is_repo = b.kind != "folder";
        b_is_repo
            .cmp(&a_is_repo)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(IdeScanReposResp {
        parent_path: parent.to_string_lossy().into_owned(),
        repos,
    })
}
