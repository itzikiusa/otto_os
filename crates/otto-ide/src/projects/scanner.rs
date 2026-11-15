//! Path normalization, legacy `ide-projects.json` atomic migration, and fast
//! 1-level child-repository scanner (`POST /ide/projects/scan`).

use std::fs;
use std::path::{Path, PathBuf};

use otto_core::domain::User;
use otto_core::{Error, Result};
use otto_state::DbPool;
use sqlx::Row;

use crate::target::{canonicalize_local_root_dir, enforce_non_root_path};
use crate::types::{
    IdeNamedProject, IdeScanReposResp, IdeScannedRepo, IdeTargetSpec, IdeUpsertProjectReq,
};
use crate::IdeCtx;

#[derive(Clone, Debug)]
pub(super) struct StoredProject {
    pub(super) owner_user_id: String,
    pub(super) raw_primary_root: String,
    pub(super) raw_folders: Vec<String>,
    pub(super) project: IdeNamedProject,
    pub(super) non_root_project: Option<IdeNamedProject>,
}

pub(super) struct NormalizedUpsertProject {
    pub(super) name: String,
    pub(super) primary_root: String,
    pub(super) folders: Vec<String>,
    pub(super) pinned: bool,
    pub(super) explicit_id: Option<String>,
}

pub(super) async fn legacy_project_authorized<S: IdeCtx>(
    ctx: &S,
    user: &User,
    proj: &IdeNamedProject,
    write: bool,
) -> bool {
    if proj.primary_root.trim().is_empty() {
        return false;
    }
    let mut spec = IdeTargetSpec::local(proj.primary_root.trim());
    spec.additional_roots = proj
        .folders
        .iter()
        .filter(|f| !f.trim().is_empty() && f.trim() != proj.primary_root.trim())
        .cloned()
        .collect();
    ctx.authorize_local_target(user, &spec, write).await.is_ok()
}

