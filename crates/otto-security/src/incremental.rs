use std::collections::{BTreeMap, HashSet};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::engine::{read_json, write_json, EngineKind, BASE_EXCLUDED_DIRS};
use crate::{Finding, ScanMode, ScanSettings};

type Snapshot = BTreeMap<String, String>;
const LINK_PREFIX: &str = "link:";

#[derive(Serialize, Deserialize)]
struct Cache {
    key: String,
    files: Snapshot,
    findings: Vec<Finding>,
}

pub(crate) enum Selection {
    Full,
    Files(Vec<String>),
}

pub(crate) struct Plan {
    pub(crate) selection: Selection,
    files: Option<Snapshot>,
    previous: Vec<Finding>,
    mode: ScanMode,
}

pub(crate) struct CacheContext<'a> {
    dir: PathBuf,
    key: &'a str,
    settings: &'a ScanSettings,
}

impl<'a> CacheContext<'a> {
    pub(crate) fn new(dir: &Path, key: &'a str, settings: &'a ScanSettings) -> Self {
        Self {
            dir: dir.join("result-cache"),
            key,
            settings,
        }
    }

    pub(crate) async fn plan(
        &self,
        root: &Path,
        kind: EngineKind,
        cancel: &CancellationToken,
    ) -> Result<Plan> {
        let current = snapshot(root, self.settings, cancel).await?;
        let cached: Option<Cache> = read_json(&self.path(root)).await;
        Ok(plan(cached, current, self.key, kind))
    }

    pub(crate) async fn finish(
        &self,
        root: &Path,
        plan: Plan,
        mode: &mut ScanMode,
        mut findings: Vec<Finding>,
        cancel: &CancellationToken,
    ) -> Result<Vec<Finding>> {
        *mode = (*mode).max(plan.mode);
        if plan.mode == ScanMode::Cached {
            return Ok(plan.previous);
        }
        findings.extend(plan.previous);
        if let Some(files) = plan.files {
            // A concurrent save must never associate new file bytes with old findings.
            if snapshot(root, self.settings, cancel).await?.as_ref() == Some(&files) {
                if let Err(error) = self.save(root, files, &findings).await {
                    tracing::warn!(%error, "security results were not cached");
                }
            }
        }
        Ok(findings)
    }

    fn path(&self, root: &Path) -> PathBuf {
        let hash = hex::encode(Sha256::digest(root.as_os_str().as_encoded_bytes()));
        self.dir.join(format!("{hash}.json"))
    }

    async fn save(&self, root: &Path, files: Snapshot, findings: &[Finding]) -> Result<()> {
        #[derive(Serialize)]
        struct Record<'a> {
            key: &'a str,
            files: Snapshot,
            findings: &'a [Finding],
        }
        tokio::fs::create_dir_all(&self.dir)
            .await
            .map_err(crate::download::io_error)?;
        write_json(
            &self.path(root),
            &Record {
                key: self.key,
                files,
                findings,
            },
        )
        .await
    }
}

fn plan(cached: Option<Cache>, files: Option<Snapshot>, key: &str, kind: EngineKind) -> Plan {
    if let (Some(cache), Some(current)) = (cached.filter(|cache| cache.key == key), &files) {
        if &cache.files == current {
            return Plan {
                selection: Selection::Files(Vec::new()),
                files,
                previous: cache.findings,
                mode: ScanMode::Cached,
            };
        }
        if kind == EngineKind::Opengrep {
            let changed: Vec<_> = current
                .iter()
                .filter(|(path, hash)| cache.files.get(*path) != Some(*hash))
                .map(|(path, _)| path.clone())
                .collect();
            // An ignore file can include an alias under an arbitrary name.
            let has_ignore_file = current.keys().any(|path| path.ends_with(".semgrepignore"));
            let scope_input = |path: &str, hash: &str| {
                path.ends_with(".semgrepignore")
                    || (has_ignore_file && hash.starts_with(LINK_PREFIX))
            };
            let removed_scope_input = cache
                .files
                .iter()
                .any(|(path, hash)| scope_input(path, hash) && !current.contains_key(path));
            if !removed_scope_input
                && changed.len() <= 64
                && changed.iter().all(|path| {
                    include_safe(path)
                        && !scope_input(path, &current[path])
                        && !cache
                            .files
                            .get(path)
                            .is_some_and(|hash| scope_input(path, hash))
                })
            {
                let changed_paths: HashSet<_> = changed.iter().map(String::as_str).collect();
                let previous = cache
                    .findings
                    .into_iter()
                    .filter(|finding| {
                        current.contains_key(&finding.path)
                            && !changed_paths.contains(finding.path.as_str())
                    })
                    .collect();
                return Plan {
                    selection: Selection::Files(changed),
                    files,
                    previous,
                    mode: ScanMode::Incremental,
                };
            }
        }
    }
    Plan {
        selection: Selection::Full,
        files,
        previous: Vec::new(),
        mode: ScanMode::Full,
    }
}

