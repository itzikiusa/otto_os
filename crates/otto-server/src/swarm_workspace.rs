//! Per-agent working directory + context for a swarm turn.
//!
//! Each agent gets a UNIQUE cwd (required for Codex token attribution and to keep
//! per-agent materialized context — `.claude/skills`, `CLAUDE.md`/`AGENTS.md` —
//! from clobbering siblings):
//! - `worktree`: a linked git worktree of the project repo on `swarm/<s>/<a>`.
//! - `scratch` : `<data_dir>/swarm/<swarm>/<agent>/work` (non-code roles).
//!
//! Then it materializes the agent's skills + soul + identity into that cwd via
//! `otto_context::materialize::provision` (reused as-is) and installs the
//! `otto-post` board helper so the agent can post to the shared surface live.

use std::path::PathBuf;

use otto_core::api::WorkspaceContextConfig;
use otto_core::Result;
use otto_state::{Swarm, SwarmAgent, SwarmProject, SwarmTask};

use crate::state::ServerCtx;

/// The board-posting helper materialized into every agent cwd. Uses the
/// per-session ingest token (same gate as `/ingest/claude`).
const OTTO_POST: &str = r#"#!/bin/sh
# otto-post — share a message on your swarm's shared board (visible to the team
# and the user). Usage: otto-post [--to AGENT_ID] [--kind KIND] "message body"
# kinds: message | idea | review_request | review | decision | status | concern | handoff
BASE="${OTTO_INGEST_BASE:-http://127.0.0.1:7700}"
TO=""; KIND="message"; BODY=""
while [ $# -gt 0 ]; do
  case "$1" in
    --to) TO="$2"; shift 2 ;;
    --kind) KIND="$2"; shift 2 ;;
    --) shift; BODY="$*"; break ;;
    *) if [ -z "$BODY" ]; then BODY="$1"; else BODY="$BODY $1"; fi; shift ;;
  esac
done
if [ -z "$BODY" ]; then echo "usage: otto-post [--to AGENT] [--kind KIND] \"message\"" >&2; exit 1; fi
PAYLOAD=$(KIND="$KIND" TO="$TO" BODY="$BODY" python3 - <<'PY'
import json, os
to = os.environ.get("TO") or None
print(json.dumps({"kind": os.environ.get("KIND", "message"), "to_agent_id": to, "body": os.environ.get("BODY", "")}))
PY
)
curl -s -X POST "$BASE/api/v1/ingest/swarm/board" \
  -H "Content-Type: application/json" \
  -H "X-Otto-Session: $OTTO_SESSION_ID" \
  -H "X-Otto-Token: $OTTO_INGEST_TOKEN" \
  -d "$PAYLOAD" >/dev/null 2>&1
"#;

fn swarm_base(ctx: &ServerCtx, swarm_id: &str, agent_id: &str) -> PathBuf {
    ctx.data_dir
        .join("swarm")
        .join(swarm_id)
        .join(agent_id)
}

fn cwd_mode(swarm: &Swarm, agent: &SwarmAgent, has_repo: bool) -> String {
    // 1. An explicit PER-AGENT choice always wins: "repo" opts a *single* agent
    //    out (a deliberately single-agent project), "scratch" for non-code roles,
    //    "worktree" to force isolation.
    if let Some(m) = agent.cwd_mode.clone().filter(|s| !s.trim().is_empty()) {
        return m;
    }
    // 2. A project repo path means agents work in CODE. Isolate each in its own
    //    git worktree so several agents can share the repo without clobbering each
    //    other's index/working tree (the requirement). Shared "repo" mode is now
    //    opt-in per agent only — there is intentionally NO global disable switch
    //    that would re-introduce the clobbering. (Worktree-default also stops
    //    `provision_agent` from writing CLAUDE.md/.claude into the user's real repo.)
    if has_repo {
        return "worktree".to_string();
    }
    // 3. No repo path: fall back to the swarm-level config, else scratch.
    swarm
        .config
        .get("cwd_mode")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| "scratch".to_string())
}

/// Short, filesystem/branch-safe slice of an id.
fn short(id: &str) -> &str {
    let n = id.len().min(8);
    &id[id.len() - n..]
}

