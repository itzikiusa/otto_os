//! Admission: everything a run does before its first node — validate the
//! graph and resolve the run scope, resolve whom it acts as, seed and
//! normalize the input, isolate declared repos on dedicated worktrees, write
//! the run's context files and build the per-node environment. Then the
//! Pending→Running CAS and the chat progress pump. Moved verbatim out of
//! `run_workflow_with_budget`; a failure here settles the run itself.

use super::super::*;
use super::state::RunState;

/// A run that passed admission: what the node loop and finalize need.
pub(in crate::workflow_engine) struct Admitted {
    pub(in crate::workflow_engine) order: Vec<String>,
    /// The set of nodes to actually execute; `None` means the whole graph.
    pub(in crate::workflow_engine) run_set: Option<HashSet<String>>,
    pub(in crate::workflow_engine) continue_unfinished: bool,
    pub(in crate::workflow_engine) rs: RunState,
    pub(in crate::workflow_engine) user: User,
    pub(in crate::workflow_engine) input: Value,
    pub(in crate::workflow_engine) files: std::sync::Arc<crate::workflow_context::RunContextFiles>,
    pub(in crate::workflow_engine) env: RunEnv,
}

/// Admit a run, or settle it (invalid graph, failed isolation) and return
/// `None`.
#[allow(clippy::too_many_arguments)]
pub(in crate::workflow_engine) async fn admit(
    ctx: &ServerCtx,
    ws: &Workspace,
    workflow: &Workflow,
    run_id: &Id,
    input: Value,
    scope: RunScope,
    prior_nodes: Option<&[NodeRunState]>,
    repo: &WorkflowsRepo,
    active_budget: Duration,
) -> Option<Admitted> {
    let order = valid_order(ctx, repo, workflow, run_id).await?;
    let RunScope {
        start_node,
        only_node,
        adopt_start,
        continue_unfinished,
    } = scope;
    let run_set = scope_set(
        &workflow.graph,
        start_node.as_deref(),
        only_node,
        adopt_start,
    );
    let rs = RunState::new(&workflow.graph);
    // Resolve the user this run acts as (for spawning visible agent sessions):
    // the user who STARTED the run when known, else the latest changer of what
    // it runs (trigger / schedule / webhook / chat runs, PC-3). Acting as the
    // starter is what lets a non-root editor attach to the sessions their own
    // run spawns — those are gated owner-or-admin, and the workflow's creator
    // is usually someone else.
    let acting = run_identity(ctx, run_id, &workflow.created_by).await;
    let user = resolve_run_user(ctx, &acting).await;
    let input = seed_from_manual_trigger(workflow, input);
    // Per-run context files (design 2026-07-02): `<data_dir>/workflow-context/
    // <run_id>/` holds the instruction brief, the live `repos.json` registry
    // and per-step handoff files. Best-effort — a failed dir create disables
    // the handle and the run proceeds on legacy inline context.
    let files = std::sync::Arc::new(crate::workflow_context::RunContextFiles::create(
        &ctx.data_dir,
        run_id,
    ));
    let input = isolate_repos(ctx, repo, workflow, run_id, input, &files, &rs, prior_nodes).await?;
    let prepared = prepare_input(ctx, ws, workflow, input, &files).await;
    write_brief(
        workflow,
        run_id,
        &order,
        run_set.as_ref(),
        &prepared,
        &files,
    );
    let env = run_env(ctx, ws, workflow, run_id, &prepared, &files, active_budget).await;
    Some(Admitted {
        order,
        run_set,
        continue_unfinished,
        rs,
        user,
        input: prepared.input,
        files,
        env,
    })
}