fn include_safe(path: &str) -> bool {
    path.len() < 512 && !path.contains(['*', '?', '[', ']', '{', '}', '\\', '\n', '\r'])
}

pub(crate) fn exclude_globs(settings: &ScanSettings) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in &settings.excludes {
        let glob = GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .map_err(|_| Error::Invalid(format!("Invalid exclude pattern: {pattern}")))?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|_| Error::Invalid("Invalid scan exclude patterns.".into()))
}

async fn snapshot(
    root: &Path,
    settings: &ScanSettings,
    cancel: &CancellationToken,
) -> Result<Option<Snapshot>> {
    let root = root.to_path_buf();
    let excludes = exclude_globs(settings)?;
    let cancel = cancel.clone();
    tokio::task::spawn_blocking(move || {
        let result = collect(&root, excludes, &cancel);
        if cancel.is_cancelled() {
            Err(crate::governor::cancelled())
        } else {
            // Unreadable or oversized inputs are scanned without result reuse.
            Ok(result)
        }
    })
    .await
    .map_err(|_| Error::Internal("security snapshot worker stopped".into()))?
}

fn collect(root: &Path, excludes: GlobSet, cancel: &CancellationToken) -> Option<Snapshot> {
    #[cfg(target_os = "macos")]
    let _background = crate::download::BackgroundThread::enter();

    let mut walk = ignore::WalkBuilder::new(root);
    walk.standard_filters(false).follow_links(false);
    walk.filter_entry({
        let root = root.to_path_buf();
        move |entry| {
            let Ok(path) = entry.path().strip_prefix(&root) else {
                return false;
            };
            path.as_os_str().is_empty()
                || (!BASE_EXCLUDED_DIRS
                    .iter()
                    .any(|name| entry.file_name() == *name)
                    && !excludes.is_match(path))
        }
    });
    let mut total = 0u64;
    let mut files = BTreeMap::new();
    let mut links = Vec::new();
    let mut buffer = [0; 64 * 1024];
    for entry in walk.build() {
        if cancel.is_cancelled() {
            return None;
        }
        let entry = entry.ok()?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            continue;
        }
        if files.len() + links.len() >= 100_000 {
            return None;
        }
        let relative = entry.path().strip_prefix(root).ok()?.to_str()?;
        if kind.is_symlink() {
            links.push((relative.to_owned(), entry.path().to_owned()));
            continue;
        }
        if !kind.is_file() {
            return None;
        }
        let mut file = open_file(entry.path()).ok()?;
        let mut hash = Sha256::new();
        loop {
            if cancel.is_cancelled() {
                return None;
            }
            let length = file.read(&mut buffer).ok()?;
            if length == 0 {
                break;
            }
            total += length as u64;
            if total > 512 * 1024 * 1024 {
                return None;
            }
            hash.update(&buffer[..length]);
        }
        files.insert(relative.to_owned(), hex::encode(hash.finalize()));
    }
    for (relative, link) in links {
        if cancel.is_cancelled() {
            return None;
        }
        // Reuse the digest of the in-scope regular file. Never open an alias as
        // an input or assume that a link to an untracked target captures its bytes.
        let target = link.canonicalize().ok()?;
        let name = target.strip_prefix(root).ok()?.to_str()?;
        let hash = files.get(name)?;
        files.insert(relative, format!("{LINK_PREFIX}{name}:{hash}"));
    }
    Some(files)
}

