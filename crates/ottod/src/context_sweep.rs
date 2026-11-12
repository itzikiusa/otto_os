//! Orphaned `workflow-context/<run_id>/` folders: the engine's per-run scratch
//! dirs whose run row no longer exists.
//!
//! Run retention (otto-server `boot::tasks`) removes the folder of every run
//! IT prunes, but a run deleted any other way (its workflow deleted, a manual
//! cleanup, a folder delete that failed) left the folder behind forever — 343
//! of them on one install. Nothing can reach such a folder: every route that
//! reads one starts from the run row.
//!
//! Safety rules:
//! - only plain run-id names directly under `<data>/workflow-context/`, real
//!   directories (never a symlink), untouched (mtime) for the grace window;
//! - removed only when the DB positively says no such run exists — a query
//!   error ends the pass with nothing removed;
//! - at most [`MAX_REMOVALS_PER_PASS`] per daily pass, every pass logged with
//!   its counts;
//! - governed by the `workflow_context_orphan_days` setting (the grace in
//!   days, default [`DEFAULT_DAYS`], floored at [`MIN_DAYS`], `0` = off) and
//!   by the `data_retention` master switch (`enabled: false` = off).
//!
//! The same daily pass also sweeps orphaned per-folder skill bundles under
//! `~/.otto/context` ([`bundles`]).

mod bundles;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use otto_state::{DbPool, SettingsRepo};

/// Settings key: grace in days (`0` turns the sweep off).
pub const SETTING_KEY: &str = "workflow_context_orphan_days";
/// Default grace: a folder untouched for a month.
pub const DEFAULT_DAYS: u64 = 30;
/// A set grace never goes below this.
pub const MIN_DAYS: u64 = 7;
/// Folders removed per pass, at most.
pub const MAX_REMOVALS_PER_PASS: usize = 100;
/// Candidates checked against the DB per pass, at most.
const MAX_CHECKS_PER_PASS: usize = 1_000;
/// Pass cadence; the first pass waits [`FIRST_PASS_AFTER`] past boot.
const EVERY: Duration = Duration::from_secs(24 * 3600);
const FIRST_PASS_AFTER: Duration = Duration::from_secs(10 * 60);

/// The grace from the setting value: absent / malformed → the default,
/// `0` → off (`None`), anything else floored at [`MIN_DAYS`].
pub fn grace_days(v: Option<&serde_json::Value>) -> Option<u64> {
    match v.map(serde_json::Value::as_u64) {
        None | Some(None) => Some(DEFAULT_DAYS),
        Some(Some(0)) => None,
        Some(Some(d)) => Some(d.max(MIN_DAYS)),
    }
}

/// A folder name that can only be a run id (ULID / UUID alphabet).
fn is_run_id(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Directories directly under `root` named like a run id and untouched for
/// `grace`, oldest first, at most `limit`. Symlinks and files are skipped.
// Sync by design: runs on the blocking pool.
#[allow(clippy::disallowed_methods)]
pub fn candidates(
    root: &Path,
    now: SystemTime,
    grace: Duration,
    limit: usize,
) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found: Vec<(SystemTime, String, PathBuf)> = rd
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            // `DirEntry::metadata` does not follow symlinks.
            let meta = entry.metadata().ok()?;
            if !is_run_id(&name) || !meta.is_dir() {
                return None;
            }
            let modified = meta.modified().ok()?;
            let old = now.duration_since(modified).is_ok_and(|age| age >= grace);
            old.then(|| (modified, name, entry.path()))
        })
        .collect();
    found.sort_by_key(|(modified, _, _)| *modified);
    found
        .into_iter()
        .take(limit)
        .map(|(_, name, path)| (name, path))
        .collect()
}

/// What one pass did (for the log line).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub checked: usize,
    pub removed: usize,
    pub failed: usize,
}

