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
    // 1. An explicit PER-AGENT choice always wins (e.g. "worktree" for isolation).
    if let Some(m) = agent.cwd_mode.clone().filter(|s| !s.trim().is_empty()) {
        return m;
    }
    // 2. A project repo path is a strong "work here" signal — it beats the
    //    swarm-level default, which is almost always the auto-inserted "scratch"
    //    (swarm creation stamps config.cwd_mode="scratch"). Honoring the path the
    //    operator set is what they expect.
    if has_repo {
        return "repo".to_string();
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

/// Ensure the agent has a prepared, unique working directory. Returns its path.
pub async fn ensure_cwd(
    ctx: &ServerCtx,
    swarm: &Swarm,
    agent: &SwarmAgent,
    project: Option<&SwarmProject>,
) -> Result<String> {
    let repo = project.and_then(|p| p.repo_path.clone());
    let mode = cwd_mode(swarm, agent, repo.is_some());

    if mode == "repo" {
        if let Some(r) = &repo {
            return Ok(r.clone());
        }
    }

    if mode == "worktree" {
        if let Some(repo_path) = &repo {
            let wt = swarm_base(ctx, &swarm.id, &agent.id).join("wt");
            let wt_str = wt.to_string_lossy().to_string();
            let branch = format!("swarm/{}/{}", short(&swarm.id), short(&agent.id));
            let git = otto_git::LocalGit::new(repo_path);
            // Base the worktree on the repo's current HEAD — but ONLY when it
            // doesn't exist yet. `worktree_add_if_absent` reuses an existing
            // tree as-is, so a resumed turn lands in the SAME worktree with the
            // agent's prior commits intact (the old unconditional `-B`/`--force`
            // reset the branch to base HEAD every turn and discarded that work).
            let base = git
                .current_branch()
                .await
                .unwrap_or_else(|_| "HEAD".to_string());
            match git.worktree_add_if_absent(&wt_str, &branch, &base).await {
                Ok(created) => {
                    if created {
                        tracing::info!("swarm: created worktree {} on {}", wt_str, branch);
                    }
                    return Ok(wt_str);
                }
                Err(e) => {
                    tracing::warn!("swarm: worktree add failed ({}), falling back to scratch: {e}", wt_str);
                }
            }
        }
    }

    // scratch (default, and the fallback).
    let scratch = swarm_base(ctx, &swarm.id, &agent.id).join("work");
    let _ = std::fs::create_dir_all(&scratch);
    Ok(scratch.to_string_lossy().to_string())
}

/// Skill names the agent should have active (must-use + recommended).
fn skill_names(agent: &SwarmAgent) -> Vec<String> {
    agent
        .skills
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.get("name").and_then(|v| v.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
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
/// Product page (a new draft story the user/PO reviews). Mirrors `otto-post`'s
/// per-session auth.
const OTTO_PRODUCT: &str = r#"#!/bin/sh
# otto-product — publish a feature DRAFT to the Product page for the user/PO to
# review. Usage: otto-product --title "Feature title" "draft markdown body"
BASE="${OTTO_INGEST_BASE:-http://127.0.0.1:7700}"
TITLE=""; BODY=""
while [ $# -gt 0 ]; do
  case "$1" in
    --title) TITLE="$2"; shift 2 ;;
    --) shift; BODY="$*"; break ;;
    *) if [ -z "$BODY" ]; then BODY="$1"; else BODY="$BODY $1"; fi; shift ;;
  esac
done
if [ -z "$BODY" ]; then echo "usage: otto-product --title \"Title\" \"markdown body\"" >&2; exit 1; fi
PAYLOAD=$(TITLE="$TITLE" BODY="$BODY" python3 - <<'PY'
import json, os
print(json.dumps({"title": os.environ.get("TITLE",""), "body_md": os.environ.get("BODY","")}))
PY
)
curl -s -X POST "$BASE/api/v1/ingest/swarm/product" \
  -H "Content-Type: application/json" \
  -H "X-Otto-Session: $OTTO_SESSION_ID" \
  -H "X-Otto-Token: $OTTO_INGEST_TOKEN" \
  -d "$PAYLOAD" >/dev/null 2>&1
"#;

/// Materialize the agent's skills + soul + identity into `cwd`, and install the
/// `otto-post` board helper + `otto-product` draft helper. Best-effort.
pub fn provision_agent(
    ctx: &ServerCtx,
    agent: &SwarmAgent,
    identity_md: String,
    cwd: &str,
) {
    let cfg = WorkspaceContextConfig {
        skills: Some(skill_names(agent)),
        soul: agent.soul_name.clone(),
        extra_context_md: identity_md,
        include_memory: true,
    };
    let _ = otto_context::materialize::provision(&ctx.context_library, &cfg, cwd, &agent.provider);
    install_helper(cwd, "otto-post", OTTO_POST);
    install_helper(cwd, "otto-product", OTTO_PRODUCT);
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
