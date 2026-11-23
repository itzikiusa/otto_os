//! Clean blob reader (`GET /repos/{id}/file`) and status query helpers.
//! Split out of `http.rs` to stay under the per-file LOC ratchet.

use std::path::{Component, Path as FsPath};
use std::process::Stdio;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use otto_core::auth::AuthUser;
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id, Result};
use serde::{Deserialize, Deserializer, Serialize};
use tokio::io::AsyncReadExt;

use super::{repo_ctx, ApiResult, GitCtx};
use crate::local::LocalGit;

const MAX_GIT_FILE_BYTES: usize = 4 * 1024 * 1024;
const GIT_FILE_TIMEOUT: Duration = Duration::from_secs(3);
const GIT_RENAME_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Default, Deserialize)]
pub(crate) struct StatusQuery {
    #[serde(default, deserialize_with = "deserialize_boolish")]
    pub fresh: bool,
}

fn deserialize_boolish<'de, D>(deserializer: D) -> std::result::Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum RawBool {
        Bool(bool),
        Int(i64),
        Str(String),
    }

    match Option::<RawBool>::deserialize(deserializer)? {
        None => Ok(false),
        Some(RawBool::Bool(b)) => Ok(b),
        Some(RawBool::Int(n)) => Ok(n != 0),
        Some(RawBool::Str(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "" | "0" | "false" | "no" | "off" => Ok(false),
            "1" | "true" | "yes" | "on" => Ok(true),
            other => Err(serde::de::Error::custom(format!(
                "invalid boolean query value '{other}'"
            ))),
        },
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct RepoFileQuery {
    pub path: String,
    #[serde(default)]
    pub rev: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoFileResp {
    pub exists: bool,
    pub content: Option<String>,
    pub binary: bool,
    pub truncated: bool,
}

impl RepoFileResp {
    const fn missing() -> Self {
        Self {
            exists: false,
            content: None,
            binary: false,
            truncated: false,
        }
    }
}

#[cfg(unix)]
struct KillPgOnDrop(Option<i32>);

#[cfg(unix)]
impl Drop for KillPgOnDrop {
    fn drop(&mut self) {
        if let Some(pgid) = self.0.take() {
            if pgid > 0 {
                // SAFETY: child was spawned with `process_group(0)` so `pgid == child_pid`.
                unsafe {
                    let _ = libc::killpg(pgid, libc::SIGKILL);
                }
            }
        }
    }
}

#[cfg(not(unix))]
struct KillPgOnDrop(#[allow(dead_code)] Option<i32>);

pub(crate) async fn resolve_project_repo_toplevel(raw_path: &str) -> Result<String> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err(Error::Invalid("path must not be empty".into()));
    }
    let expanded = otto_core::paths::expand_home_strict(trimmed).map_err(Error::Invalid)?;
    if otto_core::paths::is_broad_non_project_root(&expanded) {
        return Err(Error::Invalid(
            "refusing broad non-project directory (e.g. $HOME or system root) as a Git repository"
                .into(),
        ));
    }
    let top = LocalGit::new(&expanded)
        .toplevel()
        .await
        .map_err(|_| Error::Invalid(format!("not a git repository: {raw_path}")))?;
    if otto_core::paths::is_broad_non_project_root(FsPath::new(&top)) {
        return Err(Error::Invalid(
            "refusing broad non-project directory (e.g. $HOME or system root) as a Git repository"
                .into(),
        ));
    }
    Ok(top)
}

pub(crate) fn normalize_repo_rel_path(raw: &str) -> Result<String> {
    if raw.is_empty() {
        return Err(Error::Invalid("path must not be empty".into()));
    }
    if raw.chars().any(|c| c == '\0' || c == '\n' || c == '\r') {
        return Err(Error::Invalid("path contains a control character".into()));
    }
    if raw.trim_start().starts_with('-') {
        return Err(Error::Invalid(format!(
            "refusing option-like repository path '{raw}'"
        )));
    }
    if raw.starts_with('/') || raw.contains('\\') {
        return Err(Error::Invalid("path must be repository-relative".into()));
    }
    let mut clean = Vec::new();
    for comp in FsPath::new(raw).components() {
        match comp {
            Component::Normal(seg) => {
                let s = seg
                    .to_str()
                    .ok_or_else(|| Error::Invalid("path must be valid UTF-8".into()))?;
                if s.is_empty() || s == "." || s == ".." {
                    return Err(Error::Invalid("path traversal is not allowed".into()));
                }
                clean.push(s);
            }
            _ => return Err(Error::Invalid("path traversal is not allowed".into())),
        }
    }
    if clean.is_empty() {
        return Err(Error::Invalid("path must not be empty".into()));
    }
    Ok(clean.join("/"))
}

fn normalize_rev(rev: Option<&str>) -> Result<String> {
    let r = rev
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("HEAD");
    LocalGit::guard_ref(r)?;
    if r.contains(':') || r.chars().any(char::is_whitespace) {
        return Err(Error::Invalid(format!("invalid git revision '{r}'")));
    }
    Ok(r.to_string())
}

fn is_missing_blob_stderr(stderr: &str) -> bool {
    let low = stderr.to_ascii_lowercase();
    const MARKERS: &[&str] = &[
        "not a valid object name",
        "does not exist in",
        "exists on disk, but not in",
        "invalid object name",
        "bad revision",
        "unknown revision",
        "ambiguous argument",
        "is a tree",
        "bad file",
    ];
    MARKERS.iter().any(|m| low.contains(m))
}

async fn find_staged_rename_orig(repo_dir: &FsPath, rel_path: &str) -> Option<String> {
    let mut cmd = crate::hardened_command();
    cmd.arg("-C")
        .arg(repo_dir)
        .args([
            "diff",
            "--cached",
            "--name-status",
            "-M",
            "-l1000",
            "-z",
            "--diff-filter=R",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd.spawn().ok()?;
    let pg_guard = KillPgOnDrop(child.id().and_then(|pid| i32::try_from(pid).ok()));
    let stdout = child.stdout.take()?;
    let mut buf = Vec::new();
    let ok = tokio::time::timeout(GIT_RENAME_TIMEOUT, async {
        stdout.take(1024 * 1024).read_to_end(&mut buf).await.ok()?;
        let status = child.wait().await.ok()?;
        status.success().then_some(())
    })
    .await
    .ok()
    .flatten();
    let mut pg_guard = pg_guard;
    if ok.is_some() {
        pg_guard.0 = None;
    } else {
        return None;
    }

    let fields: Vec<&[u8]> = buf.split(|&b| b == 0).collect();
    let mut i = 0;
    while i < fields.len() {
        let status = fields[i];
        if status.is_empty() {
            i += 1;
            continue;
        }
        if status.first() == Some(&b'R') || status.first() == Some(&b'C') {
            let orig = fields.get(i + 1).copied()?;
            let dest = fields.get(i + 2).copied()?;
            if dest == rel_path.as_bytes() {
                let orig_str = std::str::from_utf8(orig).ok()?;
                return normalize_repo_rel_path(orig_str).ok();
            }
            i += 3;
        } else {
            i += 2;
        }
    }
    None
}

async fn read_blob_once(
    repo_dir: &FsPath,
    rev: &str,
    rel_path: &str,
) -> Result<Option<RepoFileResp>> {
    let spec = format!("{rev}:{rel_path}");
    let mut cmd = crate::hardened_command();
    // `cat-file blob <rev>:<path>` outputs exact raw blob bytes (identical to
    // `cat-file -p` for blobs) while rejecting directory/tree objects cleanly.
    cmd.arg("-C")
        .arg(repo_dir)
        .args(["cat-file", "blob", &spec])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Internal(format!("spawn git cat-file: {e}")))?;
    let mut pg_guard = KillPgOnDrop(child.id().and_then(|pid| i32::try_from(pid).ok()));
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Internal("missing git cat-file stdout".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Internal("missing git cat-file stderr".into()))?;

    let mut out_buf = Vec::new();
    let mut err_buf = Vec::new();
    let outcome = tokio::time::timeout(GIT_FILE_TIMEOUT, async {
        let read_out = async {
            stdout
                .take((MAX_GIT_FILE_BYTES + 1) as u64)
                .read_to_end(&mut out_buf)
                .await
        };
        let read_err = async { stderr.take(8192).read_to_end(&mut err_buf).await };
        let (ro, re) = tokio::join!(read_out, read_err);
        ro?;
        re?;
        if out_buf.len() > MAX_GIT_FILE_BYTES {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Ok::<Option<std::process::ExitStatus>, std::io::Error>(None);
        }
        let status = child.wait().await?;
        Ok(Some(status))
    })
    .await
    .map_err(|_| Error::Upstream("git cat-file timed out after 3s".into()))?
    .map_err(|e| Error::Internal(format!("read git cat-file output: {e}")))?;

    if let Some(status) = outcome {
        pg_guard.0 = None;
        if !status.success() {
            let err_str = String::from_utf8_lossy(&err_buf);
            if is_missing_blob_stderr(&err_str) {
                return Ok(None);
            }
            return Err(crate::local::upstream_err(&err_str, "", status.code()));
        }
    } else {
        pg_guard.0 = None;
    }

    let truncated = out_buf.len() > MAX_GIT_FILE_BYTES;
    if truncated {
        out_buf.truncate(MAX_GIT_FILE_BYTES);
    }
    let binary = out_buf.contains(&0);
    if binary {
        return Ok(Some(RepoFileResp {
            exists: true,
            content: None,
            binary: true,
            truncated,
        }));
    }
    let slice = if truncated {
        otto_core::text::trim_incomplete_utf8_tail(&out_buf)
    } else {
        out_buf.as_slice()
    };
    let content = match std::str::from_utf8(slice) {
        Ok(s) => s.to_owned(),
        Err(_) => String::from_utf8_lossy(slice).into_owned(),
    };
    Ok(Some(RepoFileResp {
        exists: true,
        content: Some(content),
        binary: false,
        truncated,
    }))
}

/// `GET /repos/{id}/file?path=<repo_rel_path>&rev=<optional, default HEAD>`
pub(crate) async fn repo_file<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<RepoFileQuery>,
) -> ApiResult<Json<RepoFileResp>> {
    let (repo, _git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    let rel_path = normalize_repo_rel_path(&q.path)?;
    let rev = normalize_rev(q.rev.as_deref())?;
    let repo_dir = FsPath::new(&repo.path);

    if let Some(resp) = read_blob_once(repo_dir, &rev, &rel_path).await? {
        return Ok(Json(resp));
    }
    if rev == "HEAD" {
        if let Some(orig_rel) = find_staged_rename_orig(repo_dir, &rel_path).await {
            if let Some(resp) = read_blob_once(repo_dir, "HEAD", &orig_rel).await? {
                return Ok(Json(resp));
            }
        }
    }
    Ok(Json(RepoFileResp::missing()))
}

#[cfg(test)]
mod person_call_tests {
    use otto_core::auth::{
        carry_request_credential, request_is_person, with_request_credential, RequestCredential,
        OUTSIDE_REQUEST,
    };

    /// S11-304: only a person's own credential makes a git route call
    /// `person_initiated` (in-work-tree hooks run); an agent session's
    /// credential, no request scope at all, and a task spawned off a person's
    /// request without carrying the credential do not.
    #[tokio::test]
    async fn only_a_person_s_credential_runs_in_worktree_hooks() {
        let person = RequestCredential::default();
        let agent = RequestCredential {
            agent: true,
            root_withheld: true,
        };
        assert!(with_request_credential(person, async { request_is_person() }).await);
        assert!(!with_request_credential(agent, async { request_is_person() }).await);
        assert!(!with_request_credential(OUTSIDE_REQUEST, async { request_is_person() }).await);
        assert!(!request_is_person());
        let (spawned, carried) = with_request_credential(person, async {
            let spawned = tokio::spawn(async { request_is_person() }).await.unwrap();
            let carried = tokio::spawn(carry_request_credential(async { request_is_person() }))
                .await
                .unwrap();
            (spawned, carried)
        })
        .await;
        assert!(!spawned && carried);
    }
}

#[cfg(test)]
#[path = "round5_tests.rs"]
mod round5_tests;