pub(super) async fn migrate_legacy_projects_atomic(
    pool: &DbPool,
    legacy_file_path: &Path,
) -> Result<()> {
    let legacy_path = legacy_file_path.to_path_buf();
    let legacy_items: Option<Vec<IdeNamedProject>> = tokio::task::spawn_blocking(move || {
        let raw = match fs::read_to_string(&legacy_path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(Error::Internal(format!(
                    "read legacy '{}': {e}",
                    legacy_path.display()
                )));
            }
        };
        let items: Vec<IdeNamedProject> = serde_json::from_str(&raw).map_err(|e| {
            tracing::warn!(
                path = %legacy_path.display(),
                error = %e,
                "legacy ide-projects.json is corrupt; preserving file without deleting"
            );
            Error::Internal(format!("parse legacy '{}': {e}", legacy_path.display()))
        })?;
        Ok(Some(items))
    })
    .await
    .map_err(|e| Error::Internal(format!("legacy migration worker failed: {e}")))??;

    let Some(items) = legacy_items else {
        return Ok(());
    };

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| Error::Internal(format!("begin legacy ide_named_projects tx: {e}")))?;

    for item in items {
        let folders_json = serde_json::to_string(&item.folders)
            .map_err(|e| Error::Internal(format!("serialize legacy project folders: {e}")))?;
        sqlx::query(
            "INSERT OR IGNORE INTO ide_named_projects
                (id, owner_user_id, name, primary_root, folders_json, pinned, updated_at)
             VALUES (?1, '', ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(&item.id)
        .bind(&item.name)
        .bind(&item.primary_root)
        .bind(&folders_json)
        .bind(if item.pinned { 1_i64 } else { 0_i64 })
        .bind(&item.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| Error::Internal(format!("insert legacy ide_named_projects row: {e}")))?;
    }

    tx.commit()
        .await
        .map_err(|e| Error::Internal(format!("commit legacy ide_named_projects tx: {e}")))?;

    let legacy_rm = legacy_file_path.to_path_buf();
    let _ = tokio::task::spawn_blocking(move || {
        if let Err(e) = fs::remove_file(&legacy_rm) {
            tracing::warn!(
                path = %legacy_rm.display(),
                error = %e,
                "failed to remove migrated ide-projects.json"
            );
        }
    })
    .await;
    Ok(())
}

pub(super) async fn load_projects_from_db(pool: &DbPool) -> Result<Vec<(String, IdeNamedProject)>> {
    let rows = sqlx::query(
        "SELECT id, owner_user_id, name, primary_root, folders_json, pinned, updated_at
         FROM ide_named_projects",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| Error::Internal(format!("load ide_named_projects: {e}")))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let folders_json: String = row.try_get("folders_json").unwrap_or_default();
        let folders: Vec<String> = serde_json::from_str(&folders_json).unwrap_or_default();
        let owner: String = row.try_get("owner_user_id").unwrap_or_default();
        out.push((
            owner,
            IdeNamedProject {
                id: row.try_get("id").unwrap_or_default(),
                name: row.try_get("name").unwrap_or_default(),
                primary_root: row.try_get("primary_root").unwrap_or_default(),
                folders,
                pinned: row.try_get::<i64, _>("pinned").unwrap_or(0) != 0,
                updated_at: row.try_get("updated_at").unwrap_or_default(),
            },
        ));
    }
    Ok(out)
}

#[allow(clippy::disallowed_methods)]
pub(super) fn build_stored_project(
    owner_user_id: String,
    raw_proj: IdeNamedProject,
) -> StoredProject {
    let raw_primary_root = trim_trailing_slash(&otto_core::paths::expand_tilde(
        raw_proj.primary_root.trim(),
    ));
    let raw_folders: Vec<String> = raw_proj
        .folders
        .iter()
        .map(|f| trim_trailing_slash(&otto_core::paths::expand_tilde(f.trim())))
        .filter(|s| !s.is_empty())
        .collect();
    let root_proj = revalidate_project_paths(raw_proj.clone(), true).unwrap_or(raw_proj.clone());
    let non_root_project = revalidate_project_paths(raw_proj, false);
    StoredProject {
        owner_user_id,
        raw_primary_root,
        raw_folders,
        project: root_proj,
        non_root_project,
    }
}

#[allow(clippy::disallowed_methods)]
pub(super) fn normalize_upsert_req(
    req: IdeUpsertProjectReq,
    is_root_user: bool,
) -> Result<NormalizedUpsertProject> {
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

    let mut folders = vec![primary_str.clone()];
    for raw in &req.folders {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let canon = canonicalize_local_root_dir(trimmed, is_root_user)?;
        let s = canon.to_string_lossy().into_owned();
        if !folders.contains(&s) {
            folders.push(s);
        }
    }

    let explicit_id = req
        .id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    Ok(NormalizedUpsertProject {
        name,
        primary_root: primary_str,
        folders,
        pinned: req.pinned,
        explicit_id,
    })
}

#[allow(clippy::disallowed_methods)]
fn revalidate_project_paths(mut proj: IdeNamedProject, is_root: bool) -> Option<IdeNamedProject> {
    let primary_expanded = PathBuf::from(otto_core::paths::expand_tilde(proj.primary_root.trim()));
    let primary_canon = fs::canonicalize(&primary_expanded).unwrap_or(primary_expanded);
    if !is_root && enforce_non_root_path(&primary_canon).is_err() {
        return None;
    }
    proj.primary_root = trim_trailing_slash(&primary_canon.to_string_lossy());
    proj.folders = filter_folders_for_caller(&proj.folders, is_root);
    if !proj.folders.contains(&proj.primary_root) {
        proj.folders.insert(0, proj.primary_root.clone());
    }
    Some(proj)
}

#[allow(clippy::disallowed_methods)]
fn filter_folders_for_caller(folders: &[String], is_root: bool) -> Vec<String> {
    let mut out = Vec::with_capacity(folders.len());
    for raw in folders {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let expanded = PathBuf::from(otto_core::paths::expand_tilde(trimmed));
        let canon = fs::canonicalize(&expanded).unwrap_or(expanded);
        if !is_root && enforce_non_root_path(&canon).is_err() {
            continue;
        }
        let s = trim_trailing_slash(&canon.to_string_lossy());
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

pub(super) fn trim_trailing_slash(s: &str) -> String {
    let stripped = s.trim().trim_end_matches('/');
    if stripped.is_empty() && s.trim().starts_with('/') {
        "/".to_string()
    } else {
        stripped.to_string()
    }
}

pub(super) fn normalize_lookup_fast(root: &str) -> Option<String> {
    let trimmed = root.trim();
    if trimmed.is_empty() {
        return None;
    }
    let norm = trim_trailing_slash(&otto_core::paths::expand_tilde(trimmed));
    (!norm.is_empty()).then_some(norm)
}

pub(super) fn stored_project_matches(entry: &StoredProject, lookup: &str) -> bool {
    let alt = lookup
        .strip_prefix("/var/")
        .map(|r| format!("/private/var/{r}"))
        .or_else(|| {
            lookup
                .strip_prefix("/tmp/")
                .map(|r| format!("/private/tmp/{r}"))
        });
    let eq_any = |cand: &str| cand == lookup || alt.as_deref().is_some_and(|a| cand == a);
    eq_any(&entry.project.primary_root)
        || eq_any(&entry.raw_primary_root)
        || entry.project.folders.iter().any(|f| eq_any(f))
        || entry.raw_folders.iter().any(|f| eq_any(f))
}

const MAX_SCAN_REPOS_ENTRIES: usize = 200;
const MAX_SCAN_RAW_DIR_ENTRIES: usize = 1_024;

#[allow(clippy::disallowed_methods)]
fn is_broad_system_scan_root(parent: &Path, is_root_user: bool) -> bool {
    let s = parent.to_string_lossy();
    let trimmed = s.trim_end_matches('/');
    if trimmed.is_empty()
        || matches!(
            trimmed,
            "/Users"
                | "/home"
                | "/tmp"
                | "/private/tmp"
                | "/var"
                | "/private/var"
                | "/etc"
                | "/private/etc"
                | "/System"
                | "/Volumes"
        )
    {
        return true;
    }
    if !is_root_user {
        if let Some(home) = dirs::home_dir() {
            let home_canon = std::fs::canonicalize(&home).unwrap_or(home);
            if parent == home_canon {
                return true;
            }
        }
    }
    false
}

/// Fast 1-level scan of `parent_path` returning child repositories and folders
/// so users can bulk-select multi-repo workspaces into one Named IDE Project.
#[allow(clippy::disallowed_methods)] // runs inside spawn_blocking
pub fn scan_parent_repos(parent_path: &str, is_root_user: bool) -> Result<IdeScanReposResp> {
    let parent = canonicalize_local_root_dir(parent_path, is_root_user)?;
    if is_broad_system_scan_root(&parent, is_root_user) {
        return Err(Error::Invalid(format!(
            "refusing to scan broad system directory '{}'; choose a workspace parent folder",
            parent.display()
        )));
    }
    let rd = fs::read_dir(&parent).map_err(|e| {
        Error::NotFound(format!("cannot read directory '{}': {e}", parent.display()))
    })?;

    let mut repos: Vec<IdeScannedRepo> = Vec::new();
    let mut seen_by_path: std::collections::HashMap<String, (usize, bool)> =
        std::collections::HashMap::new();
    for entry in rd.flatten().take(MAX_SCAN_RAW_DIR_ENTRIES) {
        if repos.len() >= MAX_SCAN_REPOS_ENTRIES {
            break;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || matches!(name.as_str(), "node_modules" | "target" | "Library") {
            continue;
        }
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        let is_symlink = ft.is_symlink();
        let is_dir = ft.is_dir() || (is_symlink && path.is_dir());
        if !is_dir {
            continue;
        }
        let canon = fs::canonicalize(&path).unwrap_or(path);
        if !is_root_user && enforce_non_root_path(&canon).is_err() {
            continue;
        }
        let canon_str = canon.to_string_lossy().into_owned();
        if let Some(&(existing_idx, prev_was_symlink)) = seen_by_path.get(&canon_str) {
            if prev_was_symlink && !is_symlink {
                repos[existing_idx].name = name;
                seen_by_path.insert(canon_str, (existing_idx, false));
            }
            continue;
        }

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

        let idx = repos.len();
        seen_by_path.insert(canon_str.clone(), (idx, is_symlink));
        repos.push(IdeScannedRepo {
            name,
            path: canon_str,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn scan_parent_repos_deduplicates_symlink_to_sibling_repo() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let real_repo = tmp.path().join("supprt_agent");
        fs::create_dir_all(&real_repo).expect("mkdir repo");
        fs::write(real_repo.join("go.mod"), "module example.com/supprt\n").expect("write go.mod");
        std::os::unix::fs::symlink(&real_repo, tmp.path().join("support_agent"))
            .expect("create sibling symlink");

        let other_repo = tmp.path().join("go_utilities");
        fs::create_dir_all(&other_repo).expect("mkdir go_utilities");
        fs::write(other_repo.join("go.mod"), "module example.com/utils\n").expect("write go.mod");

        let scanned = scan_parent_repos(&tmp.path().to_string_lossy(), true).expect("scan ok");
        assert_eq!(
            scanned.repos.len(),
            2,
            "sibling symlink to same canonical directory must not produce duplicate path keys"
        );
        let names: Vec<&str> = scanned.repos.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["go_utilities", "supprt_agent"]);
    }

    #[cfg(unix)]
    #[test]
    fn scan_parent_repos_real_dir_wins_over_earlier_and_later_symlinks() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let real_repo = tmp.path().join("m_real");
        fs::create_dir_all(real_repo.join(".git")).expect("mkdir m_real/.git");
        std::os::unix::fs::symlink(&real_repo, tmp.path().join("a_link"))
            .expect("create earlier symlink a_link -> m_real");
        std::os::unix::fs::symlink(&real_repo, tmp.path().join("z_link"))
            .expect("create later symlink z_link -> m_real");

        let scanned = scan_parent_repos(&tmp.path().to_string_lossy(), true).expect("scan ok");
        assert_eq!(
            scanned.repos.len(),
            1,
            "both symlinks pointing to m_real must collapse into a single canonical repo"
        );
        assert_eq!(scanned.repos[0].name, "m_real");
        assert_eq!(scanned.repos[0].kind, "git");
        assert!(scanned.repos[0].has_git);
    }

    #[test]
    fn scan_parent_repos_refuses_broad_system_roots_and_caps_entries_at_200() {
        for broad in ["/", "/tmp", "/var"] {
            let err =
                scan_parent_repos(broad, true).expect_err("broad system root must be refused");
            assert!(
                matches!(err, Error::Invalid(_)),
                "expected Error::Invalid for {broad}, got {err:?}"
            );
        }

        let tmp = tempfile::tempdir().expect("tempdir");
        for i in 0..235 {
            fs::create_dir_all(tmp.path().join(format!("repo_{i:03}"))).expect("mkdir child");
        }
        let scanned = scan_parent_repos(&tmp.path().to_string_lossy(), true).expect("scan capped");
        assert_eq!(scanned.repos.len(), MAX_SCAN_REPOS_ENTRIES);
    }
}