/// Where an agent turn runs, and (for worktrees) enough to notify the feed + merge.
#[derive(Debug, Clone)]
pub struct CwdInfo {
    pub path: String,
    pub mode: String,
    /// True iff a NEW git worktree was created on this call (vs reused/scratch).
    pub created: bool,
    /// The agent's worktree branch (worktree mode only).
    pub branch: Option<String>,
    /// The pinned integration branch the worktree is based on + merges into.
    pub integration_branch: Option<String>,
}

/// The integration branch for a project: a DEDICATED swarm branch (never the
/// user's working branch) that all of a project's agent worktrees are based on
/// and merged into. Read from the project if already pinned, else computed.
pub fn integration_branch_name(swarm: &Swarm, project: &SwarmProject) -> String {
    project
        .integration_branch
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("swarm/{}/{}/int", short(&swarm.id), short(&project.id)))
}

/// Path of the dedicated integration worktree (checked out on `integration_branch`).
/// All merges happen here so Otto never touches the user's primary checkout.
pub fn integration_worktree_path(ctx: &ServerCtx, swarm_id: &str, project_id: &str) -> std::path::PathBuf {
    ctx.data_dir
        .join("swarm")
        .join(swarm_id)
        .join(project_id)
        .join("_integration")
        .join("wt")
}

/// Ensure the dedicated integration branch + its worktree exist (idempotent), and
/// persist the branch on the project the first time. Returns (worktree_path, branch).
/// The branch is pinned from the repo's HEAD on first creation — the single base
/// every agent worktree branches from, so per-task merges compose.
pub async fn ensure_integration_worktree(
    ctx: &ServerCtx,
    swarm: &Swarm,
    project: &SwarmProject,
) -> Result<(String, String)> {
    // Expand tilde-form paths (older projects persisted them raw) — a literal
    // `~` never resolves as a git dir.
    let repo_path = project
        .repo_path
        .as_deref()
        .map(otto_core::paths::expand_tilde)
        .ok_or_else(|| otto_core::Error::Invalid("project has no repo_path".into()))?;
    let branch = integration_branch_name(swarm, project);
    let wt = integration_worktree_path(ctx, &swarm.id, &project.id);
    let wt_str = wt.to_string_lossy().to_string();
    let git = otto_git::LocalGit::new(&repo_path);
    let base = git
        .current_branch()
        .await
        .unwrap_or_else(|_| "HEAD".to_string());
    // Creates the integration branch from the repo's current HEAD on first call;
    // reuses it (with accumulated merges) afterwards.
    git.worktree_add_if_absent(&wt_str, &branch, &base).await?;
    // Persist the pin so subsequent turns + the merge step agree on the target.
    if project
        .integration_branch
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        let _ = ctx
            .swarm_repo
            .update_project(
                &project.id,
                otto_state::ProjectPatch {
                    integration_branch: Some(Some(branch.clone())),
                    ..Default::default()
                },
            )
            .await;
    }
    Ok((wt_str, branch))
}

/// Ensure the agent has a prepared, unique working directory. Returns its path.
/// Thin wrapper over [`ensure_cwd_info`] for callers that only need the path.
pub async fn ensure_cwd(
    ctx: &ServerCtx,
    swarm: &Swarm,
    agent: &SwarmAgent,
    project: Option<&SwarmProject>,
) -> Result<String> {
    Ok(ensure_cwd_info(ctx, swarm, agent, project).await?.path)
}

