//! Goals (goal.md §3, §5.3): every source — the Goal form, MCP
//! `otto.run_goal_loop`, Continue as a goal and Git's Fix review — starts its
//! goal through [`start_built`]: one workflow row of its own (`origin = goal`)
//! and one run on the ordinary run path (admission, budget, lanes).

use otto_core::api::{CreateGoalLoopReq, GoalSpec, GoalWork};
use otto_core::domain::{GoalLoop, GoalLoopStatus, Repo, User};
use otto_core::workflows::{RunScope, Workflow, WorkflowRun};
use otto_core::{Error, Id, Result};
use otto_state::WorkflowsRepo;
use otto_workflows::goal::{self, BuiltGoal, GoalRepo};

use crate::state::ServerCtx;
use crate::workflow_engine;

/// Store a built goal as its own workflow and start its run in the long lane.
pub async fn start_built(
    ctx: &ServerCtx,
    user: &User,
    ws_id: &Id,
    built: BuiltGoal,
    origin_ref: Option<&str>,
) -> Result<(Workflow, WorkflowRun)> {
    crate::workflow_validation::ensure_valid(&built.graph)?;
    let budget = crate::budget::check_budget(ctx, ws_id, "").await;
    if budget.blocked {
        return Err(Error::Invalid(
            budget
                .reason
                .unwrap_or_else(|| "Agent budget exceeded".into()),
        ));
    }
    let ws = ctx.workspaces.get(ws_id).await?;
    workflow_engine::ensure_run_location(ctx, &ws, &built.input).await?;
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    let wf = repo
        .create(
            ws_id,
            &built.name,
            &built.description,
            "",
            &built.graph,
            &user.id,
        )
        .await?;
    repo.set_origin(&wf.id, Some("goal"), origin_ref).await?;
    let wf = repo.get(&wf.id).await?;
    let run = repo
        .create_run(&wf.id, ws_id, &built.input, Some(&user.id))
        .await?;
    let id = run.id.clone();
    workflow_engine::spawn_run(
        ctx.clone(),
        ws,
        wf.clone(),
        id,
        built.input,
        RunScope::default(),
        None,
    );
    Ok((wf, run))
}

/// A workspace repository and the commit (`base`, default HEAD) a run worktree is cut from.
async fn repo_at(
    ctx: &ServerCtx,
    ws_id: &Id,
    repo_id: &Id,
    base: Option<&str>,
) -> Result<(Repo, String)> {
    let repo = ctx.git_store.get_repo(repo_id).await?;
    if &repo.workspace_id != ws_id {
        return Err(Error::Invalid(
            "work.repo_id: not a repository of this workspace".into(),
        ));
    }
    let source = otto_git::LocalGit::new(&repo.path)
        .rev_parse(base.unwrap_or("HEAD"))
        .await?;
    Ok((repo, source))
}

/// `POST /workspaces/{wid}/goals`: a spec from the Goal form (or MCP).
pub async fn start(
    ctx: &ServerCtx,
    user: &User,
    ws_id: &Id,
    spec: &GoalSpec,
    origin_ref: Option<&str>,
) -> Result<(Workflow, WorkflowRun)> {
    goal::validate_spec(spec).map_err(Error::Invalid)?;
    let built = match &spec.work {
        GoalWork::Build { repo_id, base } => {
            let (repo, source) = repo_at(ctx, ws_id, repo_id, base.as_deref()).await?;
            goal::goal_workflow(
                spec,
                Some(&GoalRepo {
                    id: &repo.id,
                    path: &repo.path,
                    source: &source,
                }),
            )
        }
        GoalWork::Research => goal::goal_workflow(spec, None),
    };
    start_built(ctx, user, ws_id, built, origin_ref).await
}

async fn repo_by_path(ctx: &ServerCtx, ws_id: &Id, path: &str) -> Result<Option<Repo>> {
    Ok(ctx
        .git_store
        .list_repos(ws_id)
        .await?
        .into_iter()
        .find(|r| r.path == path))
}

