//! New capability endpoints that back three of the outward `otto.*` MCP tools.
//! All live under `/workspaces/{wid}/mcp/...` (Feature::Mcp). They are written to
//! be injection-safe (design §14 F11): `code-search` is **pure Rust** (no
//! subprocess, so no flag injection) and confines `path` to the workspace root;
//! `proof-pack` shells `git` only with a FIXED argv and a validated ref.

use std::path::{Path, PathBuf};

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
const SKIP_DIRS: &[&str] = &[".git", "node_modules", "target", "dist", "build", ".svn", ".hg", "vendor", ".venv", "__pycache__"];

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
pub async fn code_search(
    AxPath(wid): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<CodeSearchQuery>,
) -> ApiResult<Json<Value>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let needle = q.q.trim();
    if needle.is_empty() {
        return Err(ApiError(Error::Invalid("q must not be empty".into())));
    }
    let ws = ctx.workspaces.get(&wid).await?;
    let root = std::fs::canonicalize(&ws.root_path)
        .map_err(|e| ApiError(Error::Invalid(format!("workspace root unavailable: {e}"))))?;

    // Resolve and CONFINE the optional sub-path to the root (reject traversal).
    let search_root = match q.path.as_deref().filter(|p| !p.trim().is_empty()) {
        Some(rel) => {
            if Path::new(rel).is_absolute() || rel.contains("..") {
                return Err(ApiError(Error::Invalid(
                    "path must be relative and within the workspace (no '..' / absolute)".into(),
                )));
            }
            let joined = root.join(rel);
            let canon = std::fs::canonicalize(&joined)
                .map_err(|_| ApiError(Error::NotFound("path not found".into())))?;
            if !canon.starts_with(&root) {
                return Err(ApiError(Error::Forbidden("path escapes the workspace root".into())));
            }
            canon
        }
        None => root.clone(),
    };

    let max = q.max.unwrap_or(100).min(MAX_RESULTS_CAP);
    let needle_lower = needle.to_lowercase();
    let mut results: Vec<Value> = Vec::new();
    let mut walked = 0usize;
    let mut stack = vec![search_root.clone()];
    while let Some(dir) = stack.pop() {
        if results.len() >= max || walked >= MAX_WALK {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            walked += 1;
            if walked >= MAX_WALK || results.len() >= max {
                break;
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
            let Ok(content) = std::fs::read(&p) else { continue };
            // Skip binary (NUL in the first chunk).
            if content.iter().take(1024).any(|&b| b == 0) {
                continue;
            }
            let text = String::from_utf8_lossy(&content);
            let rel = p.strip_prefix(&root).unwrap_or(&p).to_string_lossy().to_string();
            for (i, line) in text.lines().enumerate() {
                if line.to_lowercase().contains(&needle_lower) {
                    let snippet: String = line.trim().chars().take(240).collect();
                    results.push(json!({
                        "file": rel,
                        "line": i + 1,
                        "text": redact_text(&snippet).value,
                    }));
                    if results.len() >= max {
                        break;
                    }
                }
            }
        }
    }
    Ok(Json(json!({
        "query": needle,
        "root": ws.root_path,
        "matches": results,
        "truncated": results.len() >= max,
    })))
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
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | '.'))
        && !s.contains("..")
}

fn safe_git(path: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).chars().take(8000).collect())
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
        let path = PathBuf::from(&repo.path);
        let branch = q.branch.as_deref().unwrap_or("HEAD");
        if !valid_ref(branch) {
            return Err(ApiError(Error::Invalid("invalid branch/ref".into())));
        }
        let commits = safe_git(&path, &["log", "-n", "20", "--pretty=format:%h %an %ad %s", "--date=short", branch]);
        let status = safe_git(&path, &["status", "--porcelain"]);
        let diffstat = safe_git(&path, &["diff", "--stat", "HEAD"]);
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
        if let Ok(gl) = ctx.goal_loops_repo.get(&loop_id.to_string()).await {
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