/// One pass over `candidates`: `exists` answers whether a run row exists
/// (`Err` ends the pass), `remove` deletes one folder. Stops after
/// [`MAX_REMOVALS_PER_PASS`] removals.
pub async fn sweep<E, EF, R, RF>(
    candidates: Vec<(String, PathBuf)>,
    exists: E,
    remove: R,
) -> Result<Report, String>
where
    E: Fn(String) -> EF,
    EF: std::future::Future<Output = Result<bool, String>>,
    R: Fn(PathBuf) -> RF,
    RF: std::future::Future<Output = bool>,
{
    let mut report = Report::default();
    for (id, path) in candidates {
        if report.removed >= MAX_REMOVALS_PER_PASS {
            break;
        }
        report.checked += 1;
        if exists(id).await? {
            continue;
        }
        if remove(path).await {
            report.removed += 1;
        } else {
            report.failed += 1;
        }
    }
    Ok(report)
}

/// The `data_retention` master switch (on unless set `enabled: false`).
async fn retention_enabled(settings: &SettingsRepo) -> bool {
    let retention = settings
        .get(otto_state::retention::SETTING_KEY)
        .await
        .ok()
        .flatten();
    otto_state::RetentionPolicy::from_setting(retention.as_ref()).enabled
}

/// The daily pass against the real DB and data dir.
async fn pass(pool: &DbPool, root: &Path) {
    let settings = SettingsRepo::new(pool.clone());
    if !retention_enabled(&settings).await {
        return;
    }
    let Some(days) = grace_days(settings.get(SETTING_KEY).await.ok().flatten().as_ref()) else {
        return;
    };
    let grace = Duration::from_secs(days * 24 * 3600);
    let dir = root.to_path_buf();
    let found = tokio::task::spawn_blocking(move || {
        candidates(&dir, SystemTime::now(), grace, MAX_CHECKS_PER_PASS)
    })
    .await
    .unwrap_or_default();
    if found.is_empty() {
        return;
    }
    let reader = pool.reader().clone();
    let exists = |id: String| {
        let reader = reader.clone();
        async move {
            sqlx::query("SELECT 1 FROM workflow_runs WHERE id = ?")
                .bind(id)
                .fetch_optional(&reader)
                .await
                .map(|row| row.is_some())
                .map_err(|e| e.to_string())
        }
    };
    // Runs on the blocking pool (std fs is fine there).
    #[allow(clippy::disallowed_methods)]
    let remove = |path: PathBuf| async move {
        tokio::task::spawn_blocking(move || std::fs::remove_dir_all(path).is_ok())
            .await
            .unwrap_or(false)
    };
    match sweep(found, exists, remove).await {
        Ok(r) if r.removed + r.failed > 0 => tracing::info!(
            "retention: removed {} orphaned workflow-context folder(s) \
             ({} checked, {} could not be removed; grace {days} d)",
            r.removed,
            r.checked,
            r.failed
        ),
        Ok(_) => {}
        Err(e) => tracing::warn!("workflow-context orphan sweep stopped: {e}"),
    }
}

/// The daily skill-bundle pass ([`bundles`]): `~/.otto/context` (the root
/// `otto_context::materialize::default_context_root` provisions into) against
/// this daemon's `workflow-runs`. Never in an isolated E2E daemon, and never
/// without a `$HOME` (no fallback root to guess at).
async fn bundle_pass(pool: &DbPool, data_dir: &Path) {
    let settings = SettingsRepo::new(pool.clone());
    if otto_core::e2e::isolated() || !retention_enabled(&settings).await {
        return;
    }
    let setting = settings.get(bundles::SETTING_KEY).await.ok().flatten();
    let Some(days) = bundles::grace_days(setting.as_ref()) else {
        return;
    };
    let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) else {
        return;
    };
    let ctx_root = PathBuf::from(home).join(".otto").join("context");
    bundles::pass(ctx_root, data_dir.join("workflow-runs"), days).await;
}