/// The graph's topological order, or — for an invalid graph — the run
/// settled as Error and `None`.
async fn valid_order(
    ctx: &ServerCtx,
    repo: &WorkflowsRepo,
    workflow: &Workflow,
    run_id: &Id,
) -> Option<Vec<String>> {
    match crate::workflow_validation::ensure_valid(&workflow.graph)
        .map_err(|e| e.to_string())
        .and_then(|_| topo_order(&workflow.graph))
    {
        Ok(o) => Some(o),
        Err(e) => {
            // Conditional: an invalid graph must not turn a user's cancel
            // into an error (nor announce one).
            let Ok(Some(rev)) = repo
                .update_run_if(
                    run_id,
                    &[RunStatus::Pending, RunStatus::Running],
                    RunStatus::Error,
                    &[],
                    Some(&e),
                    true,
                )
                .await
            else {
                return None;
            };
            emit_run_updated(
                ctx,
                &workflow.workspace_id,
                run_id,
                "error",
                None,
                rev,
                None,
                &[],
                false,
            );
            None
        }
    }
}

/// The set of nodes to actually execute (start-from-here / run-only); `None`
/// means the whole graph. Nodes outside the set are marked skipped. An
/// `adopt_start` scope (restart resume whose entry step provably finished)
/// treats the entry itself as done — it drops OUT of the set so the adoption
/// pass keeps its prior Success state/output while its descendants run.
pub(in crate::workflow_engine) fn scope_set(
    graph: &WorkflowGraph,
    start_node: Option<&str>,
    only_node: bool,
    adopt_start: bool,
) -> Option<HashSet<String>> {
    match start_node {
        None => None,
        Some(s) if only_node => {
            let mut set: HashSet<String> = std::iter::once(s.to_string()).collect();
            if adopt_start {
                // Single-step scope whose step is already proven done —
                // nothing executes; the run finalizes over adopted states.
                set.remove(s);
            }
            Some(set)
        }
        Some(s) => {
            let mut set = descendants_inclusive(graph, s);
            if adopt_start {
                set.remove(s);
            }
            Some(set)
        }
    }
}

/// Seed the run input from the entry manual_trigger node's configured fields
/// (its inspector — prompt/working_directory/repo_id/goals/…), letting the
/// actual /run body or chat-trigger input override per key. This is what makes
/// the Start node the place to set the input for a manual run.
fn seed_from_manual_trigger(workflow: &Workflow, input: Value) -> Value {
    let mut seeded = serde_json::Map::new();
    if let Some(mt) = workflow
        .graph
        .nodes
        .iter()
        .find(|n| n.kind == "manual_trigger")
    {
        if let Some(o) = mt.params.as_object() {
            for (k, v) in o {
                if !v.is_null() {
                    seeded.insert(k.clone(), v.clone());
                }
            }
        }
    }
    if let Some(o) = input.as_object() {
        for (k, v) in o {
            seeded.insert(k.clone(), v.clone());
        }
    }
    if seeded.is_empty() {
        input
    } else {
        Value::Object(seeded)
    }
}

