//! New capability endpoints that back three of the outward `otto.*` MCP tools.
//! All live under `/workspaces/{wid}/mcp/...` (Feature::Mcp). They are written to
//! be injection-safe (design §14 F11): `code-search` is **pure Rust** (no
//! subprocess, so no flag injection) and confines `path` to the workspace root;
//! `proof-pack` shells `git` only with a FIXED argv and a validated ref.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path as AxPath, Query, State};
use axum::Json;
use otto_core::domain::WorkspaceRole;
use otto_core::redact::redact_text;
use otto_core::{Error, Id};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

const MAX_FILE_BYTES: u64 = 256 * 1024;
const MAX_RESULTS_CAP: usize = 500;
const MAX_WALK: usize = 20_000;
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".svn",
    ".hg",
    "vendor",
    ".venv",
    "__pycache__",
];

#[derive(Deserialize)]
pub struct CodeSearchQuery {
    pub q: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub max: Option<usize>,
}

/// `GET /workspaces/{wid}/mcp/code-search?q=&path=&max=` — pure-Rust literal
/// search confined to the workspace root. `q` is never parsed as flags; `path` is
/// canonicalized and rejected if it escapes the root (no traversal / absolute).
///
/// The walk runs on the blocking pool (r3-06-01: it used to read and lowercase
/// up to 20 000 files on a tokio worker, seconds at worst, and nothing could
/// stop it). It stops early — `truncated: true` plus a `stopped` reason — at
/// [`MAX_WALK`] entries, [`MAX_TOTAL_BYTES`] read or [`SEARCH_BUDGET`] of wall
/// time, and as soon as the request is dropped (client gone, MCP timeout).
pub async fn code_search(
    AxPath(wid): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<CodeSearchQuery>,
) -> ApiResult<Json<Value>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let needle = q.q.trim().to_string();
    if needle.is_empty() {
        return Err(ApiError(Error::Invalid("q must not be empty".into())));
    }
    let ws = ctx.workspaces.get(&wid).await?;
    let max = q.max.unwrap_or(100).min(MAX_RESULTS_CAP);
    let cancel = Arc::new(AtomicBool::new(false));
    // Dropped with the request future (and on normal return, when the walk is
    // already done): the walk checks the flag between files.
    let _cancel_on_drop = CancelOnDrop(Arc::clone(&cancel));
    let root_path = ws.root_path.clone();
    let rel = q.path.clone();
    let needle_lower = needle.to_lowercase();
    let outcome = crate::offload::blocking(move || -> Result<SearchOutcome, Error> {
        let (root, search_root) = confine(&root_path, rel.as_deref())?;
        let limits = SearchLimits {
            max,
            max_walk: MAX_WALK,
            max_bytes: MAX_TOTAL_BYTES,
            deadline: Instant::now() + SEARCH_BUDGET,
        };
        Ok(search_tree(
            &root,
            &search_root,
            &needle_lower,
            &limits,
            &cancel,
        ))
    })
    .await
    .map_err(ApiError)?;
    let truncated = outcome.matches.len() >= max || outcome.stopped.is_some();
    Ok(Json(json!({
        "query": needle,
        "root": ws.root_path,
        "matches": outcome.matches,
        "truncated": truncated,
        "stopped": outcome.stopped,
    })))
}

/// Total file bytes one search may read before it stops.
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
/// Wall-clock budget of one search.
const SEARCH_BUDGET: Duration = Duration::from_secs(10);

/// Sets the flag when dropped — i.e. when the request future goes away.
struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Canonical workspace root and the (confined) directory to search. Blocking.
fn confine(root_path: &str, rel: Option<&str>) -> Result<(PathBuf, PathBuf), Error> {
    let root = std::fs::canonicalize(root_path)
        .map_err(|e| Error::Invalid(format!("workspace root unavailable: {e}")))?;
    // Resolve and CONFINE the optional sub-path to the root (reject traversal).
    let search_root = match rel.filter(|p| !p.trim().is_empty()) {
        Some(rel) => {
            if Path::new(rel).is_absolute() || rel.contains("..") {
                return Err(Error::Invalid(
                    "path must be relative and within the workspace (no '..' / absolute)".into(),
                ));
            }
            let canon = std::fs::canonicalize(root.join(rel))
                .map_err(|_| Error::NotFound("path not found".into()))?;
            if !canon.starts_with(&root) {
                return Err(Error::Forbidden("path escapes the workspace root".into()));
            }
            canon
        }
        None => root.clone(),
    };
    Ok((root, search_root))
}