/// Run [`pass`] and [`bundle_pass`] daily (first pass shortly after boot),
/// supervised.
pub fn spawn(pool: DbPool, data_dir: &Path) -> tokio::task::JoinHandle<()> {
    let state = (pool, data_dir.to_path_buf());
    otto_core::supervise::spawn("context-sweep", state, |(pool, data)| async move {
        tokio::time::sleep(FIRST_PASS_AFTER).await;
        loop {
            pass(&pool, &data.join("workflow-context")).await;
            bundle_pass(&pool, &data).await;
            tokio::time::sleep(EVERY).await;
        }
    })
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // sync test scaffolding
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Mutex;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ottod-context-sweep-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_setting_defaults_floors_and_turns_off() {
        assert_eq!(grace_days(None), Some(DEFAULT_DAYS));
        assert_eq!(
            grace_days(Some(&serde_json::json!("x"))),
            Some(DEFAULT_DAYS)
        );
        assert_eq!(grace_days(Some(&serde_json::json!(0))), None, "0 = off");
        assert_eq!(grace_days(Some(&serde_json::json!(1))), Some(MIN_DAYS));
        assert_eq!(grace_days(Some(&serde_json::json!(90))), Some(90));
    }

    #[test]
    fn only_run_id_named_real_directories_past_the_grace_are_candidates() {
        let root = scratch("candidates");
        std::fs::create_dir(root.join("01JRUNA")).unwrap();
        std::fs::create_dir(root.join("not a run id")).unwrap();
        std::fs::write(root.join("01JFILE"), "a file, not a folder").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("01JRUNA"), root.join("01JLINK")).unwrap();
        // "now" a year ahead: everything on disk is past a 30-day grace.
        let later = SystemTime::now() + Duration::from_secs(365 * 24 * 3600);
        let grace = Duration::from_secs(30 * 24 * 3600);
        let names: Vec<String> = candidates(&root, later, grace, 10)
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(names, vec!["01JRUNA".to_string()]);
        // Within the grace nothing qualifies.
        assert!(candidates(&root, SystemTime::now(), grace, 10).is_empty());
        // A missing root is simply empty.
        assert!(candidates(&root.join("missing"), later, grace, 10).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    fn list(ids: &[&str]) -> Vec<(String, PathBuf)> {
        ids.iter()
            .map(|id| (id.to_string(), PathBuf::from(format!("/nowhere/{id}"))))
            .collect()
    }

    #[tokio::test]
    async fn folders_with_a_run_are_kept_and_orphans_removed() {
        let live: HashSet<String> = ["a".to_string()].into();
        let removed = Mutex::new(Vec::new());
        let report = sweep(
            list(&["a", "b", "c"]),
            |id| {
                let hit = live.contains(&id);
                async move { Ok(hit) }
            },
            |p| {
                removed.lock().unwrap().push(p);
                async { true }
            },
        )
        .await
        .unwrap();
        assert_eq!(
            report,
            Report {
                checked: 3,
                removed: 2,
                failed: 0
            }
        );
        assert_eq!(
            *removed.lock().unwrap(),
            vec![PathBuf::from("/nowhere/b"), PathBuf::from("/nowhere/c")]
        );
    }

    #[tokio::test]
    async fn a_db_error_removes_nothing_more() {
        let removed = Mutex::new(0usize);
        let out = sweep(
            list(&["a", "b"]),
            |_| async { Err::<bool, _>("db locked".to_string()) },
            |_| {
                *removed.lock().unwrap() += 1;
                async { true }
            },
        )
        .await;
        assert_eq!(out, Err("db locked".to_string()));
        assert_eq!(*removed.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn one_pass_removes_at_most_the_cap() {
        let ids: Vec<String> = (0..MAX_REMOVALS_PER_PASS + 25)
            .map(|i| format!("r{i}"))
            .collect();
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let report = sweep(list(&refs), |_| async { Ok(false) }, |_| async { true })
            .await
            .unwrap();
        assert_eq!(report.removed, MAX_REMOVALS_PER_PASS);
        assert_eq!(report.checked, MAX_REMOVALS_PER_PASS);
    }

    #[tokio::test]
    async fn a_failed_delete_is_counted_not_fatal() {
        let report = sweep(
            list(&["a", "b"]),
            |_| async { Ok(false) },
            |p| {
                let ok = p.ends_with("b");
                async move { ok }
            },
        )
        .await
        .unwrap();
        assert_eq!(
            report,
            Report {
                checked: 2,
                removed: 1,
                failed: 1
            }
        );
    }
}