/// Declared repos/branches/worktrees (`repos: [{repo, type, name, source}]`)
/// normalized against registered repos + live git state, then seeded into
/// the run input BEFORE run_cwd/run_base derive — so declaring repos is all a
/// run needs for every git-aware step to know source+destination. `None` when
/// isolation failed: the run is already settled as Error.
#[allow(clippy::too_many_arguments)]
async fn isolate_repos(
    ctx: &ServerCtx,
    repo: &WorkflowsRepo,
    workflow: &Workflow,
    run_id: &Id,
    input: Value,
    files: &crate::workflow_context::RunContextFiles,
    rs: &RunState,
    prior_nodes: Option<&[NodeRunState]>,
) -> Option<Value> {
    // The run's Base branch (from the input) is the fallback for a declared
    // repo that doesn't name its own `source` — so setting Base actually pins
    // the declaration's base, instead of it always falling back to the repo's
    // detected default branch. (R14)
    let run_base_hint = input
        .get("base")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let mut declared =
        crate::workflow_context::parse_repo_entries(input.get("repos").unwrap_or(&Value::Null));
    // No repos declared but a `Working Directory:` was given → derive a single
    // worktree entry from it, so repos.json is built AT START by looking up the
    // registered repo AT THAT PATH (a worktree path resolves back to its origin
    // repo). This honors the originating repository's identity — name, git
    // account, default branch — instead of a later step publishing a bare,
    // unnamed entry. Explicit `repos:` declarations are never overridden.
    if declared.is_empty() {
        if let Some(wd) = input
            .get("working_directory")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            // A comma-separated `Working Directory:` declares SEVERAL repos
            // (e.g. a cross-service change reviewed as one run) — one entry
            // per path, all sharing the run's base hint.
            for path in wd.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                declared.push(crate::workflow_context::RepoEntry {
                    repo: path.to_string(),
                    repo_id: None,
                    repo_name: None,
                    kind: "worktree".into(),
                    name: path.to_string(),
                    source: None,
                    worktree: None,
                    base: None,
                    error: None,
                });
            }
        }
    }
    let mut entries = resolve_repo_entries(
        ctx,
        &workflow.workspace_id,
        declared,
        run_base_hint.as_deref(),
    )
    .await;
    // Put the work on a clean, ISOLATED checkout of the BASE branch: for a repo
    // we'd otherwise operate on in the user's own working copy, cut a dedicated
    // linked worktree from `base` under the data dir and repoint the entry at
    // it — so agent steps never run on whatever branch the user's checkout
    // happens to be on. Best-effort + SAFE (only creates a new worktree under
    // data_dir; never touches the user's checkout). Returns the primary
    // provisioned worktree to adopt as the run's working directory.
    let (provisioned_cwd, isolation_failures) =
        provision_wf_worktrees(ctx, run_id, &mut entries).await;
    files.set_repos(entries.clone());
    if !isolation_failures.is_empty() {
        // Fail LOUDLY before any step runs: never let agents work in the
        // user's own checkout because isolation silently failed.
        let msg = format!(
            "cannot isolate this run from your own checkout — {}",
            isolation_failures.join("; ")
        );
        tracing::warn!(%run_id, "{msg}");
        // Nothing ran: a retry keeps its prior step history, a fresh run
        // shows every step skipped.
        let nodes: Vec<NodeRunState> = match prior_nodes {
            Some(prior) => prior.to_vec(),
            None => rs
                .states
                .iter()
                .cloned()
                .map(|mut s| {
                    s.status = NodeStatus::Skipped;
                    s
                })
                .collect(),
        };
        let rev = repo
            .update_run_if(
                run_id,
                &[RunStatus::Pending, RunStatus::Running],
                RunStatus::Error,
                &nodes,
                Some(&msg),
                true,
            )
            .await
            .ok()
            .flatten();
        if let Some(rev) = rev {
            deliver_run_result(ctx, workflow, &nodes, RunStatus::Error, &input, None).await;
            emit_run_updated(
                ctx,
                &workflow.workspace_id,
                run_id,
                "error",
                None,
                rev,
                None,
                &nodes,
                false,
            );
        }
        reap_run_worktrees(ctx, run_id).await;
        return None;
    }
    let input = seed_input_from_entries(input, &entries);
    Some(match provisioned_cwd {
        Some(wt) => {
            let mut m = match input {
                Value::Object(m) => m,
                other => {
                    let mut m = serde_json::Map::new();
                    m.insert("input".into(), other);
                    m
                }
            };
            // run_cwd + agent nodes follow the provisioned base worktree.
            m.insert("working_directory".into(), Value::String(wt));
            Value::Object(m)
        }
        None => input,
    })
}

/// The run input after prompt normalization and repo resolution, plus the
/// run-level ambient values derived from it.
struct PreparedInput {
    input: Value,
    has_instructions: bool,
    has_prompt: bool,
    run_cwd: String,
    run_base: Option<String>,
}