/// Ensure the agent's working directory and report how it was provisioned.
/// In worktree mode the agent's branch (`swarm/<s>/<a>`) is based on the project's
/// pinned integration branch so per-task work merges back cleanly.
pub async fn ensure_cwd_info(
    ctx: &ServerCtx,
    swarm: &Swarm,
    agent: &SwarmAgent,
    project: Option<&SwarmProject>,
) -> Result<CwdInfo> {
    // Expand tilde-form repo paths (older projects persisted them raw): a
    // literal `~` cwd makes the session spawn fall back to $HOME and breaks
    // transcript polling, which derives its directory from this string.
    let repo = project
        .and_then(|p| p.repo_path.as_deref())
        .map(otto_core::paths::expand_tilde);
    let mode = cwd_mode(swarm, agent, repo.is_some());

    if mode == "repo" {
        if let Some(r) = &repo {
            return Ok(CwdInfo { path: r.clone(), mode, created: false, branch: None, integration_branch: None });
        }
    }

    if mode == "worktree" {
        if let (Some(repo_path), Some(project)) = (&repo, project) {
            // Pin (and create) the integration branch first — it's the base.
            let integration_branch = match ensure_integration_worktree(ctx, swarm, project).await {
                Ok((_, branch)) => branch,
                Err(e) => {
                    tracing::warn!("swarm: integration worktree failed ({e}); scratch fallback");
                    return Ok(scratch_info(ctx, swarm, agent));
                }
            };
            let wt = swarm_base(ctx, &swarm.id, &agent.id).join("wt");
            let wt_str = wt.to_string_lossy().to_string();
            let branch = format!("swarm/{}/{}", short(&swarm.id), short(&agent.id));
            let git = otto_git::LocalGit::new(repo_path);
            // Base the agent worktree on the pinned integration branch — but only
            // on first creation. `worktree_add_if_absent` reuses an existing tree
            // as-is, so a resumed turn keeps the agent's prior commits.
            match git.worktree_add_if_absent(&wt_str, &branch, &integration_branch).await {
                Ok(created) => {
                    if created {
                        tracing::info!("swarm: created worktree {} on {} (base {})", wt_str, branch, integration_branch);
                    }
                    return Ok(CwdInfo {
                        path: wt_str,
                        mode,
                        created,
                        branch: Some(branch),
                        integration_branch: Some(integration_branch),
                    });
                }
                Err(e) => {
                    tracing::warn!("swarm: worktree add failed ({}), falling back to scratch: {e}", wt_str);
                }
            }
        }
    }

    // scratch (default, and the fallback).
    Ok(scratch_info(ctx, swarm, agent))
}

fn scratch_info(ctx: &ServerCtx, swarm: &Swarm, agent: &SwarmAgent) -> CwdInfo {
    let scratch = swarm_base(ctx, &swarm.id, &agent.id).join("work");
    let _ = std::fs::create_dir_all(&scratch);
    CwdInfo {
        path: scratch.to_string_lossy().to_string(),
        mode: "scratch".to_string(),
        created: false,
        branch: None,
        integration_branch: None,
    }
}

/// Effective skill set for a swarm agent: the UNION of team (swarm `config.skills`),
/// project (`project.skills`) and per-agent (`agent.skills`) skills, deduped by name
/// (first-seen order). Each layer is `[{name, must_use?}]` or `["name", …]`. This is
/// how a team/project adds skills on top of an agent's defaults (requirement 2).
pub fn resolve_skills(swarm: &Swarm, project: Option<&SwarmProject>, agent: &SwarmAgent) -> Vec<String> {
    use std::collections::HashSet;
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    let mut add = |v: Option<&serde_json::Value>| {
        if let Some(arr) = v.and_then(|v| v.as_array()) {
            for s in arr {
                let name = s
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| s.get("name").and_then(|v| v.as_str()).map(str::to_string));
                if let Some(name) = name {
                    if !name.is_empty() && seen.insert(name.clone()) {
                        out.push(name);
                    }
                }
            }
        }
    };
    add(swarm.config.get("skills"));
    add(project.map(|p| &p.skills));
    add(Some(&agent.skills));
    out
}