/// MCP `otto.run_goal_loop` (`POST /workspaces/{wid}/goals/from-loop`): the
/// Goal Loops request as a goal, plus the roles it ignored.
pub async fn from_loop(
    ctx: &ServerCtx,
    user: &User,
    ws_id: &Id,
    req: &CreateGoalLoopReq,
) -> Result<(Workflow, WorkflowRun, Vec<String>)> {
    let repo = repo_by_path(ctx, ws_id, &req.repo_path).await?;
    let repo_id = repo.as_ref().map(|r| r.id.as_str());
    let (spec, ignored) =
        goal::goal_spec_from_loop_args(&req.definition, &req.limits, &req.config, repo_id)
            .map_err(Error::Invalid)?;
    let (wf, run) = start(ctx, user, ws_id, &spec, Some("mcp")).await?;
    Ok((wf, run, ignored))
}

/// Continue an older loop as a goal (goal.md §5.3): in the loop's kept work
/// (its path from the row, never the client's), from its base commit, with
/// what it had learnt; the loop is then stopped.
pub async fn continue_loop(
    ctx: &ServerCtx,
    user: &User,
    loop_: &GoalLoop,
    mut spec: GoalSpec,
) -> Result<(Workflow, WorkflowRun)> {
    use GoalLoopStatus::*;
    if !matches!(loop_.status, Draft | Paused | Blocked | Exhausted) {
        let status = loop_.status.as_str();
        return Err(Error::Conflict(format!(
            "A {status} goal loop cannot be continued"
        )));
    }
    let ws_id = &loop_.workspace_id;
    let origin_ref = format!("goal_loop:{}", loop_.id);
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    if repo
        .find_goal_by_origin_ref(ws_id, &origin_ref)
        .await?
        .is_some()
    {
        return Err(Error::Conflict(
            "This goal loop was already continued as a goal".into(),
        ));
    }
    goal::validate_spec(&spec).map_err(Error::Invalid)?;
    otto_automation::goal_loop_history::cleanup_executor_sessions(ctx, ws_id, &loop_.id).await;
    goal::carry_loop_context(&mut spec, loop_);
    let kept = loop_.worktree_path.clone();
    let built = if loop_.config.mode == "research" {
        spec.work = GoalWork::Research;
        let work = ctx.data_dir.join("goal-loops").join(&loop_.id).join("work");
        let dir = kept.map_or(work, std::path::PathBuf::from);
        // No kept folder on disk: `research_dir: true` (a fresh research folder).
        let mut built = goal::goal_workflow(&spec, None);
        if tokio::fs::metadata(&dir).await.is_ok_and(|m| m.is_dir()) {
            built.input["research_dir"] = dir.to_string_lossy().into_owned().into();
        }
        built
    } else {
        let r = repo_by_path(ctx, ws_id, &loop_.repo_path)
            .await?
            .ok_or_else(|| {
                Error::Invalid(format!(
                    "{} is not a registered repository any more",
                    loop_.repo_path
                ))
            })?;
        let source = match &loop_.base_commit {
            Some(sha) => sha.clone(),
            None => otto_git::LocalGit::new(&r.path).rev_parse("HEAD").await?,
        };
        spec.work = GoalWork::Build {
            repo_id: r.id.clone(),
            base: None,
        };
        let path = kept.as_deref().unwrap_or(&r.path);
        goal::goal_workflow(
            &spec,
            Some(&GoalRepo {
                id: &r.id,
                path,
                source: &source,
            }),
        )
    };
    let (wf, run) = start_built(ctx, user, ws_id, built, Some(&origin_ref)).await?;
    let summary = format!("Continued as a goal — run {}", run.id);
    ctx.goal_loops_repo
        .finalize(&loop_.id, Stopped, Some(&summary), None)
        .await?;
    Ok((wf, run))
}