/// Fill `prompt`, write the standing-context files, and derive the run's
/// working directory, base branch and repo id.
async fn prepare_input(
    ctx: &ServerCtx,
    ws: &Workspace,
    workflow: &Workflow,
    input: Value,
    files: &crate::workflow_context::RunContextFiles,
) -> PreparedInput {
    // Fill `prompt` from a chat `msg` when the run didn't set one explicitly —
    // `prompt.md` and every agent-facing step want `prompt`; the trigger only
    // guarantees `msg`. Then write the two standing-context files this run
    // actually has: `instructions.md` (the workflow's, verbatim, only when
    // non-empty) and `prompt.md` (this run's ask, only when one exists). Both
    // flags feed the brief/preamble "how to use this directory" sections below
    // so they only ever point agents at files that exist.
    let input = normalize_prompt(input);
    let has_instructions = !workflow.instructions.trim().is_empty();
    if has_instructions {
        files.write_instructions_md(&workflow.instructions);
    }
    let has_prompt = input
        .get("prompt")
        .and_then(Value::as_str)
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    if let Some(p) = input
        .get("prompt")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
    {
        files.write_prompt_md(p);
    }
    // Run-level working directory: the `working_directory` from the run input
    // (e.g. a Slack `Working Directory:` field), else the workspace root. Agent
    // nodes run here — so a workflow owned by workspace A can operate on repo X.
    let run_cwd = input
        .get("working_directory")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(expand_tilde)
        .unwrap_or_else(|| ws.root_path.clone());
    // Run-level base branch: the ambient PR/review base for the whole run, so a
    // `review_run`/`git_pr` is aware of the run's base even after intervening
    // agent nodes (which return `{reply}`) drop it from the data flow. Per-node
    // `base` params still win. `None` ⇒ each git step resolves its repo's
    // DETECTED default branch at the point of use — never a fabricated "main"
    // (the historical literal here made `git diff main` exit 128 on repos
    // whose default is master/develop).
    let run_base: Option<String> = input
        .get("base")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    // Resilience (design §B): many steps (review_run, git_pr, …) need a repo_id.
    // If the run wasn't given one but DOES carry a working_directory, resolve the
    // registered repo for that path ONCE here and seed it into the run input — so
    // every downstream step inherits it. This is what makes a workflow that only
    // sets `Working Directory:` work, instead of failing with "missing repo_id".
    let mut input = input;
    let has_repo = input
        .get("repo_id")
        .and_then(Value::as_str)
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    if !has_repo {
        if let Some(rid) = resolve_repo_id_for_path(ctx, &workflow.workspace_id, &run_cwd).await {
            if let Value::Object(m) = &mut input {
                m.insert("repo_id".into(), Value::String(rid));
            }
        }
    }
    PreparedInput {
        input,
        has_instructions,
        has_prompt,
        run_cwd,
        run_base,
    }
}

/// The run's mission brief (`run-brief.md`): mission, repos table, planned
/// steps (IN-SCOPE only, so numbering matches what actually executes on a
/// start-from-here run) and the step-file protocol.
fn write_brief(
    workflow: &Workflow,
    run_id: &Id,
    order: &[String],
    run_set: Option<&HashSet<String>>,
    prepared: &PreparedInput,
    files: &crate::workflow_context::RunContextFiles,
) {
    let planned: Vec<(String, String)> = order
        .iter()
        .filter(|nid| run_set.map(|s| s.contains(*nid)).unwrap_or(true))
        .filter_map(|nid| workflow.graph.nodes.iter().find(|n| &n.id == nid))
        .map(|n| {
            (
                if n.name.is_empty() {
                    n.kind.clone()
                } else {
                    n.name.clone()
                },
                n.kind.clone(),
            )
        })
        .collect();
    files.write_brief(&crate::workflow_context::render_brief(
        &workflow.name,
        &workflow.description,
        run_id,
        &prepared.input,
        &files.repos(),
        &planned,
        prepared.has_instructions,
        prepared.has_prompt,
    ));
}