/// Render the agent's identity markdown (role, soul, scope, org position, the
/// current project/task, and how to use the shared board). Lands in CLAUDE.md /
/// AGENTS.md via `provision`.
pub fn render_identity(
    swarm: &Swarm,
    agent: &SwarmAgent,
    manager_title: Option<&str>,
    reports: &[String],
    project: Option<&SwarmProject>,
    task: Option<&SwarmTask>,
) -> String {
    let mut s = String::new();
    s.push_str(&format!("# You are {} — {}\n\n", agent.name, agent.title));
    s.push_str(&format!(
        "You are a member of the **{}** agent swarm. Mission: {}\n\n",
        swarm.name,
        if swarm.description.is_empty() { "(not specified)" } else { &swarm.description }
    ));
    if !agent.specialization.is_empty() {
        s.push_str(&format!("**Specialization:** {}\n\n", agent.specialization));
    }
    if let Some(soul) = &agent.soul_md {
        if !soul.is_empty() {
            s.push_str(&format!("## Who you are\n{soul}\n\n"));
        }
    }
    if !agent.scope_md.is_empty() {
        s.push_str(&format!("## Your scope\n{}\n\n", agent.scope_md));
    }
    s.push_str("## Org\n");
    if let Some(m) = manager_title {
        s.push_str(&format!("- You report to: {m}\n"));
    } else {
        s.push_str("- You are at the top of the org.\n");
    }
    if !reports.is_empty() {
        s.push_str(&format!("- Your reports: {}\n", reports.join(", ")));
    }
    s.push('\n');
    if let Some(p) = project {
        s.push_str(&format!("## Project: {}\n{}\n", p.name, p.description));
        if let Some(goal) = &p.goal_md {
            if !goal.is_empty() {
                s.push_str(&format!("\nGoal:\n{goal}\n"));
            }
        }
        s.push('\n');
    }
    if let Some(t) = task {
        s.push_str(&format!("## Your current task: {}\n{}\n\n", t.title, t.description));
    }
    s.push_str(
        "## Working with your team (shared board)\n\
         You have an `otto-post` command in your working directory. Use it to share \
         with the team and the user — it is the team's shared surface:\n\
         - `./otto-post --kind idea \"...\"` to float an idea\n\
         - `./otto-post --kind review_request --to <agent-id> \"...\"` to ask for a review\n\
         - `./otto-post --kind review \"...\"` to post a review of another's work\n\
         - `./otto-post --kind decision \"...\"` to record a decision\n\
         - `./otto-post --kind concern \"...\"` if you think the plan/timeline is wrong\n\
         Post brief, high-signal updates as you work; the user is watching.\n\n\
         When you finish, also write your structured result to the file path given in \
         your task brief (that is how your work is collected).\n",
    );
    s
}

/// `otto-product` — a PO/feature-design agent publishes a feature DRAFT to the
/// Product page, filed under the swarm project's epic (`--folder` groups it,
/// `--kind doc|story` picks the tree role; the same title updates in place).
/// Mirrors `otto-post`'s per-session auth. Flags live INSIDE the `case` block —
/// the `*)` catch-all appends to the body, so an unknown flag never silently
/// becomes markdown. Failures are loud: the HTTP status on stderr and a
/// non-zero exit — an agent sees why nothing landed.
const OTTO_PRODUCT: &str = r#"#!/bin/sh
# otto-product — publish a feature DRAFT to the Product page for the user/PO to
# review. It lands under this project's epic on the Product page.
# Usage: otto-product --title "Feature title" [--folder Design] [--kind doc|story] "draft markdown body"
BASE="${OTTO_INGEST_BASE:-http://127.0.0.1:7700}"
TITLE=""; FOLDER=""; KIND=""; BODY=""
while [ $# -gt 0 ]; do
  case "$1" in
    --title) TITLE="$2"; shift 2 ;;
    --folder) FOLDER="$2"; shift 2 ;;
    --kind) KIND="$2"; shift 2 ;;
    --) shift; BODY="$*"; break ;;
    --*) # An unknown flag ONLY while no body has started and the arg is a single
         # token; a body that begins with `---` front-matter or an hrule (it has
         # whitespace/newlines) is body, never a flag.
         if [ -z "$BODY" ] && [ "$1" = "${1%%[[:space:]]*}" ]; then
           echo "otto-product: unknown flag $1 (known: --title --folder --kind)" >&2; exit 2
         fi
         if [ -z "$BODY" ]; then BODY="$1"; else BODY="$BODY $1"; fi; shift ;;
    *) if [ -z "$BODY" ]; then BODY="$1"; else BODY="$BODY $1"; fi; shift ;;
  esac