struct SearchLimits {
    max: usize,
    max_walk: usize,
    max_bytes: u64,
    deadline: Instant,
}

struct SearchOutcome {
    matches: Vec<Value>,
    /// Why the walk stopped before covering the tree (`None` = it finished,
    /// or stopped because `max` matches were found).
    stopped: Option<&'static str>,
}

/// The walk itself. Blocking; checks `cancel` and the budgets between files.
/// Each file is lowercased ONCE and skipped whole unless it contains the
/// needle — the old loop lowercased (and allocated) every line of every file.
#[allow(clippy::disallowed_methods)] // sync helper: code_search runs it via offload::blocking
fn search_tree(
    root: &Path,
    search_root: &Path,
    needle_lower: &str,
    limits: &SearchLimits,
    cancel: &AtomicBool,
) -> SearchOutcome {
    let mut matches: Vec<Value> = Vec::new();
    let mut walked = 0usize;
    let mut bytes = 0u64;
    let mut stack = vec![search_root.to_path_buf()];
    let stop = |walked: usize, bytes: u64| -> Option<&'static str> {
        if cancel.load(Ordering::Relaxed) {
            Some("cancelled")
        } else if walked >= limits.max_walk {
            Some("walk_limit")
        } else if bytes >= limits.max_bytes {
            Some("byte_budget")
        } else if Instant::now() >= limits.deadline {
            Some("time_budget")
        } else {
            None
        }
    };
    while let Some(dir) = stack.pop() {
        if matches.len() >= limits.max {
            break;
        }
        if let Some(why) = stop(walked, bytes) {
            return SearchOutcome {
                matches,
                stopped: Some(why),
            };
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            walked += 1;
            if matches.len() >= limits.max {
                break;
            }
            if let Some(why) = stop(walked, bytes) {
                return SearchOutcome {
                    matches,
                    stopped: Some(why),
                };
            }
            let p = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if SKIP_DIRS.contains(&name.as_ref()) || name.starts_with('.') {
                    continue;
                }
                stack.push(p);
                continue;
            }
            if !ft.is_file() {
                continue;
            }
            let meta = entry.metadata().ok();
            if meta.map(|m| m.len() > MAX_FILE_BYTES).unwrap_or(true) {
                continue;
            }
            let Ok(content) = std::fs::read(&p) else {
                continue;
            };
            bytes += content.len() as u64;
            // Skip binary (NUL in the first chunk).
            if content.iter().take(1024).any(|&b| b == 0) {
                continue;
            }
            let text = String::from_utf8_lossy(&content);
            let lower = text.to_lowercase();
            if !lower.contains(needle_lower) {
                continue;
            }
            let rel = p
                .strip_prefix(root)
                .unwrap_or(&p)
                .to_string_lossy()
                .to_string();
            // Lowercasing never adds or removes a newline, so the two line
            // iterators stay aligned.
            for (i, (line, low)) in text.lines().zip(lower.lines()).enumerate() {
                if low.contains(needle_lower) {
                    let snippet: String = line.trim().chars().take(240).collect();
                    matches.push(json!({
                        "file": rel,
                        "line": i + 1,
                        "text": redact_text(&snippet).value,
                    }));
                    if matches.len() >= limits.max {
                        break;
                    }
                }
            }
        }
    }
    SearchOutcome {
        matches,
        stopped: None,
    }
}

#[derive(Deserialize)]
pub struct ContextPacketReq {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub story_id: Option<String>,
    #[serde(default)]
    pub max_excerpts: Option<usize>,
}