/// The per-node execution environment (run identity + ambient cwd/base +
/// context files). The registry on `files` is the run's source of truth for
/// repos/branches — input threading loses keys hop by hop.
async fn run_env(
    ctx: &ServerCtx,
    ws: &Workspace,
    workflow: &Workflow,
    run_id: &Id,
    prepared: &PreparedInput,
    files: &std::sync::Arc<crate::workflow_context::RunContextFiles>,
    active_budget: Duration,
) -> RunEnv {
    // Resolve the workspace's effective default agent once (workspace default →
    // global default → "claude"), so nodes left on "default" honor the user's
    // configured provider instead of a bare hardcoded "claude".
    let global_default_setting = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    // `resolve_default` (not bare `resolve_provider`) so a DISABLED provider is
    // never auto-selected — it skips excluded providers, matching the UI picker.
    let default_provider = ctx.manager.providers().resolve_default(&[
        otto_core::provider::workspace_default(&ws.settings),
        otto_core::provider::global_default(global_default_setting.as_ref()),
    ]);
    RunEnv {
        budget: budget::ActiveBudget::new(active_budget),
        run_id: run_id.clone(),
        wf_name: workflow.name.clone(),
        run_cwd: prepared.run_cwd.clone(),
        run_base: prepared.run_base.clone(),
        files: files.clone(),
        default_provider,
        run_input: prepared.input.clone(),
    }
}

/// Lifecycle transition Pending→Running — a compare-and-set. Startup (repo
/// resolution, `git worktree add` of a full tree) takes seconds; a Cancel
/// that landed in that window already wrote `canceled`, and the old
/// unconditional write flipped it back to `running` and executed the whole
/// workflow the user saw canceled. Every LATER in-loop write uses
/// `update_run_progress` (nodes only, no status) for the same reason.
pub(in crate::workflow_engine) async fn mark_running(
    ctx: &ServerCtx,
    repo: &WorkflowsRepo,
    workflow: &Workflow,
    run_id: &Id,
    rs: &mut RunState,
) {
    match write_status_retrying(
        repo,
        run_id,
        &[RunStatus::Pending],
        RunStatus::Running,
        &rs.states,
        None,
        false,
    )
    .await
    {
        Ok(Some(rev)) => emit_run_updated(
            ctx,
            &workflow.workspace_id,
            run_id,
            "running",
            None,
            rev,
            None,
            &rs.states,
            false,
        ),
        Ok(None) => {
            // Settled (canceled) before it started: execute nothing — the
            // node loop's first iteration breaks straight to the canceled
            // finalize, which reaps the worktrees just provisioned.
            tracing::info!(%run_id, "workflow run canceled during startup — not executing");
            rs.canceled = true;
        }
        Err(e) => tracing::warn!(%run_id, "workflow run start write failed: {e}"),
    }
}

/// Live progress: if this run was triggered from a chat thread, stream brief
/// per-step updates back to it. A single pump task posts them in order; manual
/// UI / webhook-only runs get a disabled (no-op) sink.
pub(in crate::workflow_engine) fn open_progress(
    ctx: &ServerCtx,
    workflow: &Workflow,
    input: &Value,
    steps: usize,
    canceled: bool,
) -> (ProgressSink, Option<tokio::task::JoinHandle<()>>) {
    let (progress, progress_pump) = match resolve_chat_target(workflow, input) {
        Some(target) => {
            let (sink, handle) = spawn_progress_pump(ctx.clone(), target);
            (sink, Some(handle))
        }
        None => (ProgressSink::disabled(), None),
    };
    if progress.enabled() && !canceled {
        let goals: Vec<String> = input
            .get("goals")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let goals_line = if goals.is_empty() {
            String::new()
        } else {
            format!("\n*Goals:* {}", goals.join("; "))
        };
        progress.post(format!(
            "🚀 *{}* started — {} step(s) queued.{}",
            workflow.name, steps, goals_line
        ));
    }
    (progress, progress_pump)
}