fn open_file(path: &Path) -> io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if file.metadata()?.is_file() {
        Ok(file)
    } else {
        Err(io::Error::other("scanner input changed type"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn local_file_symlinks_keep_result_reuse_but_invalidate_changed_inputs() {
        use std::fs;
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("real.go"), "package one").unwrap();
        fs::write(root.join("other.go"), "package one").unwrap();
        symlink("real.go", root.join("alias.go")).unwrap();
        let take = || collect(&root, GlobSet::empty(), &CancellationToken::new());
        let initial = take().expect("a local file alias should be cacheable");
        let cached = || Cache {
            key: "k".into(),
            files: initial.clone(),
            findings: Vec::new(),
        };
        for kind in [EngineKind::Trivy, EngineKind::Opengrep] {
            assert_eq!(
                plan(Some(cached()), take(), "k", kind).mode,
                ScanMode::Cached
            );
        }

        fs::write(root.join("real.go"), "package two").unwrap();
        let changed = plan(Some(cached()), take(), "k", EngineKind::Opengrep);
        assert!(
            matches!(changed.selection, Selection::Files(files) if files == ["alias.go", "real.go"])
        );
        fs::write(root.join("real.go"), "package one").unwrap();
        fs::remove_file(root.join("alias.go")).unwrap();
        symlink("other.go", root.join("alias.go")).unwrap();
        assert_ne!(take(), Some(initial));
    }

    #[cfg(unix)]
    #[test]
    fn local_linked_ignore_inputs_require_a_full_scan_after_edits_or_deletion() {
        use std::fs;
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("ignore-patterns"), "generated.go\n").unwrap();
        symlink("ignore-patterns", root.join(".semgrepignore")).unwrap();
        let take = || collect(&root, GlobSet::empty(), &CancellationToken::new());
        let initial = take().expect("linked ignore content is in the scan scope");
        let cached = || Cache {
            key: "k".into(),
            files: initial.clone(),
            findings: Vec::new(),
        };
        fs::write(root.join("ignore-patterns"), "another.go\n").unwrap();
        assert_eq!(
            plan(Some(cached()), take(), "k", EngineKind::Opengrep).mode,
            ScanMode::Full
        );
        fs::remove_file(root.join(".semgrepignore")).unwrap();
        assert_eq!(
            plan(Some(cached()), take(), "k", EngineKind::Opengrep).mode,
            ScanMode::Full
        );

        fs::write(root.join(".semgrepignore"), ":include linked-patterns\n").unwrap();
        symlink("ignore-patterns", root.join("linked-patterns")).unwrap();
        let initial = take().unwrap();
        fs::remove_file(root.join("linked-patterns")).unwrap();
        let cache = Cache {
            key: "k".into(),
            files: initial,
            findings: Vec::new(),
        };
        assert_eq!(
            plan(Some(cache), take(), "k", EngineKind::Opengrep).mode,
            ScanMode::Full
        );
    }

    #[cfg(unix)]
    #[test]
    fn local_symlinks_cannot_reuse_results_for_unhashed_targets() {
        use std::fs;
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        fs::create_dir(&project).unwrap();
        let root = project.canonicalize().unwrap();
        let target = root.join("excluded.txt");
        fs::write(&target, "excluded").unwrap();
        let settings = ScanSettings {
            excludes: vec!["excluded.txt".into()],
            ..Default::default()
        };
        symlink(&target, root.join("alias")).unwrap();
        let take = || {
            collect(
                &root,
                exclude_globs(&settings).unwrap(),
                &CancellationToken::new(),
            )
        };
        assert!(take().is_none());

        let outside = dir.path().join("outside.txt");
        fs::write(&outside, "outside").unwrap();
        fs::remove_file(root.join("alias")).unwrap();
        symlink(outside, root.join("alias")).unwrap();
        assert!(take().is_none());
    }

    #[test]
    fn changed_or_removed_files_invalidate_only_sast_findings() {
        let before = Snapshot::from([
            ("same.go".into(), "1".into()),
            ("edit.go".into(), "2".into()),
            ("old.go".into(), "3".into()),
        ]);
        let after = Snapshot::from([
            ("same.go".into(), "1".into()),
            ("edit.go".into(), "4".into()),
        ]);
        let create = || Cache {
            key: "k".into(),
            files: before.clone(),
            findings: before
                .keys()
                .map(|path| Finding {
                    fingerprint: path.clone(),
                    engine: "opengrep".into(),
                    check: crate::Check::Sast,
                    severity: crate::Severity::Low,
                    root: "/src".into(),
                    path: path.clone(),
                    line: Some(1),
                    rule: "rule".into(),
                    title: String::new(),
                    description: String::new(),
                    fix: String::new(),
                })
                .collect(),
        };
        let selected = plan(
            Some(create()),
            Some(after.clone()),
            "k",
            EngineKind::Opengrep,
        );
        assert!(matches!(selected.selection, Selection::Files(files) if files == ["edit.go"]));
        assert_eq!(selected.previous[0].path, "same.go");
        assert_eq!(selected.previous.len(), 1);
        assert_eq!(
            plan(Some(create()), Some(after), "k", EngineKind::Trivy).mode,
            ScanMode::Full
        );
        assert_eq!(
            plan(
                Some(create()),
                Some(before.clone()),
                "new DB",
                EngineKind::Trivy
            )
            .mode,
            ScanMode::Full
        );
        assert_eq!(
            plan(Some(create()), Some(before.clone()), "k", EngineKind::Trivy).mode,
            ScanMode::Cached
        );
    }
}
