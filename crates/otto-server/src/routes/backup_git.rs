//! Portable archive transport through an explicitly selected local Git repository.
//! Archive selection and sanitation belong exclusively to `state_archive`.

use crate::{
    auth::{require_root, CurrentUser},
    error::{ApiError, ApiResult},
    state::ServerCtx,
};
use axum::{extract::State, routing::post, Json, Router};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};

mod files;

const DIRECTORY: &str = ".otto-sync";
const MANIFEST: &str = ".otto-sync/manifest.json";
const ATTRIBUTES: &str = ".otto-sync/.gitattributes";
fn invalid(message: impl Into<String>) -> ApiError {
    otto_core::Error::Invalid(message.into()).into()
}
fn conflict(message: impl Into<String>) -> ApiError {
    otto_core::Error::Conflict(message.into()).into()
}
fn internal(error: impl std::fmt::Display) -> ApiError {
    otto_core::Error::Internal(error.to_string()).into()
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn json_bytes(value: &impl Serialize) -> ApiResult<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(internal)?;
    bytes.push(b'\n');
    Ok(bytes)
}
fn segment(value: &str) -> ApiResult<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(invalid("Invalid snapshot identifier"));
    }
    Ok(())
}
fn relative(value: &str) -> ApiResult<()> {
    if value.is_empty()
        || value.contains('\\')
        || value.chars().any(char::is_control)
        || Path::new(value)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        || value
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == "..")
    {
        return Err(invalid("Invalid snapshot relative path"));
    }
    Ok(())
}
/// Reject links at every existing component, including dangling symlinks.
fn safe_path(repo: &Path, relative_path: &str) -> ApiResult<PathBuf> {
    relative(relative_path)?;
    if !relative_path.starts_with(".otto-sync/") && relative_path != DIRECTORY {
        return Err(invalid("Path is outside the managed snapshot"));
    }
    let mut path = repo.to_path_buf();
    for component in Path::new(relative_path).components() {
        path.push(component);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(conflict("Snapshot paths must not contain symlinks"))
            }
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(internal(e)),
        }
    }
    Ok(path)
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    sync_format: u32,
    archive: Value,
    entries: BTreeMap<String, String>,
}
struct Snapshot {
    files: BTreeMap<String, Vec<u8>>,
    manifest_bytes: Vec<u8>,
}
impl Snapshot {
    fn from_archive(mut archive: Value) -> ApiResult<Self> {
        let object = archive
            .as_object_mut()
            .ok_or_else(|| invalid("Invalid archive"))?;
        // Capture time is informational; content identity must not change on each export.
        object.insert("snapshot_at".into(), json!("1970-01-01T00:00:00Z"));
        let records = object
            .remove("records")
            .and_then(|v| v.as_object().cloned())
            .ok_or_else(|| invalid("Archive records are missing"))?;
        let mut files = BTreeMap::new();
        files.insert(
            ATTRIBUTES.into(),
            b"** -text -filter -ident -working-tree-encoding\n".to_vec(),
        );
        for (table, rows) in records {
            segment(&table)?;
            files.insert(
                format!(".otto-sync/config/{table}.json"),
                json_bytes(&rows)?,
            );
        }
        let documents = object
            .get_mut("files")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| invalid("Archive files are missing"))?;
        for file in documents {
            let root = file["root"]
                .as_str()
                .ok_or_else(|| invalid("Missing file root"))?;
            let path = file["path"]
                .as_str()
                .ok_or_else(|| invalid("Missing file path"))?;
            segment(root)?;
            relative(path)?;
            let target = format!(".otto-sync/files/{root}/{path}");
            let content = STANDARD
                .decode(
                    file["content_base64"]
                        .as_str()
                        .ok_or_else(|| invalid("Missing file contents"))?,
                )
                .map_err(|_| invalid("Invalid file encoding"))?;
            if file["sha256"].as_str() != Some(&digest(&content)) {
                return Err(invalid("Archive file checksum mismatch"));
            }
            if files.insert(target, content).is_some() {
                return Err(invalid("Duplicate snapshot file"));
            }
            file.as_object_mut()
                .ok_or_else(|| invalid("Invalid file entry"))?
                .remove("content_base64");
        }
        if files.len() > files::MAX_ENTRIES
            || files.values().map(Vec::len).sum::<usize>() > files::MAX_TOTAL
            || files.values().any(|v| v.len() > files::MAX_FILE)
        {
            return Err(invalid("Snapshot exceeds file count or size limits"));
        }
        let entries = files.iter().map(|(p, b)| (p.clone(), digest(b))).collect();
        let manifest_bytes = json_bytes(&Manifest {
            sync_format: 1,
            archive,
            entries,
        })?;
        if manifest_bytes.len() > files::MAX_FILE
            || manifest_bytes.len() + files.values().map(Vec::len).sum::<usize>() > files::MAX_TOTAL
        {
            return Err(invalid("Snapshot manifest or total exceeds size limits"));
        }
        Ok(Self {
            files,
            manifest_bytes,
        })
    }
    fn all_files(&self) -> BTreeMap<String, Vec<u8>> {
        let mut files = self.files.clone();
        files.insert(MANIFEST.into(), self.manifest_bytes.clone());
        files
    }
}
fn existing_manifest(repo: &Path) -> ApiResult<Option<Manifest>> {
    let path = safe_path(repo, MANIFEST)?;
    if !path.exists() {
        if safe_path(repo, DIRECTORY)?.exists() {
            return Err(conflict(".otto-sync already exists without an Otto manifest; choose another repository or move that directory"));
        }
        return Ok(None);
    }
    let manifest: Manifest = serde_json::from_slice(
        &files::read(repo, MANIFEST)?.ok_or_else(|| conflict("Snapshot manifest disappeared"))?,
    )
    .map_err(|_| invalid("Invalid Git snapshot manifest"))?;
    if manifest.entries.len() > files::MAX_ENTRIES {
        return Err(invalid("Snapshot has too many files"));
    }
    if manifest.sync_format != 1 {
        return Err(invalid("Unsupported Git snapshot format"));
    }
    for (path, hash) in &manifest.entries {
        safe_path(repo, path)?;
        if path == MANIFEST
            || !(path == ATTRIBUTES
                || path.starts_with(".otto-sync/config/")
                || path.starts_with(".otto-sync/files/"))
            || hash.len() != 64
        {
            return Err(invalid("Invalid managed manifest entry"));
        }
    }
    Ok(Some(manifest))
}
fn read_snapshot(repo: &Path) -> ApiResult<Value> {
    let manifest =
        existing_manifest(repo)?.ok_or_else(|| invalid("This repository has no Otto snapshot"))?;
    let mut files = BTreeMap::new();
    let mut total = 0usize;
    for (path, hash) in manifest.entries {
        safe_path(repo, &path)?;
        let bytes =
            files::read(repo, &path)?.ok_or_else(|| conflict("Snapshot file is missing"))?;
        total += bytes.len();
        if total > files::MAX_TOTAL {
            return Err(invalid("Snapshot exceeds 256 MiB"));
        }
        if digest(&bytes) != hash {
            return Err(conflict(format!("Snapshot checksum mismatch: {path}")));
        }
        files.insert(path, bytes);
    }
    let mut archive = manifest.archive;
    let mut records = serde_json::Map::new();
    for (path, bytes) in &files {
        if let Some(table) = path
            .strip_prefix(".otto-sync/config/")
            .and_then(|s| s.strip_suffix(".json"))
        {
            segment(table)?;
            records.insert(
                table.into(),
                serde_json::from_slice(bytes)
                    .map_err(|_| invalid("Invalid snapshot config JSON"))?,
            );
        }
    }
    archive
        .as_object_mut()
        .ok_or_else(|| invalid("Snapshot archive metadata must be an object"))?
        .insert("records".into(), Value::Object(records));
    for file in archive["files"]
        .as_array_mut()
        .ok_or_else(|| invalid("Missing file list"))?
    {
        let path = format!(
            ".otto-sync/files/{}/{}",
            file["root"]
                .as_str()
                .ok_or_else(|| invalid("Missing file root"))?,
            file["path"]
                .as_str()
                .ok_or_else(|| invalid("Missing file path"))?
        );
        let bytes = files
            .get(&path)
            .ok_or_else(|| invalid("Document missing from snapshot manifest"))?;
        file["content_base64"] = json!(STANDARD.encode(bytes));
    }
    Ok(archive)
}

