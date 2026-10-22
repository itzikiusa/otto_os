//! Whom a run acts as (PC-3, access-control-3-02): its starter, else the
//! latest changer of what it runs, else the workflow's creator.

use otto_core::Id;
use otto_state::WorkflowsRepo;

use crate::state::ServerCtx;

/// Whose behalf a run executes on: the user who started it when recorded
/// (`workflow_runs.created_by`), else `fallback` — for a trigger / schedule /
/// chat / webhook run, the latest changer [`run_identity`] resolved. The
/// agent sessions a run spawns are created as this user, and session access
/// is owner-or-admin — so a run MUST act as its starter or a non-admin editor
/// can never attach to the sessions of a workflow someone else authored.
pub(in crate::workflow_engine) fn acting_user_id(run_started_by: Option<Id>, fallback: &Id) -> Id {
    run_started_by
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| fallback.clone())
}

/// [`acting_user_id`] for a stored run: its starter, else (a trigger /
/// schedule / webhook / chat run) the latest changer of its graph version or
/// of a trigger spec ([`WorkflowsRepo::run_acting_user`], PC-3), else the
/// workflow's creator. A workspace Editor who rewrites another user's graph or
/// trigger thus makes the next unattended run act as THEM — their DB grants,
/// MCP token and accounts — never the creator's (access-control-3-02).
pub(in crate::workflow_engine) async fn run_identity(
    ctx: &ServerCtx,
    run_id: &Id,
    workflow_created_by: &Id,
) -> Id {
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    let run_starter = repo.run_created_by(run_id).await.ok().flatten();
    if run_starter.as_deref().is_some_and(|s| !s.trim().is_empty()) {
        return acting_user_id(run_starter, workflow_created_by);
    }
    // A lookup failure falls back to the creator, as before PC-3.
    let changer = repo
        .run_acting_user(run_id)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| workflow_created_by.clone());
    acting_user_id(None, &changer)
}
