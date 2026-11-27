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
            let renamed_ignore = cache
                .files
                .keys()
                .any(|path| path.ends_with(".semgrepignore") && !current.contains_key(path));
            if !renamed_ignore
                && changed.len() <= 64
                && changed
                    .iter()
                    .all(|p| !p.ends_with(".semgrepignore") && include_safe(p))
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
        if !kind.is_file() || files.len() >= 100_000 {
            // Symlinks and special files make the scan's inputs ambiguous.
            return None;
        }
        let relative = entry.path().strip_prefix(root).ok()?.to_str()?;
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