done
if [ -z "$BODY" ]; then echo "usage: otto-product --title \"Title\" [--folder F] [--kind doc|story] \"markdown body\"" >&2; exit 1; fi
PAYLOAD=$(TITLE="$TITLE" FOLDER="$FOLDER" KIND="$KIND" BODY="$BODY" python3 - <<'PY'
import json, os
d = {"title": os.environ.get("TITLE",""), "body_md": os.environ.get("BODY","")}
if os.environ.get("FOLDER"): d["folder"] = os.environ["FOLDER"]
if os.environ.get("KIND"): d["tree_kind"] = os.environ["KIND"]
print(json.dumps(d))
PY
)
STATUS=$(curl -sS -o /dev/null -w '%{http_code}' -X POST "$BASE/api/v1/ingest/swarm/product" \
  -H "Content-Type: application/json" \
  -H "X-Otto-Session: $OTTO_SESSION_ID" \
  -H "X-Otto-Token: $OTTO_INGEST_TOKEN" \
  -d "$PAYLOAD") || { echo "otto-product: could not reach $BASE" >&2; exit 1; }
case "$STATUS" in
  2*) echo "otto-product: published \"$TITLE\"${FOLDER:+ in folder $FOLDER}" ;;
  *) echo "otto-product: failed with HTTP $STATUS (bad --kind? not a swarm session?)" >&2; exit 1 ;;
esac
"#;

/// `otto-mockup` — a discovery/design agent publishes a design artifact (an HTML
/// screen, a Mermaid diagram, an Excalidraw board or a `scene3d` document) to
/// the story under discovery — or, outside a Discovery run, to the project's
/// linked story / epic. Mirrors `otto-post`'s per-session auth; the target story
/// is derived server-side. Unknown `--format` values are rejected locally AND by
/// the server (400), never silently stored as HTML.
const OTTO_MOCKUP: &str = r#"#!/bin/sh
# otto-mockup — publish a design artifact for this project's story on the Product page.
# Usage: otto-mockup --title "Title" [--format html|mermaid|excalidraw|scene3d] [--folder F] "<content>"
BASE="${OTTO_INGEST_BASE:-http://127.0.0.1:7700}"
TITLE=""; FORMAT="html"; FOLDER=""; CONTENT=""
while [ $# -gt 0 ]; do
  case "$1" in
    --title) TITLE="$2"; shift 2 ;;
    --format) FORMAT="$2"; shift 2 ;;
    --folder) FOLDER="$2"; shift 2 ;;
    --) shift; CONTENT="$*"; break ;;
    --*) # Unknown flag only while no content has started and the arg is one
         # token; content starting with `--`/`---` (front-matter, hrule) is content.
         if [ -z "$CONTENT" ] && [ "$1" = "${1%%[[:space:]]*}" ]; then
           echo "otto-mockup: unknown flag $1 (known: --title --format --folder)" >&2; exit 2
         fi
         if [ -z "$CONTENT" ]; then CONTENT="$1"; else CONTENT="$CONTENT $1"; fi; shift ;;
    *) if [ -z "$CONTENT" ]; then CONTENT="$1"; else CONTENT="$CONTENT $1"; fi; shift ;;
  esac
done
if [ -z "$CONTENT" ]; then echo "usage: otto-mockup --title \"Title\" [--format html|mermaid|excalidraw|scene3d] \"content\"" >&2; exit 1; fi
case "$FORMAT" in
  html|mermaid|excalidraw|scene3d) ;;
  *) echo "otto-mockup: unknown --format $FORMAT (html|mermaid|excalidraw|scene3d)" >&2; exit 2 ;;
esac
PAYLOAD=$(TITLE="$TITLE" FORMAT="$FORMAT" FOLDER="$FOLDER" CONTENT="$CONTENT" python3 - <<'PY'
import json, os
d = {"title": os.environ.get("TITLE",""), "format": os.environ.get("FORMAT","html"), "content": os.environ.get("CONTENT","")}
if os.environ.get("FOLDER"): d["folder"] = os.environ["FOLDER"]
print(json.dumps(d))
PY
)
STATUS=$(curl -sS -o /dev/null -w '%{http_code}' -X POST "$BASE/api/v1/ingest/swarm/mockup" \
  -H "Content-Type: application/json" \
  -H "X-Otto-Session: $OTTO_SESSION_ID" \
  -H "X-Otto-Token: $OTTO_INGEST_TOKEN" \
  -d "$PAYLOAD") || { echo "otto-mockup: could not reach $BASE" >&2; exit 1; }