/// `POST /workspaces/{wid}/mcp/context-packet` — assemble a code-grounded context
/// packet: workspace metadata + (when a `query` is given) the most relevant code
/// excerpts. Reuses the same confined, injection-safe search as `code-search`.
pub async fn context_packet(
    AxPath(wid): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<ContextPacketReq>,
) -> ApiResult<Json<Value>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let ws = ctx.workspaces.get(&wid).await?;
    let mut excerpts: Vec<Value> = Vec::new();
    if let Some(query) = req.query.as_deref().filter(|q| !q.trim().is_empty()) {
        let inner = code_search(
            AxPath(wid.clone()),
            State(ctx.clone()),
            CurrentUser(user.clone()),
            Query(CodeSearchQuery {
                q: query.to_string(),
                path: None,
                max: Some(req.max_excerpts.unwrap_or(20).min(50)),
            }),
        )
        .await?;
        if let Some(m) = inner.0.get("matches").and_then(Value::as_array) {
            excerpts = m.clone();
        }
    }
    Ok(Json(json!({
        "workspace": { "id": ws.id, "name": ws.name, "root_path": ws.root_path },
        "query": req.query,
        "story_id": req.story_id,
        "code_excerpts": excerpts,
        "assembled_by": "otto.get_context_packet",
    })))
}

#[derive(Deserialize)]
pub struct ProofPackQuery {
    #[serde(default)]
    pub repo_id: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub goal_loop_id: Option<String>,
}

fn valid_ref(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | '.'))
        && !s.contains("..")
}

/// Fixed-argv git on tokio's process driver — `status` on a large repo can take
/// seconds, which must not park a runtime worker.
async fn safe_git(path: &Path, args: &[&str]) -> Option<String> {
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&out.stdout)
            .chars()
            .take(8000)
            .collect(),
    )
}

/// `GET /workspaces/{wid}/mcp/proof-pack?repo_id=&branch=&goal_loop_id=` — a
/// redacted evidence bundle: git status/recent-commits/diffstat for a repo (safe
/// fixed-argv git, validated ref) and a goal loop's machine-checked acceptance
/// criteria. This is the "proof pack" Otto can hand back to prove a claim of done.
pub async fn proof_pack(
    AxPath(wid): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<ProofPackQuery>,
) -> ApiResult<Json<Value>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let mut pack = json!({ "assembled_by": "otto.get_proof_pack" });

    if let Some(repo_id) = q.repo_id.as_deref().filter(|r| !r.is_empty()) {
        let repo = ctx.git_store.get_repo(&repo_id.to_string()).await?;
        // The Viewer check above is for `{wid}` only — a repo registered in
        // another workspace must not be readable through it.
        if repo.workspace_id != wid {
            return Err(ApiError(Error::NotFound(format!(
                "repo {repo_id} is not in workspace {wid}"
            ))));
        }
        let path = PathBuf::from(&repo.path);
        let branch = q.branch.as_deref().unwrap_or("HEAD");
        if !valid_ref(branch) {
            return Err(ApiError(Error::Invalid("invalid branch/ref".into())));
        }
        // Independent reads: run the three concurrently.
        let log_args = [
            "log",
            "-n",
            "20",
            "--pretty=format:%h %an %ad %s",
            "--date=short",
            branch,
        ];
        let (commits, status, diffstat) = tokio::join!(
            safe_git(&path, &log_args),
            safe_git(&path, &["status", "--porcelain"]),
            safe_git(&path, &["diff", "--stat", "HEAD"]),
        );
        pack["repo"] = json!({
            "id": repo.id,
            "name": repo.name,
            "branch": branch,
            "recent_commits": commits.map(|c| redact_text(&c).value),
            "working_tree_status": status.map(|s| redact_text(&s).value),
            "uncommitted_diffstat": diffstat.map(|d| redact_text(&d).value),
        });
    }

    if let Some(loop_id) = q.goal_loop_id.as_deref().filter(|r| !r.is_empty()) {
        let gl = ctx
            .goal_loops_repo
            .get(&loop_id.to_string())
            .await
            .ok()
            // Same workspace confinement as the repo above.
            .filter(|gl| gl.workspace_id == wid);
        if let Some(gl) = gl {
            // Machine-checked acceptance criteria + status = the strongest evidence.
            pack["goal_loop"] = json!({
                "id": gl.id,
                "name": gl.name,
                "status": gl.status,
                "acceptance_criteria": gl.definition.acceptance_criteria,
            });
        }
    }

    Ok(Json(pack))
}