async fn git(repo: &Path, args: &[&str]) -> ApiResult<String> {
    let remote = args
        .first()
        .is_some_and(|arg| matches!(*arg, "fetch" | "push" | "clone"));
    let mut command = vec!["--literal-pathspecs", "-c", "core.hooksPath=/dev/null"];
    command.extend_from_slice(args);
    otto_git::LocalGit::new(repo)
        .run_automation(&command, remote)
        .await
        .map_err(|error| conflict(otto_core::redact::redact_text(&error.to_string()).value))
}
async fn canonical_repo(path: &str) -> ApiResult<PathBuf> {
    let path = std::fs::canonicalize(path)
        .map_err(|_| invalid("Select an existing local Git repository"))?;
    let root = git(&path, &["rev-parse", "--show-toplevel"]).await?;
    std::fs::canonicalize(root.strip_suffix("\n").unwrap_or(&root)).map_err(internal)
}
#[derive(Debug, Serialize)]
pub struct GitStatus {
    repo_path: String,
    head: Option<String>,
    branch: Option<String>,
    upstream: Option<String>,
    ahead: u64,
    behind: u64,
    dirty: bool,
    remotes: Vec<String>,
}
async fn status(repo: &Path) -> ApiResult<GitStatus> {
    let head = git(repo, &["rev-parse", "--verify", "HEAD"])
        .await
        .ok()
        .map(|s| s.trim().to_owned());
    let branch = git(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .await
        .ok()
        .map(|s| s.trim().to_owned());
    let upstream = git(
        repo,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    )
    .await
    .ok()
    .map(|s| s.trim().to_owned());
    let counts = if upstream.is_some() {
        git(
            repo,
            &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
        )
        .await?
    } else {
        String::new()
    };
    let mut counts = counts.split_whitespace().filter_map(|s| s.parse().ok());
    Ok(GitStatus {
        repo_path: repo.display().to_string(),
        head,
        branch,
        upstream,
        ahead: counts.next().unwrap_or(0),
        behind: counts.next().unwrap_or(0),
        dirty: !git(repo, &["status", "--porcelain"]).await?.is_empty(),
        remotes: git(repo, &["remote"])
            .await?
            .lines()
            .map(str::to_owned)
            .collect(),
    })
}
#[derive(Debug, Serialize)]
struct FileChange {
    path: String,
    action: String,
    bytes: usize,
}
#[derive(Debug, Serialize)]
pub struct ExportPreview {
    token: String,
    snapshot_digest: String,
    status: GitStatus,
    changes: Vec<FileChange>,
    excluded: Vec<String>,
    reconnect: Vec<String>,
}
async fn preview_snapshot(repo: &Path, snapshot: &Snapshot) -> ApiResult<ExportPreview> {
    let old = existing_manifest(repo)?;
    let mut current = BTreeMap::<String, Option<String>>::new();
    let desired = snapshot.all_files();
    let mut paths = desired
        .keys()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    if let Some(manifest) = &old {
        paths.extend(manifest.entries.keys().cloned());
    }
    let mut changes = Vec::new();
    let mut total_bytes = 0usize;
    for path in paths {
        safe_path(repo, &path)?;
        let bytes = files::read(repo, &path)?;
        total_bytes += bytes.as_ref().map(Vec::len).unwrap_or(0);
        if total_bytes > files::MAX_TOTAL {
            return Err(invalid("Existing snapshot exceeds 256 MiB"));
        }
        let hash = bytes.as_ref().map(|b| digest(b));
        if path != MANIFEST {
            match old.as_ref().and_then(|m| m.entries.get(&path)) {
                Some(expected) if hash.as_ref() != Some(expected) => return Err(conflict(format!("Previously exported file changed locally: {path}. Commit or restore the Git snapshot before exporting again."))),
                None if bytes.is_some() => return Err(conflict(format!("Unowned file would be overwritten: {path}"))),
                _ => ()
            }
        }
        let action = match (desired.get(&path), bytes.as_ref()) {
            (Some(a), Some(b)) if a == b => "unchanged",
            (Some(_), Some(_)) => "modified",
            (Some(_), None) => "added",
            (None, _) => "removed",
        };
        changes.push(FileChange {
            path: path.clone(),
            action: action.into(),
            bytes: desired.get(&path).map(Vec::len).unwrap_or(0),
        });
        current.insert(path, hash);
    }
    let status = status(repo).await?;
    let snapshot_digest = digest(&snapshot.manifest_bytes);
    let token = digest(&json_bytes(
        &json!({"repo":repo,"snapshot":snapshot_digest,"head":status.head,"current":current}),
    )?);
    let manifest: Manifest = serde_json::from_slice(&snapshot.manifest_bytes).map_err(internal)?;
    let list = |key| {
        manifest.archive[key]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    Ok(ExportPreview {
        token,
        snapshot_digest,
        status,
        changes,
        excluded: list("excluded"),
        reconnect: list("reconnect"),
    })
}
async fn export_snapshot(
    repo: &Path,
    snapshot: &Snapshot,
    token: &str,
) -> ApiResult<ExportPreview> {
    let preview = preview_snapshot(repo, snapshot).await?;
    if preview.token != token {
        return Err(conflict(
            "Export preview is stale; preview the snapshot again",
        ));
    }
    let desired = snapshot.all_files();
    // Pre-read expected hashes, then check again immediately before each publication.
    let mut expected = BTreeMap::new();
    for change in &preview.changes {
        expected.insert(
            change.path.clone(),
            files::read(repo, &change.path)?.map(|b| digest(&b)),
        );
    }
    if preview_snapshot(repo, snapshot).await?.token != token {
        return Err(conflict("Export preview is stale; preview again"));
    }
    for change in &preview.changes {
        if change.path == MANIFEST || change.action == "unchanged" {
            continue;
        }
        safe_path(repo, &change.path)?;
        if change.action == "removed" {
            files::remove(
                repo,
                &change.path,
                expected[&change.path]
                    .as_deref()
                    .ok_or_else(|| conflict("Snapshot file disappeared"))?,
            )?;
        } else {
            files::publish(
                repo,
                &change.path,
                &desired[&change.path],
                expected[&change.path].as_deref(),
            )?;
        }
    }
    files::publish(
        repo,
        MANIFEST,
        &snapshot.manifest_bytes,
        expected[MANIFEST].as_deref(),
    )?;
    preview_snapshot(repo, snapshot).await
}
async fn require_head(repo: &Path, expected: Option<&str>) -> ApiResult<GitStatus> {
    let current = status(repo).await?;
    if current.head.as_deref() != expected {
        return Err(conflict(
            "Repository HEAD changed; refresh its status before continuing",
        ));
    }
    Ok(current)
}
async fn commit_snapshot(
    repo: &Path,
    expected: Option<&str>,
    message: &str,
    snapshot_digest: &str,
) -> ApiResult<GitStatus> {
    if message.trim().is_empty() || message.len() > 8192 {
        return Err(invalid("Enter a commit message of 1–8192 bytes"));
    }
    require_head(repo, expected).await?;
    read_snapshot(repo)?; // Check all hashes before staging.
    let bytes =
        files::read(repo, MANIFEST)?.ok_or_else(|| invalid("Snapshot manifest is missing"))?;
    if digest(&bytes) != snapshot_digest {
        return Err(conflict("Snapshot changed; preview it before committing"));
    }
    let manifest = existing_manifest(repo)?.ok_or_else(|| invalid("Export a snapshot first"))?;
    let mut paths = manifest
        .entries
        .keys()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    paths.insert(MANIFEST.into());
    // Include removed files tracked by the preceding snapshot, but never unlisted files.
    if let Ok(previous) = git(repo, &["show", "HEAD:.otto-sync/manifest.json"]).await {
        if let Ok(previous) = serde_json::from_str::<Manifest>(&previous) {
            for path in previous.entries.keys() {
                safe_path(repo, path)?;
                if path != ATTRIBUTES
                    && !path.starts_with(".otto-sync/config/")
                    && !path.starts_with(".otto-sync/files/")
                {
                    return Err(invalid("Invalid previous snapshot path"));
                }
                paths.insert(path.clone());
            }
        }
    }
    // A NUL-delimited pathspec file preserves literal names and avoids ARG_MAX
    // for documentation vaults containing thousands of snapshot files.
    use std::io::Write;
    let mut pathspec = tempfile::NamedTempFile::new().map_err(internal)?;
    for path in &paths {
        pathspec.write_all(path.as_bytes()).map_err(internal)?;
        pathspec.write_all(&[0]).map_err(internal)?;
    }
    pathspec.flush().map_err(internal)?;
    let option = format!("--pathspec-from-file={}", pathspec.path().display());
    git(repo, &["add", "-A", &option, "--pathspec-file-nul"]).await?;
    git(
        repo,
        &[
            "commit",
            "--only",
            "-m",
            message,
            &option,
            "--pathspec-file-nul",
        ],
    )
    .await?;
    status(repo).await
}
async fn sync_repo(
    repo: &Path,
    expected: Option<&str>,
    action: &str,
    remote: &str,
) -> ApiResult<GitStatus> {
    let current = require_head(repo, expected).await?;
    if !current.remotes.iter().any(|r| r == remote) || remote.starts_with('-') {
        return Err(invalid("Choose a configured Git remote"));
    }
    let branch = current
        .branch
        .as_deref()
        .ok_or_else(|| conflict("Check out a branch before synchronizing"))?;
    match action {
        "fetch" => {
            git(repo, &["fetch", "--", remote]).await?;
        }
        "pull" => {
            if current.dirty {
                return Err(conflict(
                    "Commit or stash repository changes before pulling",
                ));
            }
            // A private ref avoids another Git client replacing FETCH_HEAD between
            // our fetch and merge. Only this operation owns this temporary ref.
            let reference = format!("refs/otto-sync/{}", uuid::Uuid::new_v4());
            let refspec = format!("refs/heads/{branch}:{reference}");
            let result: ApiResult<()> = async {
                git(
                    repo,
                    &[
                        "fetch",
                        "--no-write-fetch-head",
                        "--no-tags",
                        "--",
                        remote,
                        &refspec,
                    ],
                )
                .await?;
                let target = git(repo, &["rev-parse", "--verify", &reference]).await?;
                let latest = require_head(repo, expected).await?;
                if latest.dirty || latest.branch != current.branch {
                    return Err(conflict(
                        "Repository changed during fetch; refresh before pulling",
                    ));
                }
                git(repo, &["merge", "--ff-only", "--no-edit", target.trim()]).await?;
                Ok(())
            }
            .await;
            let cleanup = git(repo, &["update-ref", "-d", &reference]).await;
            result?;
            cleanup?;
        }
        "push" => {
            let source = expected.ok_or_else(|| invalid("Commit before pushing"))?;
            let target = format!("{source}:refs/heads/{branch}");
            git(repo, &["push", "--", remote, &target]).await?;
        }
        _ => return Err(invalid("Sync action must be fetch, pull, or push")),
    }
    status(repo).await
}

// Weak entries bound the lock registry to active repositories. Guards are owned by
// detached mutation tasks, so disconnecting an HTTP client cannot release a lock early.
async fn repo_lock(repo: &Path) -> tokio::sync::OwnedMutexGuard<()> {
    static LOCKS: OnceLock<Mutex<BTreeMap<PathBuf, Weak<tokio::sync::Mutex<()>>>>> =
        OnceLock::new();
    let lock = {
        let mut map = LOCKS
            .get_or_init(Default::default)
            .lock()
            .expect("repository locks");
        map.retain(|_, v| v.strong_count() > 0);
        let lock = map
            .get(repo)
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| Arc::new(tokio::sync::Mutex::new(())));
        map.insert(repo.to_owned(), Arc::downgrade(&lock));
        lock
    };
    lock.lock_owned().await
}

#[derive(Deserialize)]
pub struct RepoRequest {
    repo_path: String,
}
#[derive(Deserialize)]
pub struct ExportRequest {
    repo_path: String,
    preview_token: String,
}
#[derive(Deserialize)]
pub struct CommitRequest {
    repo_path: String,
    expected_head: Option<String>,
    snapshot_digest: String,
    message: String,
}
#[derive(Deserialize)]
pub struct SyncRequest {
    repo_path: String,
    expected_head: Option<String>,
    action: String,
    remote: String,
}
#[derive(Deserialize)]
pub struct RestoreRequest {
    repo_path: String,
    conflicts: crate::state_archive::ConflictPolicy,
    #[serde(default)]
    preview_token: String,
    #[serde(default)]
    confirm: bool,
}

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route("/state/git/status", post(repo_status))
        .route("/state/git/preview", post(preview))
        .route("/state/git/export", post(export))
        .route("/state/git/commit", post(commit))
        .route("/state/git/sync", post(sync))
        .route("/state/git/restore/preview", post(restore_preview))
        .route("/state/git/restore", post(restore))
}
async fn build(ctx: &ServerCtx) -> ApiResult<Snapshot> {
    let archive = crate::state_archive::build_snapshot(
        ctx,
        crate::state_archive::SnapshotOptions {
            portable: true,
            include_files: true,
        },
    )
    .await?;
    Snapshot::from_archive(serde_json::to_value(archive).map_err(internal)?)
}
async fn repo_status(
    CurrentUser(user): CurrentUser,
    Json(req): Json<RepoRequest>,
) -> ApiResult<Json<GitStatus>> {
    require_root(&user)?;
    Ok(Json(status(&canonical_repo(&req.repo_path).await?).await?))
}
async fn preview(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<RepoRequest>,
) -> ApiResult<Json<ExportPreview>> {
    require_root(&user)?;
    let repo = canonical_repo(&req.repo_path).await?;
    let _guard = repo_lock(&repo).await;
    Ok(Json(preview_snapshot(&repo, &build(&ctx).await?).await?))
}
async fn export(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<ExportRequest>,
) -> ApiResult<Json<ExportPreview>> {
    require_root(&user)?;
    let repo = canonical_repo(&req.repo_path).await?;
    let guard = repo_lock(&repo).await;
    tokio::spawn(async move {
        let _guard = guard;
        let result = export_snapshot(&repo, &build(&ctx).await?, &req.preview_token).await?;
        super::backup::record_action(
            &ctx,
            &user.id,
            "state.git.export",
            Some(&repo.display().to_string()),
            None,
        )
        .await;
        Ok(Json(result))
    })
    .await
    .map_err(internal)?
}
async fn commit(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CommitRequest>,
) -> ApiResult<Json<Value>> {
    require_root(&user)?;
    let repo = canonical_repo(&req.repo_path).await?;
    let guard = repo_lock(&repo).await;
    tokio::spawn(async move {
        let _guard = guard;
        let status = commit_snapshot(
            &repo,
            req.expected_head.as_deref(),
            &req.message,
            &req.snapshot_digest,
        )
        .await?;
        super::backup::record_action(
            &ctx,
            &user.id,
            "state.git.commit",
            Some(&repo.display().to_string()),
            None,
        )
        .await;
        Ok(Json(json!({"status":status})))
    })
    .await
    .map_err(internal)?
}
async fn sync(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<SyncRequest>,
) -> ApiResult<Json<Value>> {
    require_root(&user)?;
    let repo = canonical_repo(&req.repo_path).await?;
    let guard = repo_lock(&repo).await;
    tokio::spawn(async move {
        let _guard = guard;
        let status = sync_repo(
            &repo,
            req.expected_head.as_deref(),
            &req.action,
            &req.remote,
        )
        .await?;
        super::backup::record_action(
            &ctx,
            &user.id,
            "state.git.sync",
            Some(&repo.display().to_string()),
            Some(json!({"action":req.action})),
        )
        .await;
        Ok(Json(json!({"status":status})))
    })
    .await
    .map_err(internal)?
}
async fn restore_preview(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<RestoreRequest>,
) -> ApiResult<Json<crate::state_archive::RestorePreview>> {
    require_root(&user)?;
    let repo = canonical_repo(&req.repo_path).await?;
    let _guard = repo_lock(&repo).await;
    let archive = serde_json::from_value(read_snapshot(&repo)?)
        .map_err(|_| invalid("Invalid snapshot archive"))?;
    Ok(Json(
        crate::state_archive::preview_restore(&ctx, &archive, req.conflicts).await?,
    ))
}
async fn restore(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<RestoreRequest>,
) -> ApiResult<Json<crate::state_archive::RestoreResult>> {
    require_root(&user)?;
    let repo = canonical_repo(&req.repo_path).await?;
    let guard = repo_lock(&repo).await;
    tokio::spawn(async move {
        let _guard = guard;
        let archive = serde_json::from_value(read_snapshot(&repo)?)
            .map_err(|_| invalid("Invalid snapshot archive"))?;
        let result = crate::state_archive::restore_snapshot(
            &ctx,
            &archive,
            crate::state_archive::RestoreOptions {
                conflicts: req.conflicts,
                preview_token: req.preview_token,
                confirm: req.confirm,
            },
        )
        .await?;
        super::backup::record_action(
            &ctx,
            &user.id,
            "state.git.restore",
            Some(&repo.display().to_string()),
            None,
        )
        .await;
        Ok(Json(result))
    })
    .await
    .map_err(internal)?
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    async fn repo() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-b", "main"]).await.unwrap();
        git(dir.path(), &["config", "user.name", "Otto fixture"])
            .await
            .unwrap();
        git(
            dir.path(),
            &["config", "user.email", "fixture@example.invalid"],
        )
        .await
        .unwrap();
        dir
    }
    fn archive() -> Value {
        json!({"archive_format":2,"schema_version":1,"daemon_version":"test","snapshot_at":"now","records":{"settings":[{"key":"theme","value":"dark"}]},"roots":[{"id":"vault","kind":"vault"}],"files":[{"root":"vault","path":"note.md","sha256":digest(b"hello"),"content_base64":"aGVsbG8="}],"excluded":[],"reconnect":[]})
    }
    #[tokio::test]
    async fn export_is_stable_preserves_unrelated_and_round_trips() {
        let dir = repo().await;
        std::fs::write(dir.path().join("personal.txt"), "keep").unwrap();
        let snapshot = Snapshot::from_archive(archive()).unwrap();
        let preview = preview_snapshot(dir.path(), &snapshot).await.unwrap();
        export_snapshot(dir.path(), &snapshot, &preview.token)
            .await
            .unwrap();
        let first = std::fs::read(dir.path().join(MANIFEST)).unwrap();
        let mut next = archive();
        next["snapshot_at"] = json!("later");
        let next = Snapshot::from_archive(next).unwrap();
        let preview = preview_snapshot(dir.path(), &next).await.unwrap();
        assert!(preview.changes.iter().all(|c| c.action == "unchanged"));
        export_snapshot(dir.path(), &next, &preview.token)
            .await
            .unwrap();
        assert_eq!(first, std::fs::read(dir.path().join(MANIFEST)).unwrap());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("personal.txt")).unwrap(),
            "keep"
        );
        let restored = read_snapshot(dir.path()).unwrap();
        assert_eq!(restored["records"], archive()["records"]);
        assert_eq!(restored["files"], archive()["files"]);
    }
    #[tokio::test]
    async fn stale_preview_and_edited_managed_files_are_rejected() {
        let dir = repo().await;
        let snapshot = Snapshot::from_archive(archive()).unwrap();
        let preview = preview_snapshot(dir.path(), &snapshot).await.unwrap();
        std::fs::create_dir(dir.path().join(".otto-sync")).unwrap();
        std::fs::write(dir.path().join(".otto-sync/personal"), "keep").unwrap();
        assert!(export_snapshot(dir.path(), &snapshot, &preview.token)
            .await
            .is_err());
        std::fs::remove_file(dir.path().join(".otto-sync/personal")).unwrap();
        std::fs::remove_dir(dir.path().join(".otto-sync")).unwrap();
        let preview = preview_snapshot(dir.path(), &snapshot).await.unwrap();
        export_snapshot(dir.path(), &snapshot, &preview.token)
            .await
            .unwrap();
        std::fs::write(
            dir.path().join(".otto-sync/config/settings.json"),
            "user edit",
        )
        .unwrap();
        assert!(preview_snapshot(dir.path(), &snapshot).await.is_err());
    }
    #[tokio::test]
    async fn commit_only_snapshot_preserves_unrelated_staging() {
        let dir = repo().await;
        std::fs::write(dir.path().join("personal.txt"), "staged work").unwrap();
        git(dir.path(), &["add", "personal.txt"]).await.unwrap();
        let snapshot = Snapshot::from_archive(archive()).unwrap();
        let preview = preview_snapshot(dir.path(), &snapshot).await.unwrap();
        export_snapshot(dir.path(), &snapshot, &preview.token)
            .await
            .unwrap();
        commit_snapshot(
            dir.path(),
            None,
            "snapshot",
            &digest(&snapshot.manifest_bytes),
        )
        .await
        .unwrap();
        assert!(git(dir.path(), &["show", "HEAD:personal.txt"])
            .await
            .is_err());
        assert_eq!(
            git(dir.path(), &["diff", "--cached", "--name-only"])
                .await
                .unwrap()
                .trim(),
            "personal.txt"
        );
        assert_eq!(
            git(dir.path(), &["show", "HEAD:.otto-sync/files/vault/note.md"])
                .await
                .unwrap(),
            "hello"
        );
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_destination_and_parent_traversal_are_rejected() {
        let dir = repo().await;
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join(".otto-sync")).unwrap();
        let snapshot = Snapshot::from_archive(archive()).unwrap();
        assert!(preview_snapshot(dir.path(), &snapshot).await.is_err());
        let mut invalid = archive();
        invalid["files"][0]["path"] = json!("../outside");
        assert!(Snapshot::from_archive(invalid).is_err());
    }
    async fn export_commit(repo: &Path, archive: Value) -> GitStatus {
        let snapshot = Snapshot::from_archive(archive).unwrap();
        let preview = preview_snapshot(repo, &snapshot).await.unwrap();
        export_snapshot(repo, &snapshot, &preview.token)
            .await
            .unwrap();
        commit_snapshot(
            repo,
            preview.status.head.as_deref(),
            "test: save snapshot",
            &preview.snapshot_digest,
        )
        .await
        .unwrap()
    }
    #[tokio::test]
    async fn remote_sync_fast_forwards_and_refuses_divergence() {
        let source = repo().await;
        let bare = tempfile::tempdir().unwrap();
        git(bare.path(), &["init", "--bare", "--initial-branch=main"])
            .await
            .unwrap();
        git(
            source.path(),
            &["remote", "add", "origin", bare.path().to_str().unwrap()],
        )
        .await
        .unwrap();
        let first = export_commit(source.path(), archive()).await;
        sync_repo(source.path(), first.head.as_deref(), "push", "origin")
            .await
            .unwrap();
        let other = tempfile::tempdir().unwrap();
        git(
            other.path(),
            &["clone", bare.path().to_str().unwrap(), "checkout"],
        )
        .await
        .unwrap();
        let checkout = other.path().join("checkout");
        git(&checkout, &["config", "user.name", "Otto fixture"])
            .await
            .unwrap();
        git(
            &checkout,
            &["config", "user.email", "fixture@example.invalid"],
        )
        .await
        .unwrap();
        let mut changed = archive();
        changed["records"]["settings"][0]["value"] = json!("light");
        let second = export_commit(source.path(), changed.clone()).await;
        sync_repo(source.path(), second.head.as_deref(), "push", "origin")
            .await
            .unwrap();
        let pulled = sync_repo(&checkout, first.head.as_deref(), "pull", "origin")
            .await
            .unwrap();
        assert_eq!(pulled.head, second.head);
        assert_eq!(
            read_snapshot(&checkout).unwrap()["records"],
            changed["records"]
        );
        std::fs::write(checkout.join("local.txt"), "local").unwrap();
        git(&checkout, &["add", "local.txt"]).await.unwrap();
        git(
            &checkout,
            &["commit", "-m", "test: local independent change"],
        )
        .await
        .unwrap();
        let divergent = status(&checkout).await.unwrap();
        changed["records"]["settings"][0]["value"] = json!("system");
        let third = export_commit(source.path(), changed).await;
        sync_repo(source.path(), third.head.as_deref(), "push", "origin")
            .await
            .unwrap();
        assert!(
            sync_repo(&checkout, divergent.head.as_deref(), "pull", "origin")
                .await
                .is_err()
        );
        assert_eq!(status(&checkout).await.unwrap().head, divergent.head);
        assert!(git(&checkout, &["for-each-ref", "refs/otto-sync/"])
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            std::fs::read_to_string(checkout.join("local.txt")).unwrap(),
            "local"
        );
    }
    #[tokio::test]
    async fn removed_owned_files_commit_but_unlisted_files_survive() {
        let dir = repo().await;
        export_commit(dir.path(), archive()).await;
        std::fs::write(
            dir.path().join(".otto-sync/files/vault/personal.md"),
            "personal",
        )
        .unwrap();
        let mut archive = archive();
        archive["files"] = json!([]);
        export_commit(dir.path(), archive).await;
        assert!(!dir.path().join(".otto-sync/files/vault/note.md").exists());
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".otto-sync/files/vault/personal.md")).unwrap(),
            "personal"
        );
        assert!(
            git(dir.path(), &["show", "HEAD:.otto-sync/files/vault/note.md"])
                .await
                .is_err()
        );
        assert!(git(
            dir.path(),
            &["show", "HEAD:.otto-sync/files/vault/personal.md"]
        )
        .await
        .is_err());
    }
    #[tokio::test]
    async fn head_change_invalidates_export_and_commit() {
        let dir = repo().await;
        let snapshot = Snapshot::from_archive(archive()).unwrap();
        let preview = preview_snapshot(dir.path(), &snapshot).await.unwrap();
        let different_repo = repo().await;
        assert!(
            export_snapshot(different_repo.path(), &snapshot, &preview.token)
                .await
                .is_err()
        );
        assert!(!different_repo.path().join(DIRECTORY).exists());
        git(
            dir.path(),
            &["commit", "--allow-empty", "-m", "test: independent head"],
        )
        .await
        .unwrap();
        assert!(export_snapshot(dir.path(), &snapshot, &preview.token)
            .await
            .is_err());
        let preview = preview_snapshot(dir.path(), &snapshot).await.unwrap();
        export_snapshot(dir.path(), &snapshot, &preview.token)
            .await
            .unwrap();
        assert!(commit_snapshot(
            dir.path(),
            None,
            "test: stale snapshot",
            &preview.snapshot_digest
        )
        .await
        .is_err());
    }
    #[tokio::test]
    async fn registered_vault_git_export_does_not_capture_itself() {
        let dir = repo().await;
        let data = tempfile::tempdir().unwrap();
        let pool = otto_state::db::test_pool().await;
        sqlx::query("INSERT INTO workspaces(id,name,root_path,created_at) VALUES('snapshot-ws','Snapshot',?,'now')").bind(dir.path().to_str().unwrap()).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO vaults(id,ws_id,name,root_path,created_at) VALUES(400,'snapshot-ws','Snapshot Vault',?,'now')").bind(dir.path().to_str().unwrap()).execute(&pool).await.unwrap();
        std::fs::write(dir.path().join("guide.md"), "# Complete document\n").unwrap();
        let options = crate::state_archive::SnapshotOptions {
            portable: true,
            include_files: true,
        };
        let first =
            crate::state_archive::build_snapshot_parts(&pool, data.path(), "test", options.clone())
                .await
                .unwrap();
        assert!(first.files.iter().any(|f| f.path == "guide.md"));
        let first = Snapshot::from_archive(serde_json::to_value(first).unwrap()).unwrap();
        let preview = preview_snapshot(dir.path(), &first).await.unwrap();
        export_snapshot(dir.path(), &first, &preview.token)
            .await
            .unwrap();
        let second =
            crate::state_archive::build_snapshot_parts(&pool, data.path(), "test", options)
                .await
                .unwrap();
        assert!(!second.files.iter().any(|f| f.path.contains(".otto-sync")));
        let second = Snapshot::from_archive(serde_json::to_value(second).unwrap()).unwrap();
        assert_eq!(first.manifest_bytes, second.manifest_bytes);
        assert_eq!(first.files, second.files);
        let preview = preview_snapshot(dir.path(), &second).await.unwrap();
        assert!(preview.changes.iter().all(|f| f.action == "unchanged"));
        export_snapshot(dir.path(), &second, &preview.token)
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn malformed_manifest_returns_an_error_instead_of_panicking() {
        let dir = repo().await;
        files::publish(
            dir.path(),
            MANIFEST,
            br#"{"sync_format":1,"archive":"invalid","entries":{}}"#,
            None,
        )
        .unwrap();
        assert!(read_snapshot(dir.path()).is_err());
    }
    #[tokio::test]
    async fn git_attributes_cannot_transform_snapshot_bytes() {
        let dir = repo().await;
        std::fs::write(dir.path().join(".gitattributes"), "*.md text eol=lf\n").unwrap();
        let mut archive = archive();
        let bytes = b"first\r\nsecond\r\n";
        archive["files"][0]["sha256"] = json!(digest(bytes));
        archive["files"][0]["content_base64"] = json!(STANDARD.encode(bytes));
        export_commit(dir.path(), archive).await;
        assert_eq!(
            git(dir.path(), &["show", "HEAD:.otto-sync/files/vault/note.md"])
                .await
                .unwrap()
                .as_bytes(),
            bytes
        );
    }
}