case "$STATUS" in
  2*) echo "otto-mockup: published \"$TITLE\" ($FORMAT)" ;;
  *) echo "otto-mockup: failed with HTTP $STATUS (bad --format? no story for this project?)" >&2; exit 1 ;;
esac
"#;

/// `otto-discovery-report` — a discovery agent publishes the consolidated
/// discovery report (markdown) for the story under discovery. Mirrors
/// `otto-post`'s per-session auth; the target run is derived server-side from the
/// session's project. Usage: otto-discovery-report "<markdown report>".
const OTTO_DISCOVERY_REPORT: &str = r#"#!/bin/sh
# otto-discovery-report — publish the consolidated discovery report (markdown).
# Usage: otto-discovery-report "<markdown report>"
BASE="${OTTO_INGEST_BASE:-http://127.0.0.1:7700}"
REPORT=""
while [ $# -gt 0 ]; do
  case "$1" in
    --) shift; REPORT="$*"; break ;;
    *) if [ -z "$REPORT" ]; then REPORT="$1"; else REPORT="$REPORT $1"; fi; shift ;;
  esac
done
if [ -z "$REPORT" ]; then echo "usage: otto-discovery-report \"<markdown report>\"" >&2; exit 1; fi
PAYLOAD=$(REPORT="$REPORT" python3 - <<'PY'
import json, os
print(json.dumps({"report_md": os.environ.get("REPORT","")}))
PY
)
curl -s -X POST "$BASE/api/v1/ingest/swarm/discovery-report" \
  -H "Content-Type: application/json" \
  -H "X-Otto-Session: $OTTO_SESSION_ID" \
  -H "X-Otto-Token: $OTTO_INGEST_TOKEN" \
  -d "$PAYLOAD" >/dev/null 2>&1
"#;

/// Materialize the agent's skills + soul + identity into `cwd`, and install the
/// `otto-post` board helper + `otto-product` draft helper + `otto-mockup` /
/// `otto-discovery-report` discovery helpers. Best-effort.
pub fn provision_agent(
    ctx: &ServerCtx,
    swarm: &Swarm,
    project: Option<&SwarmProject>,
    agent: &SwarmAgent,
    identity_md: String,
    cwd: &str,
) {
    let cfg = WorkspaceContextConfig {
        skills: Some(resolve_skills(swarm, project, agent)),
        soul: agent.soul_name.clone(),
        extra_context_md: identity_md,
        include_memory: true,
        repo_rules_md: String::new(),
        ..Default::default()
    };
    let ctx_root = otto_context::materialize::default_context_root();
    let _ =
        otto_context::materialize::provision(&ctx.context_library, &cfg, cwd, &agent.provider, &ctx_root);
    install_helper(cwd, "otto-post", OTTO_POST);
    install_helper(cwd, "otto-product", OTTO_PRODUCT);
    install_helper(cwd, "otto-mockup", OTTO_MOCKUP);
    install_helper(cwd, "otto-discovery-report", OTTO_DISCOVERY_REPORT);
}