#[cfg(test)]
mod search_tests {
    use super::*;

    fn limits(max: usize) -> SearchLimits {
        SearchLimits {
            max,
            max_walk: MAX_WALK,
            max_bytes: MAX_TOTAL_BYTES,
            deadline: Instant::now() + Duration::from_secs(30),
        }
    }

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
        std::fs::write(
            root.join("src/a.rs"),
            "fn Alpha() {}\nlet beta = 1;\nALPHA again\n",
        )
        .unwrap();
        std::fs::write(root.join("src/deep/b.rs"), "nothing here\nalpha\r\n").unwrap();
        std::fs::write(root.join("node_modules/x/c.js"), "alpha in a skipped dir").unwrap();
        std::fs::write(root.join("bin.dat"), b"alpha\0binary").unwrap();
        dir
    }

    #[test]
    fn finds_case_insensitive_matches_with_line_numbers() {
        let dir = tree();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let out = search_tree(&root, &root, "alpha", &limits(100), &AtomicBool::new(false));
        assert!(out.stopped.is_none());
        let mut hits: Vec<(String, u64, String)> = out
            .matches
            .iter()
            .map(|m| {
                (
                    m["file"].as_str().unwrap().to_string(),
                    m["line"].as_u64().unwrap(),
                    m["text"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        hits.sort();
        assert_eq!(
            hits,
            vec![
                ("src/a.rs".into(), 1, "fn Alpha() {}".into()),
                ("src/a.rs".into(), 3, "ALPHA again".into()),
                ("src/deep/b.rs".into(), 2, "alpha".into()),
            ]
        );
    }

    #[test]
    fn stops_on_cancel_and_on_budgets() {
        let dir = tree();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        // Cancelled before it starts: nothing is read.
        let out = search_tree(&root, &root, "alpha", &limits(100), &AtomicBool::new(true));
        assert_eq!(out.stopped, Some("cancelled"));
        assert!(out.matches.is_empty());
        // Byte budget.
        let tight = SearchLimits {
            max_bytes: 1,
            ..limits(100)
        };
        let out = search_tree(&root, &root, "alpha", &tight, &AtomicBool::new(false));
        assert_eq!(out.stopped, Some("byte_budget"));
        // Time budget.
        let late = SearchLimits {
            deadline: Instant::now(),
            ..limits(100)
        };
        let out = search_tree(&root, &root, "alpha", &late, &AtomicBool::new(false));
        assert_eq!(out.stopped, Some("time_budget"));
        // Walk limit.
        let short = SearchLimits {
            max_walk: 1,
            ..limits(100)
        };
        let out = search_tree(&root, &root, "alpha", &short, &AtomicBool::new(false));
        assert_eq!(out.stopped, Some("walk_limit"));
        // `max` matches is a normal finish, not a budget stop.
        let out = search_tree(&root, &root, "alpha", &limits(1), &AtomicBool::new(false));
        assert_eq!(out.matches.len(), 1);
        assert!(out.stopped.is_none());
    }

    #[test]
    fn confine_rejects_escapes() {
        let dir = tree();
        let root = dir.path().to_str().unwrap();
        assert!(confine(root, Some("../etc")).is_err());
        assert!(confine(root, Some("/etc")).is_err());
        assert!(matches!(
            confine(root, Some("nope")),
            Err(Error::NotFound(_))
        ));
        let (r, s) = confine(root, Some("src")).unwrap();
        assert!(s.starts_with(&r) && s.ends_with("src"));
    }

    /// The drop guard is what cancels a walk when the request goes away.
    #[test]
    fn cancel_guard_sets_the_flag_on_drop() {
        let flag = Arc::new(AtomicBool::new(false));
        let guard = CancelOnDrop(Arc::clone(&flag));
        assert!(!flag.load(Ordering::Relaxed));
        drop(guard);
        assert!(flag.load(Ordering::Relaxed));
    }
}