/// Write a helper script into `cwd` and mark it executable (best-effort).
pub fn install_helper(cwd: &str, name: &str, body: &str) {
    let path = std::path::Path::new(cwd).join(name);
    if std::fs::write(&path, body).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn swarm(config: serde_json::Value) -> Swarm {
        let now = chrono::Utc::now();
        Swarm {
            id: "s".into(),
            workspace_id: "w".into(),
            name: "t".into(),
            description: String::new(),
            preset_slug: None,
            status: "active".into(),
            config,
            max_total_runs: None,
            max_cost_usd: None,
            max_runtime_secs: None,
            max_attempts: 3,
            run_started_at: None,
            pause_reason: None,
            created_by: "u".into(),
            created_at: now,
            updated_at: now,
        }
    }

    fn project(skills: serde_json::Value) -> SwarmProject {
        let now = chrono::Utc::now();
        SwarmProject {
            id: "p".into(),
            swarm_id: "s".into(),
            workspace_id: "w".into(),
            name: "proj".into(),
            description: String::new(),
            repo_path: Some("/tmp/repo".into()),
            goal_md: None,
            story_id: None,
            skills,
            integration_branch: None,
            origin_channel: None,
            origin_chat: None,
            origin_thread: None,
            status: "active".into(),
            order_idx: 0,
            created_by: "u".into(),
            created_at: now,
            updated_at: now,
        }
    }

    fn agent(skills: serde_json::Value, cwd_mode: Option<String>) -> SwarmAgent {
        let now = chrono::Utc::now();
        SwarmAgent {
            id: "a".into(),
            swarm_id: "s".into(),
            workspace_id: "w".into(),
            name: "Dev".into(),
            title: "Backend".into(),
            reports_to: None,
            provider: "claude".into(),
            model: None,
            soul_name: None,
            soul_md: None,
            specialization: String::new(),
            scope_md: String::new(),
            skills,
            schedule: None,
            cwd_mode,
            avatar: String::new(),
            status: "active".into(),
            order_idx: 0,
            created_by: "u".into(),
            created_at: now,
            updated_at: now,
        }
    }

    /// The helper scripts parse their flags INSIDE the `case` block (an unknown
    /// `--flag` errors instead of being appended to the body) and fail loudly.
    #[test]
    fn helper_scripts_parse_flags_and_fail_loud() {
        for (name, body, flags) in [
            ("otto-product", OTTO_PRODUCT, &["--title", "--folder", "--kind"][..]),
            ("otto-mockup", OTTO_MOCKUP, &["--title", "--format", "--folder"][..]),
        ] {
            for f in flags {
                assert!(body.contains(&format!("    {f}) ")), "{name}: {f} handled in case");
            }
            // Unknown flags are rejected ONLY while the body is empty and the arg is a
            // single token — `---` front-matter / an hrule body (has whitespace) is body.
            assert!(body.contains("--*) #"), "{name}: unknown-flag arm present");
            assert!(
                body.contains("[ -z \"$BODY\" ] && [ \"$1\" = \"${1%%[[:space:]]*}\" ]")
                    || body.contains("[ -z \"$CONTENT\" ] && [ \"$1\" = \"${1%%[[:space:]]*}\" ]"),
                "{name}: unknown-flag arm guarded by empty-body + single-token"
            );
            assert!(body.contains("exit 2"), "{name}: unknown flags exit 2");
            assert!(body.contains("-w '%{http_code}'"), "{name}: reports HTTP status");
            assert!(!body.contains(">/dev/null 2>&1"), "{name}: no swallowed curl");
            assert!(body.contains("exit 1 ;;"), "{name}: non-zero on failure");
        }
        assert!(OTTO_PRODUCT.contains("\"tree_kind\""), "--kind maps to tree_kind");
        assert!(OTTO_MOCKUP.contains("html|mermaid|excalidraw|scene3d"));
    }

    #[test]
    fn resolve_skills_unions_team_project_agent() {
        // Team (string form) + project ({name} form) + agent — union, deduped,
        // team-first order, both shapes accepted.
        let sw = swarm(json!({ "skills": ["team-a", "shared"] }));
        let pr = project(json!([{ "name": "proj-b" }, { "name": "shared" }]));
        let ag = agent(json!([{ "name": "agent-c", "must_use": true }, { "name": "team-a" }]), None);
        let got = resolve_skills(&sw, Some(&pr), &ag);
        assert_eq!(got, vec!["team-a", "shared", "proj-b", "agent-c"]);
    }

    #[test]
    fn resolve_skills_agent_only_when_no_layers() {
        let sw = swarm(json!({}));
        let ag = agent(json!([{ "name": "only" }]), None);
        assert_eq!(resolve_skills(&sw, None, &ag), vec!["only"]);
    }

    #[test]
    fn cwd_mode_defaults_to_worktree_for_code_projects() {
        let sw = swarm(json!({}));
        let ag = agent(json!([]), None);
        // Has repo, no per-agent override → worktree (isolation), not "repo".
        assert_eq!(cwd_mode(&sw, &ag, true), "worktree");
        // No repo → scratch.
        assert_eq!(cwd_mode(&sw, &ag, false), "scratch");
        // Explicit per-agent "repo" still opts out (single-agent intent).
        let ag_repo = agent(json!([]), Some("repo".into()));
        assert_eq!(cwd_mode(&sw, &ag_repo, true), "repo");
    }
}
