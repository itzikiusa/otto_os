//! Workflow engine routes: CRUD, the node-type catalog, agent-mode generation
//! (describe a flow → we build the graph), run + run-status, triggers, and
//! the human-approval resume endpoint.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use otto_core::domain::WorkspaceRole;
use otto_core::event::Event;
use otto_core::workflows::{
    ActiveWorkflowRun, CreateWorkflowReq, FromTemplateReq, NodeTypeSpec, RestoreVersionReq,
    RetryRunNodeReq, RunStatus, RunWorkflowReq, UpdateWorkflowReq, Workflow, WorkflowEdge,
    WorkflowGraph, WorkflowNode, WorkflowRun, WorkflowTemplate, WorkflowVersion,
};
use otto_core::{Error, Id};
use otto_state::{NewWorkflowTrigger, TriggersRepo, WorkflowTrigger, WorkflowsRepo};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;
use crate::workflow_engine;

fn repo(ctx: &ServerCtx) -> WorkflowsRepo {
    WorkflowsRepo::new(ctx.pool.clone())
}

/// `GET /workflows/node-types` — the editor palette / executor contract.
pub async fn node_types() -> Json<Vec<NodeTypeSpec>> {
    Json(workflow_engine::node_catalog())
}

/// `GET /workspaces/{wid}/workflows`
pub async fn list_workflows(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<Workflow>>> {
    crate::auth::require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    Ok(Json(repo(&ctx).list(&wid).await.map_err(ApiError)?))
}

/// `POST /workspaces/{wid}/workflows`
pub async fn create_workflow(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateWorkflowReq>,
) -> ApiResult<Json<Workflow>> {
    crate::auth::require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError(Error::Invalid("name must not be empty".into())));
    }
    let graph = req.graph.unwrap_or_default();
    let wf = repo(&ctx)
        .create(
            &wid,
            name,
            req.description.as_deref().unwrap_or(""),
            req.instructions.as_deref().unwrap_or(""),
            &graph,
            &user.id,
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(wf))
}

/// `GET /workflows/{id}`
pub async fn get_workflow(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Workflow>> {
    let wf = repo(&ctx).get(&id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &wf.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(wf))
}

/// `PATCH /workflows/{id}`
pub async fn update_workflow(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpdateWorkflowReq>,
) -> ApiResult<Json<Workflow>> {
    let wf = repo(&ctx).get(&id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &wf.workspace_id, WorkspaceRole::Editor).await?;
    if let Some(v) = req.on_restart.as_deref() {
        if !matches!(v, "resume" | "fail") {
            return Err(ApiError(Error::Invalid(format!(
                "on_restart must be 'resume' or 'fail', got '{v}'"
            ))));
        }
    }
    Ok(Json(
        repo(&ctx)
            .publish(
                &id,
                req.name.as_deref(),
                req.description.as_deref(),
                req.instructions.as_deref(),
                req.graph.as_ref(),
                req.on_restart.as_deref(),
                "edited",
                Some(&user.id),
                None,
            )
            .await
            .map_err(ApiError)?,
    ))
}

#[derive(Debug, Default, Deserialize)]
pub struct VersionQuery {
    #[serde(default)]
    summary: bool,
    limit: Option<i64>,
    before_version: Option<i64>,
}

/// `GET /workflows/{id}/versions` — bounded, newest-first array pages.
pub async fn list_versions(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(query): Query<VersionQuery>,
) -> ApiResult<Json<Value>> {
    let wf = repo(&ctx).get(&id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &wf.workspace_id, WorkspaceRole::Viewer).await?;
    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    if query.summary {
        Ok(Json(json!(repo(&ctx)
            .version_summaries(&id, query.before_version, limit)
            .await
            .map_err(ApiError)?)))
    } else {
        Ok(Json(json!(repo(&ctx)
            .version_page(&id, query.before_version, limit)
            .await
            .map_err(ApiError)?)))
    }
}

/// `GET /workflows/{id}/versions/{v}` — a single snapshot.
pub async fn get_version(
    Path((id, v)): Path<(Id, i64)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<WorkflowVersion>> {
    let wf = repo(&ctx).get(&id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &wf.workspace_id, WorkspaceRole::Viewer).await?;
    repo(&ctx)
        .get_version(&id, v)
        .await
        .map_err(ApiError)?
        .map(Json)
        .ok_or_else(|| ApiError(Error::NotFound(format!("version {v}"))))
}

/// `POST /workflows/{id}/versions/{v}/restore` — copy a version's graph back in
/// as a NEW version (append-only history).
pub async fn restore_version(
    Path((id, v)): Path<(Id, i64)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    body: Option<Json<RestoreVersionReq>>,
) -> ApiResult<Json<Workflow>> {
    let wf = repo(&ctx).get(&id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &wf.workspace_id, WorkspaceRole::Editor).await?;
    let ver = repo(&ctx)
        .get_version(&id, v)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("version {v}"))))?;
    let note = body
        .and_then(|b| b.0.note)
        .unwrap_or_else(|| format!("restored from v{v}"));
    // Preserve live labels, while the appended snapshot keeps historical labels.
    Ok(Json(
        repo(&ctx)
            .publish(
                &id,
                None,
                None,
                Some(&ver.instructions),
                Some(&ver.graph),
                Some(&ver.on_restart),
                &note,
                Some(&user.id),
                Some((&ver.name, &ver.description)),
            )
            .await
            .map_err(ApiError)?,
    ))
}

/// `DELETE /workflows/{id}`
pub async fn delete_workflow(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<axum::http::StatusCode> {
    let wf = repo(&ctx).get(&id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &wf.workspace_id, WorkspaceRole::Editor).await?;
    repo(&ctx).delete(&id).await.map_err(ApiError)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct GenerateReq {
    /// Natural-language description of the flow the user wants.
    pub description: String,
    /// Optional name; defaults to a slug of the description.
    #[serde(default)]
    pub name: Option<String>,
}

/// `POST /workspaces/{wid}/workflows/generate` — agent mode: turn a description
/// into a workflow graph and save it. The primary way users build workflows.
pub async fn generate_workflow(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<GenerateReq>,
) -> ApiResult<Json<Workflow>> {
    crate::auth::require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let description = req.description.trim();
    if description.is_empty() {
        return Err(ApiError(Error::Invalid(
            "description must not be empty".into(),
        )));
    }
    let ws = ctx.workspaces.get(&wid).await.map_err(ApiError)?;

    let graph = generate_graph(&ctx, &ws.root_path, description).await;
    let name = req
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| slug_title(description));

    let wf = repo(&ctx)
        // Agent-generated workflows have no separate instructions source (the
        // description IS the generation prompt); leave instructions empty.
        .create(&wid, &name, description, "", &graph, &user.id)
        .await
        .map_err(ApiError)?;
    Ok(Json(wf))
}

/// Ask the agent for a workflow graph; validate kinds; lay it out. Falls back to
/// a minimal trigger→agent graph when the LLM is unavailable or output is junk.
async fn generate_graph(ctx: &ServerCtx, cwd: &str, description: &str) -> WorkflowGraph {
    let catalog = workflow_engine::node_catalog();
    let kinds = catalog
        .iter()
        .map(|s| {
            format!(
                "- {} (in {}, out {}): {}",
                s.kind, s.inputs, s.outputs, s.description
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = format!(
        "You are building an automation workflow as a directed graph. Available node kinds:\n{kinds}\n\n\
         Produce ONLY a JSON object of shape \
         {{\"nodes\":[{{\"id\":\"n1\",\"kind\":\"<kind>\",\"name\":\"<label>\",\"params\":{{}}}}],\
         \"edges\":[{{\"id\":\"e1\",\"source\":\"n1\",\"target\":\"n2\"}}]}}. \
         Start with a manual_trigger. Use only the listed kinds. Wire nodes left-to-right to \
         accomplish the goal. No prose, no markdown fences.\n\n\
         Engine rules — a graph that violates these will run but silently misbehave:\n\
         - Params are LITERAL values. There is NO template interpolation: never write \
         `{{{{input.x}}}}` or any `{{{{...}}}}` placeholder — it reaches the node as that exact string.\n\
         - Agent nodes receive the run input and prior step results as context files \
         (run-brief.md, repos.json, jira-<KEY>.md, stepN-*.md) in the run context directory; \
         write prompts that tell the agent to read those. Omit `cwd` — agents run in the \
         run's prepared repo worktree.\n\
         - prepare_context: omit `key`; it resolves the Jira key from the run input itself.\n\
         - A condition node's `expression` evaluates on the node's INPUT (its upstream's \
         output), so place it directly after the node whose fields it tests. Edge \
         `condition` strings evaluate on the source node's OUTPUT and need the `output.` \
         prefix, e.g. `output.result == true`.\n\
         - review_run params: {{\"threshold\":80,\"reviewers\":[{{\"lens\":\"<skill>\",\
         \"providers\":[\"claude\",\"codex\"]}}],\"summarizer\":{{\"provider\":\"claude\"}},\
         \"scoring\":{{\"bug\":10,\"warn\":5,\"info\":1}}}} — the flat `providers`/`lenses` \
         form is legacy and runs a single provider.\n\nGoal: {description}"
    );

    let parsed = match ctx
        .orchestrator
        .run_agent(&prompt, cwd, None, std::time::Duration::from_secs(120))
        .await
    {
        Ok(text) => extract_graph(&text),
        Err(e) => {
            tracing::warn!("workflow generate: LLM unavailable: {e}");
            None
        }
    };

    let mut graph = parsed
        .filter(|g: &WorkflowGraph| !g.nodes.is_empty())
        .map(sanitize)
        .unwrap_or_else(|| fallback_graph(description));
    layout(&mut graph);
    graph
}

/// Parse a WorkflowGraph out of possibly-fenced agent text.
fn extract_graph(text: &str) -> Option<WorkflowGraph> {
    let t = text.trim();
    if let Ok(g) = serde_json::from_str::<WorkflowGraph>(t) {
        return Some(g);
    }
    let start = t.find('{')?;
    let mut depth = 0usize;
    for (i, ch) in t[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return serde_json::from_str(&t[start..start + i + 1]).ok();
                }
            }
            _ => {}
        }
    }
    None
}

/// Drop nodes with unknown kinds and edges referencing missing nodes; scrub
/// params/conditions the engine would take literally and silently misrun.
fn sanitize(mut g: WorkflowGraph) -> WorkflowGraph {
    g.nodes.retain(|n| workflow_engine::is_known_kind(&n.kind));
    for n in &mut g.nodes {
        strip_placeholder_params(&mut n.params);
    }
    let ids: std::collections::HashSet<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
    g.edges
        .retain(|e| ids.contains(e.source.as_str()) && ids.contains(e.target.as_str()));
    for e in &mut g.edges {
        // Edge conditions evaluate in `output.*` scope; the LLM habitually emits
        // bare `result == …`, which never matches and silently skips the branch.
        if let Some(c) = &e.condition {
            let t = c.trim();
            if t.starts_with("result") || t.starts_with("value") {
                e.condition = Some(format!("output.{t}"));
            }
        }
    }
    g
}

/// Remove object entries / array items whose string value is a pure `{{…}}`
/// placeholder — the engine has no template interpolation, so such params reach
/// the node verbatim (a literal `{{input.x}}` Jira key, cwd, …). Dropping them
/// falls back to each node's own input-resolution defaults, which is what the
/// placeholder was trying to express. Longer strings that merely embed a
/// placeholder (agent prompts) are kept: agents read the run context and can
/// interpret the intent.
fn strip_placeholder_params(v: &mut Value) {
    fn is_placeholder(val: &Value) -> bool {
        matches!(val, Value::String(s)
            if s.trim().starts_with("{{") && s.trim().ends_with("}}"))
    }
    match v {
        Value::Object(map) => {
            map.retain(|_, val| !is_placeholder(val));
            for val in map.values_mut() {
                strip_placeholder_params(val);
            }
        }
        Value::Array(items) => {
            items.retain(|val| !is_placeholder(val));
            for val in items {
                strip_placeholder_params(val);
            }
        }
        _ => {}
    }
}

/// Minimal always-valid graph when generation fails.
fn fallback_graph(description: &str) -> WorkflowGraph {
    WorkflowGraph {
        nodes: vec![
            WorkflowNode {
                id: "trigger".into(),
                kind: "manual_trigger".into(),
                name: "Start".into(),
                x: 0.0,
                y: 0.0,
                params: Value::Null,
                retry: None,
            },
            WorkflowNode {
                id: "agent".into(),
                kind: "agent_prompt".into(),
                name: "Agent".into(),
                x: 0.0,
                y: 0.0,
                params: serde_json::json!({ "prompt": description }),
                retry: None,
            },
        ],
        edges: vec![WorkflowEdge {
            id: "e1".into(),
            source: "trigger".into(),
            target: "agent".into(),
            condition: None,
        }],
    }
}

/// Assign positions by topological layer so the graph reads left-to-right.
fn layout(g: &mut WorkflowGraph) {
    use std::collections::HashMap;
    let mut layer: HashMap<String, usize> = HashMap::new();
    for n in &g.nodes {
        layer.insert(n.id.clone(), 0);
    }
    // Relax layers a few passes (graph is small).
    for _ in 0..g.nodes.len() {
        for e in &g.edges {
            let s = *layer.get(&e.source).unwrap_or(&0);
            let t = layer.entry(e.target.clone()).or_insert(0);
            if *t <= s {
                *t = s + 1;
            }
        }
    }
    let mut per_layer: HashMap<usize, f64> = HashMap::new();
    for n in g.nodes.iter_mut() {
        let l = *layer.get(&n.id).unwrap_or(&0);
        let row = per_layer.entry(l).or_insert(0.0);
        n.x = l as f64 * 280.0 + 40.0;
        n.y = *row * 130.0 + 40.0;
        *row += 1.0;
    }
}

fn slug_title(description: &str) -> String {
    let words: Vec<&str> = description.split_whitespace().take(6).collect();
    let s = words.join(" ");
    if s.len() > 60 {
        format!(
            "{}…",
            &s[..s
                .char_indices()
                .take(57)
                .last()
                .map(|(i, _)| i)
                .unwrap_or(57)]
        )
    } else {
        s
    }
}

/// Fold a per-run `review_mode` into the run input so the engine reads it
/// from `RunEnv.run_input` (node inputs lose keys hop by hop). `Null`
/// becomes an object; a non-object input is a 400.
pub(crate) fn seed_review_mode(
    input: Value,
    review_mode: Option<&str>,
) -> otto_core::Result<Value> {
    let Some(raw) = review_mode else {
        return Ok(input);
    };
    let mode = otto_core::domain::ReviewMode::parse(raw).ok_or_else(|| {
        otto_core::Error::Invalid("review_mode must be \"fan_out\" or \"orchestrator\"".into())
    })?;
    match input {
        Value::Null => Ok(json!({ "review_mode": mode.as_str() })),
        Value::Object(mut m) => {
            m.insert("review_mode".into(), json!(mode.as_str()));
            Ok(Value::Object(m))
        }
        _ => Err(otto_core::Error::Invalid(
            "input must be a JSON object when review_mode is set".into(),
        )),
    }
}

/// `POST /workflows/{id}/run`
pub async fn run_workflow(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<RunWorkflowReq>,
) -> ApiResult<Json<WorkflowRun>> {
    let wf = repo(&ctx).get(&id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &wf.workspace_id, WorkspaceRole::Editor).await?;
    crate::workflow_validation::ensure_valid(&wf.graph).map_err(ApiError)?;
    if req
        .start_node
        .as_ref()
        .is_some_and(|id| !wf.graph.nodes.iter().any(|node| &node.id == id))
        || (req.only_node && req.start_node.is_none())
    {
        return Err(ApiError(Error::Invalid(
            "Select an existing start step for a partial run".into(),
        )));
    }
    let ws = ctx
        .workspaces
        .get(&wf.workspace_id)
        .await
        .map_err(ApiError)?;

    let input = seed_review_mode(req.input.unwrap_or(Value::Null), req.review_mode.as_deref())
        .map_err(ApiError)?;
    let run = repo(&ctx)
        .create_run(&wf.id, &wf.workspace_id, &input, Some(&user.id))
        .await
        .map_err(ApiError)?;

    let scope = otto_core::workflows::RunScope {
        continue_unfinished: false,
        start_node: req.start_node.clone(),
        only_node: req.only_node,
        adopt_start: false,
    };
    let scope_json =
        serde_json::to_string(&scope).map_err(|e| ApiError(Error::Internal(e.to_string())))?;
    repo(&ctx)
        .set_run_resume_scope(&run.id, Some(&scope_json))
        .await
        .map_err(ApiError)?;

    // Execute in the background (gated to the daemon-wide parallel-run cap;
    // beyond it the run queues as this `pending` row). The UI polls
    // GET /workflow-runs/{id}.
    workflow_engine::spawn_run(ctx.clone(), ws, wf, run.id.clone(), input, scope, None);

    Ok(Json(run))
}

/// `POST /workflow-runs/{id}/retry-node` — re-run ONE errored step of a
/// FINISHED run, in place, without repeating the (possibly hours-long) earlier
/// steps. The run row is reopened and re-entered with `start_node=node,
/// only_node=true`; every other node ADOPTS its prior state (status/output/
/// sessions), the retried node starts fresh, and the run's final status is
/// recomputed over all of them. The run's context dir + provisioned
/// `otto-wf/<run_id>` worktrees are keyed by run id, so the retried step sees
/// the same branch/worktree the original attempt worked on
/// (`worktree_add_if_absent` re-attaches a surviving branch). Only `error`
/// steps of non-active runs are retryable — retrying a successful/skipped
/// step would replay side effects that already happened.
pub async fn retry_run_node(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<RetryRunNodeReq>,
) -> ApiResult<Json<WorkflowRun>> {
    let run = repo(&ctx).get_run(&id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &run.workspace_id, WorkspaceRole::Editor).await?;
    if matches!(run.status, RunStatus::Pending | RunStatus::Running) {
        return Err(ApiError(Error::Conflict(
            "run is still active — cancel it first or wait for it to finish".into(),
        )));
    }
    // Canceled, but its old driver hasn't noticed yet (it polls; mid retry
    // backoff that took up to a minute): a retry now would run next to it.
    if workflow_engine::driver_alive(&id) {
        return Err(ApiError(Error::Conflict(
            "the run is still stopping — try again in a few seconds".into(),
        )));
    }
    let node_id = req.node_id.trim();
    let target = run
        .nodes
        .iter()
        .find(|n| n.node_id == node_id)
        .ok_or_else(|| ApiError(Error::NotFound(format!("run has no node '{node_id}'"))))?;
    // Single-step retry is for ERRORED steps only (re-executing a successful
    // step alone would replay side effects out of context). "Re-run from
    // here" re-executes the whole downstream flow, so any settled entry step
    // is a valid starting point.
    use otto_core::workflows::NodeStatus;
    let ok_entry = if req.include_downstream {
        !matches!(target.status, NodeStatus::Pending | NodeStatus::Running)
    } else {
        target.status == NodeStatus::Error
    };
    if !ok_entry {
        return Err(ApiError(Error::Invalid(format!(
            "step '{node_id}' ({:?}) can't be retried — only an errored step, or any settled step with include_downstream",
            target.status
        ))));
    }
    let wf = repo(&ctx)
        .definition_for_run(&run)
        .await
        .map_err(ApiError)?;
    let ws = ctx
        .workspaces
        .get(&wf.workspace_id)
        .await
        .map_err(ApiError)?;
    let retry_nodes: Vec<String> = if req.include_downstream {
        workflow_engine::descendants_inclusive(&wf.graph, node_id)
            .into_iter()
            .collect()
    } else {
        vec![node_id.to_string()]
    };
    let scope = otto_core::workflows::RunScope {
        continue_unfinished: false,
        start_node: Some(node_id.to_string()),
        only_node: !req.include_downstream,
        adopt_start: false,
    };
    repo(&ctx)
        .prepare_retry(&id, &retry_nodes, req.include_downstream, &scope)
        .await
        .map_err(ApiError)?;
    workflow_engine::spawn_run(
        ctx.clone(),
        ws,
        wf,
        run.id.clone(),
        run.input.clone(),
        scope,
        Some(run.nodes.clone()),
    );
    let run = repo(&ctx).get_run(&id).await.map_err(ApiError)?;
    Ok(Json(run))
}

/// `POST /workflow-runs/{id}/cancel` — request a running workflow to stop. Takes
/// effect at the next node boundary (a node already executing finishes first).
pub async fn cancel_run(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<WorkflowRun>> {
    let run = repo(&ctx).get_run(&id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &run.workspace_id, WorkspaceRole::Editor).await?;
    if matches!(run.status, RunStatus::Pending | RunStatus::Running) {
        // Status-only and conditional: never writes back the node snapshot
        // read above (a run that finished in between kept a step "running"
        // forever and had its success overwritten), and a no-op once the run
        // has settled.
        let rev = repo(&ctx).request_cancel(&id).await.map_err(ApiError)?;
        // Announce the cancel right away (the engine re-emits once its current
        // node reaches a boundary and it marks the remaining nodes skipped).
        if let Some(rev) = rev {
            let _ = ctx.events.send(Event::WorkflowRunUpdated {
                workspace_id: run.workspace_id.clone(),
                run_id: id.clone(),
                status: "canceled".into(),
                node_id: None,
                rev,
                node: None,
                nodes_done: 0,
                nodes_total: 0,
                waiting_approval: false,
            });
        }
    }
    repo(&ctx).get_run(&id).await.map(Json).map_err(ApiError)
}

#[derive(Default, Deserialize)]
pub struct RunListQuery {
    #[serde(default)]
    summary: bool,
}
/// `GET /workflows/{id}/runs?summary=true` — legacy default keeps full rows.
pub async fn list_runs(
    Path(id): Path<Id>,
    Query(q): Query<RunListQuery>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let wf = repo(&ctx).get(&id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &wf.workspace_id, WorkspaceRole::Viewer).await?;
    let rows = if q.summary {
        serde_json::to_value(
            otto_state::workflow_progress::run_summaries(&ctx.pool, &id)
                .await
                .map_err(ApiError)?,
        )
    } else {
        serde_json::to_value(repo(&ctx).list_runs(&id).await.map_err(ApiError)?)
    };
    Ok(Json(
        rows.map_err(|e| ApiError(Error::Internal(e.to_string())))?,
    ))
}

/// Fill the derived `context_dir` field: the run's context directory
/// (`<data_dir>/workflow-context/<run_id>/`) when it exists on disk. Kept out
/// of the DB — run_id derives it — and out of list endpoints (one stat per
/// run there is wasted work; the run VIEW is where the file browser lives).
fn with_context_dir(ctx: &ServerCtx, mut run: WorkflowRun) -> WorkflowRun {
    // Run ids are daemon-generated ULIDs, but confine the join anyway so a
    // hostile id can never stat outside the workflow-context tree.
    let Some(dir) = otto_core::paths::confine_join(&ctx.data_dir.join("workflow-context"), &run.id)
    else {
        return run;
    };
    if dir.is_dir() {
        run.context_dir = Some(dir.to_string_lossy().into_owned());
    }
    run
}

/// `GET /workflow-runs/{id}`
pub async fn get_run(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<WorkflowRun>> {
    let mut run = repo(&ctx).get_run(&id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &run.workspace_id, WorkspaceRole::Viewer).await?;
    run.checkpoints = repo(&ctx).checkpoints(&id).await.map_err(ApiError)?;
    Ok(Json(with_context_dir(&ctx, run)))
}

/// `GET /workspaces/{wid}/workflow-runs/active` — in-flight runs (pending|running)
/// across the workspace, for the "Running" sidebar list. Refreshed by the UI on
/// each `workflow_run_updated` WS event.
pub async fn list_active_runs(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<ActiveWorkflowRun>>> {
    crate::auth::require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    Ok(Json(
        repo(&ctx).list_active_runs(&wid).await.map_err(ApiError)?,
    ))
}

// ---------------------------------------------------------------------------
// Example templates (game pipelines: agent design + engine scaffold)
// ---------------------------------------------------------------------------

/// `GET /workflows/templates`
pub async fn list_templates() -> Json<Vec<WorkflowTemplate>> {
    Json(all_templates())
}

/// All built-in templates: the orchestrator examples first, then game pipelines.
fn all_templates() -> Vec<WorkflowTemplate> {
    let mut v = flow_templates();
    v.extend(game_templates());
    v
}

/// `POST /workspaces/{wid}/workflows/from-template`
pub async fn create_from_template(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<FromTemplateReq>,
) -> ApiResult<Json<Workflow>> {
    crate::auth::require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let tpl = all_templates()
        .into_iter()
        .find(|t| t.id == req.template_id)
        .ok_or_else(|| ApiError(Error::NotFound("template".into())))?;
    let name = req
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| tpl.name.clone());
    let wf = repo(&ctx)
        .create(
            &wid,
            &name,
            &tpl.description,
            &tpl.instructions,
            &tpl.graph,
            &user.id,
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(wf))
}

/// Build the three game-pipeline templates. Each chains:
/// trigger → agent (design rules) → game_engine (kind) → verifier.
fn game_templates() -> Vec<WorkflowTemplate> {
    fn pipeline(game: &str, design: &str) -> WorkflowGraph {
        let node =
            |id: &str, kind: &str, name: &str, x: f64, params: serde_json::Value| WorkflowNode {
                id: id.into(),
                kind: kind.into(),
                name: name.into(),
                x,
                y: 70.0,
                params,
                retry: None,
            };
        let edge = |s: &str, t: &str| WorkflowEdge {
            id: format!("{s}-{t}"),
            source: s.into(),
            target: t.into(),
            condition: None,
        };
        WorkflowGraph {
            nodes: vec![
                node(
                    "trigger",
                    "manual_trigger",
                    "Start",
                    40.0,
                    serde_json::Value::Null,
                ),
                node(
                    "design",
                    "agent_prompt",
                    "Design game",
                    320.0,
                    serde_json::json!({ "prompt": design }),
                ),
                node(
                    "game",
                    "game_engine",
                    "Build game",
                    600.0,
                    serde_json::json!({ "game": game }),
                ),
                node(
                    "verify",
                    "verifier",
                    "Verify",
                    880.0,
                    serde_json::Value::Null,
                ),
            ],
            edges: vec![
                edge("trigger", "design"),
                edge("design", "game"),
                edge("game", "verify"),
            ],
        }
    }

    vec![
        WorkflowTemplate {
            id: "game-slots".into(),
            name: "Slots game".into(),
            description: "5×3 slot machine: agent designs the paytable & RTP, then the engine assembles and verifies it.".into(),
            instructions: String::new(),
            icon: "grid".into(),
            graph: pipeline(
                "slots",
                "Design a 5x3 slot game for the given theme: reel symbols, paytable, win lines, RTP ~96%, volatility, and a bonus feature. Return a structured spec.",
            ),
        },
        WorkflowTemplate {
            id: "game-crash".into(),
            name: "Crash game (Aviator style)".into(),
            description: "Aviator-style crash game: agent designs the multiplier curve & provably-fair RNG, then build & verify.".into(),
            instructions: String::new(),
            icon: "zap".into(),
            graph: pipeline(
                "crash",
                "Design an aviator-style crash game: the multiplier growth curve, a provably-fair RNG seed/commit scheme, auto-cashout rules, house edge ~3%, and max multiplier. Return a structured spec.",
            ),
        },
        WorkflowTemplate {
            id: "game-scratch".into(),
            name: "Scratch card".into(),
            description: "Scratch-card game: agent designs prize tiers & win probabilities, then build & verify.".into(),
            instructions: String::new(),
            icon: "ticket".into(),
            graph: pipeline(
                "scratch",
                "Design a scratch-card game: prize tiers and their win probabilities, panel layout, reveal mechanic, and RTP ~95%. Return a structured spec.",
            ),
        },
    ]
}

/// The orchestrator example templates: end-to-end flows that exercise the wired
/// nodes (product/review), control flow (loop + edge conditions), goals-scored
/// review, human approval, and a drafted PR. They expect the run **input** to
/// carry `repo_id` (and optionally `base`, `story_id`, `goals`) — a Slack
/// `Action: Workflow` message or the Run dialog supplies these; the first node
/// consolidates the Jira ticket / working dir / relevant info into a brief.
fn flow_templates() -> Vec<WorkflowTemplate> {
    let node = |id: &str, kind: &str, name: &str, x: f64, params: serde_json::Value| WorkflowNode {
        id: id.into(),
        kind: kind.into(),
        name: name.into(),
        x,
        y: 80.0,
        params,
        retry: None,
    };
    let edge = |s: &str, t: &str| WorkflowEdge {
        id: format!("{s}-{t}"),
        source: s.into(),
        target: t.into(),
        condition: None,
    };
    // A conditional edge: the target only runs when `cond` holds against the
    // source's output (ctx = { output, input, node, run }). Used to gate the PR
    // step on the review having passed.
    let edge_if = |s: &str, t: &str, cond: &str| WorkflowEdge {
        id: format!("{s}-{t}"),
        source: s.into(),
        target: t.into(),
        condition: Some(cond.into()),
    };
    let prepare = |goal: &str, x: f64| {
        node(
            "prepare",
            "agent_prompt",
            "Prepare relevant info",
            x,
            json!({ "prompt": format!(
                "{goal}\n\nFrom the input data, read the Jira ticket (if any), the working \
                 directory, and every 'relevant_info' path, plus the message and goals. Search \
                 the codebase for references. Produce a single consolidated brief: scope, where \
                 the code lives, acceptance criteria, and the goals to satisfy. This brief is \
                 passed to the following steps."
            ) }),
        )
    };
    // Loop body — REVIEW first, then FIX (design §E): the work already exists
    // from the upstream implement step, so each iteration reviews it, then fixes
    // the findings, repeating until the review passes. The reviewers mirror PR
    // review (design §F): per-lens provider sets + a summarizer. Scoring is a
    // generic, configurable severity→deduction guideline (design §G). `reviewers`
    // is a JSON array of { lens, providers[] (, instructions?) }.
    let fix_review_loop = |max: u64, threshold: u64, reviewers: serde_json::Value, x: f64| {
        node(
            "iterate",
            "loop",
            "Review → fix until passing",
            x,
            json!({
                "max_iterations": max,
                // Pass iff the review step (by name) cleared the threshold.
                "until": "steps.review.passed == true",
                "steps": [
                    { "kind": "review_run", "name": "review", "params": {
                        "threshold": threshold,
                        "reviewers": reviewers,
                        "summarizer": { "provider": "claude" },
                        // Generic scoring guideline — percent deducted per OPEN
                        // finding by severity (bug=critical, warn=high, info=low).
                        "scoring": { "bug": 10, "warn": 5, "info": 1 }
                    }},
                    { "kind": "agent_prompt", "name": "fix", "params": {
                        "prompt": "You are given the latest code review in the input (its findings, \
                                   score, and `passed`). If `passed` is true or there are no \
                                   findings, make NO changes. Otherwise address EVERY finding and \
                                   make all tests pass while satisfying the goals."
                    }}
                ]
            }),
        )
    };
    // A separate, terminal "offer improvements" block (design §I): after the
    // work + review loop, reflect on the session and OFFER skill/memory
    // improvements — queued for approval, never auto-applied. Placed BELOW the
    // post-loop x (y=300) because it shares that x with the `git_pr` node — the
    // two are PARALLEL branches off the loop (PR on pass, improvements always), so
    // they must not render stacked on top of each other (which hid the PR node).
    let offer_improvements = |x: f64| WorkflowNode {
        id: "improve".into(),
        kind: "self_improve".into(),
        name: "Offer improvements".into(),
        x,
        y: 300.0,
        params: Value::Null,
        retry: None,
    };

    vec![
        // 1) Writing tests for a story.
        WorkflowTemplate {
            id: "write-tests".into(),
            name: "Write tests for a story".into(),
            description: "Read the story & search references → write tests → multi-agent \
                          review-iterate until the score passes the threshold → open the PR \
                          automatically on pass (the review IS the approval). Provide repo_id \
                          (and base) in the run input."
                .into(),
            instructions: String::new(),
            icon: "check-square".into(),
            graph: WorkflowGraph {
                nodes: vec![
                    node("trigger", "manual_trigger", "Start", 40.0, Value::Null),
                    prepare("You are preparing context to WRITE TESTS.", 300.0),
                    node("implement", "agent_prompt", "Write tests", 600.0, json!({
                        "prompt": "Using the brief, implement comprehensive tests (happy path, \
                                   meaningful validations, realistic errors). Run the suite and make them pass."
                    })),
                    fix_review_loop(
                        3,
                        80,
                        json!([
                            { "lens": "correctness-review", "providers": ["claude", "codex"] },
                            { "lens": "test-review", "providers": ["claude"] }
                        ]),
                        900.0,
                    ),
                    node("pr", "git_pr", "Open PR (on pass)", 1300.0, json!({ "open": true })),
                    offer_improvements(1300.0),
                ],
                edges: vec![
                    edge("trigger", "prepare"),
                    edge("prepare", "implement"),
                    edge("implement", "iterate"),
                    // Open the PR only when the review→fix loop passed.
                    edge_if("iterate", "pr", "output.satisfied == true"),
                    // Offer improvements after the loop, pass or fail.
                    edge("iterate", "improve"),
                ],
            },
        },
        // 2) Implementing a feature from a story.
        WorkflowTemplate {
            id: "implement-feature".into(),
            name: "Implement a feature from a story".into(),
            description: "Analyze the story → search references → implement → tests → multi-agent \
                          review-iterate until passing → open the PR automatically on pass (the \
                          review IS the approval). Provide repo_id and story_id in the run input."
                .into(),
            instructions: String::new(),
            icon: "command".into(),
            graph: WorkflowGraph {
                nodes: vec![
                    node("trigger", "manual_trigger", "Start", 40.0, Value::Null),
                    node("analyze", "product_analyze", "Analyze story", 300.0, Value::Null),
                    prepare("You are preparing context to IMPLEMENT a feature.", 600.0),
                    node("implement", "agent_prompt", "Implement", 900.0, json!({
                        "prompt": "Using the analysis + brief, implement the feature and its tests. \
                                   Run the suite and make it pass."
                    })),
                    fix_review_loop(
                        4,
                        80,
                        json!([
                            { "lens": "correctness-review", "providers": ["claude", "codex"] },
                            { "lens": "security-review", "providers": ["codex"] },
                            { "lens": "test-review", "providers": ["claude"] }
                        ]),
                        1200.0,
                    ),
                    node("pr", "git_pr", "Open PR (on pass)", 1600.0, json!({ "open": true })),
                    offer_improvements(1600.0),
                ],
                edges: vec![
                    edge("trigger", "analyze"),
                    edge("analyze", "prepare"),
                    edge("prepare", "implement"),
                    edge("implement", "iterate"),
                    // Open the PR only when the review→fix loop passed.
                    edge_if("iterate", "pr", "output.satisfied == true"),
                    // Offer improvements after the loop, pass or fail.
                    edge("iterate", "improve"),
                ],
            },
        },
        // 3) PO discovery → diagram → review → refine → RFC/Jira.
        WorkflowTemplate {
            id: "po-lifecycle".into(),
            name: "PO discovery → RFC/Jira".into(),
            description: "Discovery → diagram → review → refine → publication preview → human approval → publish. \
                          Before running, configure story_id on Refine and Preview, and account plus Confluence space \
                          (or Jira project) on Preview. Publication only follows approval of that exact preview."
                .into(),
            instructions: String::new(),
            icon: "compass".into(),
            graph: WorkflowGraph {
                nodes: vec![
                    node("trigger", "manual_trigger", "Start", 40.0, Value::Null),
                    node("discovery", "agent_prompt", "Discovery draft", 300.0, json!({
                        "prompt": "Expand the idea/message in the input into a structured product \
                                   draft: problem, target users, value, scope, out-of-scope, risks, \
                                   and open questions."
                    })),
                    node("diagram", "canvas", "Diagram", 600.0, json!({
                        "prompt": "Diagram the proposed solution flow described above.",
                        "mode": "mermaid"
                    })),
                    node("review1", "human_approval", "Review discovery + diagram", 900.0, json!({
                        "prompt": "Review the discovery draft and the diagram."
                    })),
                    node("refine", "product_rewrite", "Refine + attach info", 1200.0, json!({"persist":true})),
                    node("preview", "product_publish", "Preview — configure account/destination", 1500.0, json!({"kind":"rfc","dry_run":true})),
                    node("review2", "human_approval", "Review publication content and destination", 1800.0, json!({
                        "prompt": "Review the full publication title, content, account and destination. Approve to publish this snapshot."
                    })),
                    node("publish", "product_publish", "Publish approved snapshot", 2100.0, json!({
                        "dry_run": false
                    })),
                ],
                edges: vec![
                    edge("trigger", "discovery"),
                    edge("discovery", "diagram"),
                    edge("diagram", "review1"),
                    edge("review1", "refine"),
                    edge("refine", "preview"),
                    edge("preview", "review2"),
                    edge("review2", "publish"),
                ],
            },
        },
        // 4) UI test authoring: story → app-fetched Jira context → write UI
        // tests → review-fix loop → PR on pass → final report (both branches).
        WorkflowTemplate {
            id: "ui-test-authoring".into(),
            name: "UI test authoring".into(),
            description: "Story → app-fetched Jira context → write UI tests → review-fix loop \
                          until green → PR on pass → final report. Bind a Slack channel or run \
                          with a prompt."
                .into(),
            instructions: "# Standing instructions — UI test authoring\n\
                - Tests are Playwright specs; follow the repo's existing spec layout and naming.\n\
                - Never sleep-poll; use Playwright auto-waiting and web-first assertions.\n\
                - Selectors: prefer role/test-id selectors over CSS/text.\n\
                - Each spec must be independently runnable and idempotent.\n\
                - Run ONLY the specs you created or changed — never the full suite.\n\
                - If a jira-<KEY>.md exists in the context dir, it is the requirements source of truth.\n\
                - Write your step handoff file before finishing (path given in your prompt)."
                .into(),
            icon: "check-square".into(),
            graph: WorkflowGraph {
                nodes: vec![
                    node("trigger", "manual_trigger", "Start", 40.0, Value::Null),
                    node("prep", "prepare_context", "Prepare relevant data", 300.0, json!({
                        "prompt": "Analyze the story and the codebase. If a jira-<KEY>.md file \
                                   exists in the context directory it is the source of truth for \
                                   requirements — read it fully (description AND comments). \
                                   Produce a test plan: what to cover, where the tests live, exact \
                                   conventions to follow per instructions.md."
                    })),
                    node("implement", "agent_prompt", "Write UI tests", 600.0, json!({
                        "prompt": "Write the UI tests per the test plan, following instructions.md \
                                   by the letter. Run only the specs you created/changed and make \
                                   them pass."
                    })),
                    fix_review_loop(
                        3,
                        80,
                        json!([
                            { "lens": "correctness-review", "providers": ["claude"] },
                            { "lens": "test-review", "providers": ["claude"] }
                        ]),
                        900.0,
                    ),
                    node("pr", "git_pr", "Open PR (on pass)", 1300.0, json!({ "open": true })),
                    node("report", "agent_prompt", "Final report", 1600.0, json!({
                        "prompt": "Write the final run report: what was requested, what was \
                                   implemented (files/specs), the final suite result, and PR links \
                                   from the input if present. If the review loop did not pass, \
                                   state exactly what is still failing. This report is the run's \
                                   final output."
                    })),
                    offer_improvements(1300.0),
                ],
                edges: vec![
                    edge("trigger", "prep"),
                    edge("prep", "implement"),
                    edge("implement", "iterate"),
                    // Open the PR only when the review→fix loop passed.
                    edge_if("iterate", "pr", "output.satisfied == true"),
                    // `report` MUST remain the run's LAST content-bearing step: on
                    // success it becomes final-output.md (workflow_context.rs's
                    // `content_step` is overwritten by every non-utility step that
                    // runs, last one wins — see `write_final_output`). This edge is
                    // what forces `report` to execution-order after `improve`
                    // (`self_improve` is content-bearing too, and both `report` and
                    // `improve` are otherwise parallel branches off `iterate`).
                    // Deleting this edge would silently make the self-improve offer
                    // the run's deliverable instead of the report.
                    edge("pr", "report"),
                    // The final report always runs, even when the loop didn't pass.
                    edge("iterate", "report"),
                    // Offer improvements after the loop, pass or fail.
                    edge("iterate", "improve"),
                ],
            },
        },
        // 5) API acceptance test authoring: same shape as UI test authoring,
        // targeting the acceptance-test framework's Gateway/ServiceLocator layers.
        WorkflowTemplate {
            id: "api-acceptance-test-authoring".into(),
            name: "API acceptance test authoring".into(),
            description: "Story → app-fetched Jira context → write API acceptance tests → \
                          review-fix loop until green → PR on pass → final report. Bind a Slack \
                          channel or run with a prompt."
                .into(),
            instructions: "# Standing instructions — API acceptance test authoring\n\
                - Follow the acceptance-test framework's two layers strictly: Gateway APIs for \
                player-behavior flows, ServiceLocator for internal service features.\n\
                - Keep API-call code and validation/assertion code in their designated layers.\n\
                - Reuse existing player-creation / balance helpers; never duplicate setup utilities.\n\
                - Tests must be independently runnable and leave no dirty state.\n\
                - Run ONLY the tests you created or changed — never the full suite.\n\
                - If a jira-<KEY>.md exists in the context dir, it is the requirements source of truth.\n\
                - Write your step handoff file before finishing (path given in your prompt)."
                .into(),
            icon: "send".into(),
            graph: WorkflowGraph {
                nodes: vec![
                    node("trigger", "manual_trigger", "Start", 40.0, Value::Null),
                    node("prep", "prepare_context", "Prepare relevant data", 300.0, json!({
                        "prompt": "Analyze the story and the codebase. If a jira-<KEY>.md file \
                                   exists in the context directory it is the source of truth for \
                                   requirements — read it fully (description AND comments). \
                                   Produce a test plan: what to cover, where the tests live, exact \
                                   conventions to follow per instructions.md."
                    })),
                    node("implement", "agent_prompt", "Write API acceptance tests", 600.0, json!({
                        "prompt": "Write the API acceptance tests per the test plan, following \
                                   instructions.md by the letter. Run only the specs you \
                                   created/changed and make them pass."
                    })),
                    fix_review_loop(
                        3,
                        80,
                        json!([
                            { "lens": "correctness-review", "providers": ["claude"] },
                            { "lens": "test-review", "providers": ["claude"] }
                        ]),
                        900.0,
                    ),
                    node("pr", "git_pr", "Open PR (on pass)", 1300.0, json!({ "open": true })),
                    node("report", "agent_prompt", "Final report", 1600.0, json!({
                        "prompt": "Write the final run report: what was requested, what was \
                                   implemented (files/specs), the final suite result, and PR links \
                                   from the input if present. If the review loop did not pass, \
                                   state exactly what is still failing. This report is the run's \
                                   final output."
                    })),
                    offer_improvements(1300.0),
                ],
                edges: vec![
                    edge("trigger", "prep"),
                    edge("prep", "implement"),
                    edge("implement", "iterate"),
                    // Open the PR only when the review→fix loop passed.
                    edge_if("iterate", "pr", "output.satisfied == true"),
                    // `report` MUST remain the run's LAST content-bearing step: on
                    // success it becomes final-output.md (workflow_context.rs's
                    // `content_step` is overwritten by every non-utility step that
                    // runs, last one wins — see `write_final_output`). This edge is
                    // what forces `report` to execution-order after `improve`
                    // (`self_improve` is content-bearing too, and both `report` and
                    // `improve` are otherwise parallel branches off `iterate`).
                    // Deleting this edge would silently make the self-improve offer
                    // the run's deliverable instead of the report.
                    edge("pr", "report"),
                    // The final report always runs, even when the loop didn't pass.
                    edge("iterate", "report"),
                    // Offer improvements after the loop, pass or fail.
                    edge("iterate", "improve"),
                ],
            },
        },
    ]
}

// ---------------------------------------------------------------------------
// Trigger CRUD: GET/POST/PATCH/DELETE on workflow triggers
// ---------------------------------------------------------------------------

fn triggers(ctx: &ServerCtx) -> TriggersRepo {
    TriggersRepo::new(ctx.pool.clone())
}

#[derive(Debug, Deserialize)]
pub struct ValidateGraphReq {
    pub graph: Option<WorkflowGraph>,
}

/// Validate a working copy without persisting it or executing any node.
pub async fn validate_graph(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<ValidateGraphReq>,
) -> ApiResult<Json<Value>> {
    let wf = repo(&ctx).get(&id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &wf.workspace_id, WorkspaceRole::Viewer).await?;
    let issues = crate::workflow_validation::validate(req.graph.as_ref().unwrap_or(&wf.graph));
    Ok(Json(json!({"valid": issues.is_empty(), "issues": issues})))
}

/// Preview uses the scheduler's cadence/timezone evaluator; never advances the
/// cursor or creates a run. Non-schedule previews only validate the spec.
pub async fn preview_trigger(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateTriggerReq>,
) -> ApiResult<Json<Value>> {
    let wf = repo(&ctx).get(&id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &wf.workspace_id, WorkspaceRole::Viewer).await?;
    validate_trigger_spec(&req.kind, &req.spec)?;
    let mut next = Vec::new();
    if req.kind == "schedule" {
        let tz = crate::cadence::task_tz(
            req.spec
                .get("timezone")
                .and_then(Value::as_str)
                .unwrap_or("UTC"),
        );
        let mut cursor = chrono::Utc::now();
        for _ in 0..5 {
            let Some(at) = crate::cadence::next_run(&req.spec, cursor, tz) else {
                break;
            };
            next.push(at.to_rfc3339());
            cursor = at;
        }
    }
    Ok(Json(json!({"next_fire_times": next, "kind": req.kind})))
}

/// `GET /workflows/{id}/triggers`
pub async fn list_triggers(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<WorkflowTrigger>>> {
    let wf = repo(&ctx).get(&id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &wf.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(triggers(&ctx).list(&id).await.map_err(ApiError)?))
}

#[derive(Debug, Deserialize)]
pub struct CreateTriggerReq {
    pub kind: String,
    #[serde(default)]
    pub spec: Value,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// `POST /workflows/{id}/triggers`
pub async fn create_trigger(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateTriggerReq>,
) -> ApiResult<Json<WorkflowTrigger>> {
    let wf = repo(&ctx).get(&id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &wf.workspace_id, WorkspaceRole::Editor).await?;
    if !matches!(req.kind.as_str(), "schedule" | "webhook" | "event" | "chat") {
        return Err(ApiError(Error::Invalid(
            "trigger kind must be 'schedule', 'webhook', 'event', or 'chat'".into(),
        )));
    }
    // For webhook triggers, auto-generate a cryptographically random token if
    // the caller didn't supply one.  Always normalise spec to a JSON object.
    let mut spec = match req.spec {
        Value::Object(m) => Value::Object(m),
        _ => Value::Object(Default::default()),
    };
    if req.kind == "webhook" && spec.get("token").and_then(Value::as_str).is_none() {
        let token = generate_webhook_token();
        if let Value::Object(obj) = &mut spec {
            obj.insert("token".into(), Value::String(token));
        }
    }
    validate_trigger_spec(&req.kind, &spec)?;
    let t = triggers(&ctx)
        .create(NewWorkflowTrigger {
            workflow_id: id.clone(),
            kind: req.kind,
            spec,
            enabled: req.enabled,
        })
        .await
        .map_err(ApiError)?;
    Ok(Json(t))
}

/// Produce a 32-byte URL-safe random token for webhook triggers.
fn generate_webhook_token() -> String {
    use std::fmt::Write;
    let bytes: Vec<u8> = (0..32).map(|_| rand::random::<u8>()).collect();
    let mut s = String::with_capacity(64);
    for b in bytes {
        write!(s, "{b:02x}").unwrap();
    }
    s
}

#[derive(Debug, Deserialize)]
pub struct UpdateTriggerReq {
    pub spec: Option<Value>,
    pub enabled: Option<bool>,
}

/// `PATCH /workflow-triggers/{id}`
pub async fn update_trigger(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpdateTriggerReq>,
) -> ApiResult<Json<WorkflowTrigger>> {
    let t = triggers(&ctx).get(&id).await.map_err(ApiError)?;
    let wf = repo(&ctx).get(&t.workflow_id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &wf.workspace_id, WorkspaceRole::Editor).await?;
    if let Some(spec) = &req.spec {
        validate_trigger_spec(&t.kind, spec)?;
    }
    let updated = triggers(&ctx)
        .update(&id, req.spec, req.enabled)
        .await
        .map_err(ApiError)?;
    Ok(Json(updated))
}

/// Validate a trigger spec at WRITE time so misconfigurations fail loudly
/// instead of silently never firing:
/// - `schedule`: cadence/at/weekday/cron validated by the shared cadence
///   engine (an unknown cadence like "monthly" previously saved fine and then
///   `is_due` returned false forever, with no error anywhere).
/// - `event`: `event_kind` must be a kind the listener actually fires, and
///   never `workflow_run_updated` (the engine emits it per node transition —
///   triggering on it is a recursive run explosion).
fn validate_trigger_spec(kind: &str, spec: &Value) -> Result<(), ApiError> {
    if !matches!(kind, "schedule" | "event" | "webhook" | "chat") {
        return Err(ApiError(Error::Invalid("unsupported trigger kind".into())));
    }
    if let Some(timezone) = spec.get("timezone").and_then(Value::as_str) {
        if timezone.parse::<chrono_tz::Tz>().is_err() {
            return Err(ApiError(Error::Invalid(format!(
                "Unknown IANA timezone '{timezone}'"
            ))));
        }
    }
    if let Some(channel) = spec
        .get("result_channel")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        if !matches!(channel, "slack" | "telegram")
            || spec
                .get("result_chat")
                .and_then(Value::as_str)
                .is_none_or(|s| s.trim().is_empty())
        {
            return Err(ApiError(Error::Invalid(
                "Result delivery needs slack|telegram and a chat/channel ID".into(),
            )));
        }
    }
    if let Some(url) = spec
        .get("result_webhook")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        if reqwest::Url::parse(url)
            .ok()
            .is_none_or(|url| !matches!(url.scheme(), "http" | "https") || url.host_str().is_none())
        {
            return Err(ApiError(Error::Invalid(
                "Result webhook must be an HTTP(S) URL".into(),
            )));
        }
    }
    match kind {
        "schedule" => crate::cadence::validate(spec).map_err(ApiError),
        "event" => {
            const FIREABLE: &[&str] = &[
                "review_changed",
                "budget_exceeded",
                "product_changed",
                "swarm_status",
                "improvement_run_finished",
                "insight_ready",
            ];
            let ek = spec.get("event_kind").and_then(Value::as_str).unwrap_or("");
            if ek == "workflow_run_updated" {
                return Err(ApiError(Error::Invalid(
                    "event_kind 'workflow_run_updated' is not allowed: the engine emits it on \
                     every node transition, so triggering on it would recursively spawn runs"
                        .into(),
                )));
            }
            if !FIREABLE.contains(&ek) {
                return Err(ApiError(Error::Invalid(format!(
                    "event_kind must be one of {} (got '{ek}')",
                    FIREABLE.join("|")
                ))));
            }
            if let Some(f) = spec.get("filter_json") {
                if !f.is_object() && !f.is_null() {
                    return Err(ApiError(Error::Invalid(
                        "filter_json must be a flat object of field: expected-value pairs".into(),
                    )));
                }
            }
            Ok(())
        }
        "chat" => {
            if !matches!(
                spec.get("channel").and_then(Value::as_str),
                Some("slack" | "telegram")
            ) || spec
                .get("chat")
                .and_then(Value::as_str)
                .is_none_or(|s| s.trim().is_empty())
            {
                return Err(ApiError(Error::Invalid(
                    "Chat binding needs slack|telegram and a chat/channel ID".into(),
                )));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// `DELETE /workflow-triggers/{id}`
pub async fn delete_trigger(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    let t = triggers(&ctx).get(&id).await.map_err(ApiError)?;
    let wf = repo(&ctx).get(&t.workflow_id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &wf.workspace_id, WorkspaceRole::Editor).await?;
    triggers(&ctx).delete(&id).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Webhook trigger: PUBLIC-by-token endpoint
// Route: POST /workflows/{id}/webhook/{token}
// Policy: PUBLIC (validated by token in the handler, not by bearer auth).
// Consumers: any external system that knows the workflow id + token.
// ---------------------------------------------------------------------------

/// `POST /workflows/{id}/webhook/{token}` — start a workflow run. The bearer
/// auth is NOT required; the token in the URL path IS the credential.
/// The request body (if any, JSON) becomes the run input.
pub async fn webhook_trigger(
    Path((wf_id, token)): Path<(Id, String)>,
    State(ctx): State<ServerCtx>,
    body: axum::body::Bytes,
) -> ApiResult<Json<WorkflowRun>> {
    // Verify the token belongs to an enabled webhook trigger on this workflow.
    let trigger = triggers(&ctx)
        .find_webhook(&wf_id, &token)
        .await
        .map_err(|_| ApiError(Error::Unauthorized))?;

    let wf = repo(&ctx).get(&wf_id).await.map_err(ApiError)?;
    let ws = ctx
        .workspaces
        .get(&wf.workspace_id)
        .await
        .map_err(ApiError)?;

    // Parse the body as JSON input; fall back to null if empty/invalid.
    let mut input: Value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap_or(Value::Null)
    };
    // Thread the trigger's default result destinations into the input (the
    // caller's own body keys win) so a webhook-fired run reports somewhere by
    // default instead of finishing silently.
    if let Value::Object(spec_defaults) = &trigger.spec {
        let map = match &mut input {
            Value::Object(m) => m,
            other => {
                *other = Value::Object(Default::default());
                match other {
                    Value::Object(m) => m,
                    _ => unreachable!(),
                }
            }
        };
        for key in [
            "result_channel",
            "result_chat",
            "result_thread",
            "result_webhook",
        ] {
            if map.contains_key(key) {
                continue;
            }
            if let Some(v) = spec_defaults
                .get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
            {
                map.insert(key.into(), Value::String(v.to_string()));
            }
        }
    }

    let run = repo(&ctx)
        .create_run(&wf.id, &wf.workspace_id, &input, None)
        .await
        .map_err(ApiError)?;

    workflow_engine::spawn_run(
        ctx.clone(),
        ws,
        wf,
        run.id.clone(),
        input,
        otto_core::workflows::RunScope::default(),
        None,
    );

    Ok(Json(run))
}

// ---------------------------------------------------------------------------
// Human-approval resume: POST /workflow-runs/{id}/approve
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ApproveRunReq {
    /// Required for Product gates: the full pending node detail version displayed.
    #[serde(default)]
    pub expected_detail_version: Option<String>,
    /// The node id of the `human_approval` node being resolved.
    pub node_id: String,
    /// `true` = approve, `false` = reject.
    pub approved: bool,
    /// Optional human-readable note (shown in the run log).
    #[serde(default)]
    pub note: Option<String>,
}

/// `POST /workflow-runs/{id}/approve` — resume (or reject) a paused run.
///
/// The `human_approval` node polls the run row's `waiting_approval` flag.
/// This handler:
///   1. Validates the caller is an Editor in the run's workspace.
///   2. Writes the decision into the row (`approved_by` + `approval_note`).
///   3. Clears `waiting_approval = 0` so the engine's poll loop resumes.
///   4. On rejection, leaves `approved_by = NULL` (the engine detects this
///      and errors the node).
pub async fn approve_run(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    auth: crate::auth::CurrentAuthContext,
    CurrentUser(user): CurrentUser,
    Json(req): Json<ApproveRunReq>,
) -> ApiResult<Json<Value>> {
    // S3-01: the `human_approval` gate supervises the run's own agents. A
    // workflow step's managed token authorizes as the run's owner, so the
    // Editor check alone would let the supervised agent approve itself.
    crate::auth::require_human(&auth.0)?;
    let run = repo(&ctx).get_run(&id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &run.workspace_id, WorkspaceRole::Editor).await?;
    let rev = repo(&ctx)
        .record_approval(
            &id,
            &req.node_id,
            req.approved.then_some(&user.id),
            req.note
                .as_deref()
                .unwrap_or(if req.approved { "" } else { "rejected" }),
            req.expected_detail_version.as_deref(),
        )
        .await
        .map_err(ApiError)?;
    emit_run_decision(&ctx, &run.workspace_id, &id, &req.node_id, rev);
    if req.approved {
        Ok(Json(
            json!({"approved":true,"approved_by":user.id,"note":req.note}),
        ))
    } else {
        Ok(Json(
            json!({"approved":false,"rejected_by":user.id,"note":req.note}),
        ))
    }
}

/// Announce an approval decision over WS: the run is no longer waiting. The
/// node itself is still `running` until the engine's resume poll finishes it.
fn emit_run_decision(ctx: &ServerCtx, workspace_id: &Id, run_id: &Id, node_id: &str, rev: i64) {
    let _ = ctx.events.send(Event::WorkflowRunUpdated {
        workspace_id: workspace_id.clone(),
        run_id: run_id.clone(),
        status: "running".into(),
        node_id: Some(node_id.to_string()),
        rev,
        node: None,
        nodes_done: 0,
        nodes_total: 0,
        waiting_approval: false,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn version_page_fixture(
        count: i64,
    ) -> (tempfile::TempDir, ServerCtx, axum::Router, Workflow) {
        use crate::routes::browser::tests::{mem_pool, seed_workspace, test_ctx};
        let tmp = tempfile::TempDir::new().unwrap();
        let pool = mem_pool().await;
        seed_workspace(&pool, "version-ws").await;
        sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,created_at) VALUES('root','root','x','Root',1,?)")
            .bind(chrono::Utc::now().to_rfc3339()).execute(&pool).await.unwrap();
        let ctx = test_ctx(&pool, tmp.path().to_path_buf()).await;
        let graph: WorkflowGraph = serde_json::from_value(json!({"nodes":[{"id":"n","kind":"agent","params":{"prompt":"x".repeat(4096)}}],"edges":[]})).unwrap();
        let wf = repo(&ctx)
            .create(
                &"version-ws".into(),
                "Current name",
                "Current description",
                &"instruction".repeat(512),
                &graph,
                &"root".into(),
            )
            .await
            .unwrap();
        for version in 2..=count {
            repo(&ctx)
                .snapshot_version(
                    &wf.id,
                    version,
                    "Historical name",
                    "Historical description",
                    &format!("instruction-{version}"),
                    &graph,
                    "saved",
                    "resume",
                    &"root".into(),
                )
                .await
                .unwrap();
        }
        sqlx::query("UPDATE workflows SET version=? WHERE id=?")
            .bind(count)
            .bind(&wf.id)
            .execute(&ctx.pool)
            .await
            .unwrap();
        let app = axum::Router::new()
            .route("/workflows/{id}", axum::routing::patch(update_workflow))
            .route(
                "/workflows/{id}/versions",
                axum::routing::get(list_versions),
            )
            .route(
                "/workflows/{id}/versions/{v}",
                axum::routing::get(get_version),
            )
            .route(
                "/workflows/{id}/versions/{v}/restore",
                axum::routing::post(restore_version),
            )
            .with_state(ctx.clone());
        (tmp, ctx, app, wf)
    }

    async fn version_request(app: &axum::Router, method: &str, path: &str) -> (StatusCode, Value) {
        version_request_body(app, method, path, None).await
    }

    async fn version_request_body(
        app: &axum::Router,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        use tower::ServiceExt;
        let has_body = body.is_some();
        let request_body = body
            .map(|v| axum::body::Body::from(v.to_string()))
            .unwrap_or_default();
        let mut builder = axum::http::Request::builder().method(method).uri(path);
        if has_body {
            builder = builder.header("content-type", "application/json");
        }
        let mut request = builder.body(request_body).unwrap();
        request.extensions_mut().insert(otto_core::auth::AuthUser(
            crate::routes::browser::tests::root_user(),
        ));
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes))),
        )
    }

    #[tokio::test]
    async fn review4_workflow_routes_publish_matching_snapshots_with_run_admission() {
        for restore in [false, true] {
            let (_tmp, ctx, app, wf) = version_page_fixture(2).await;
            let path = format!("/workflows/{}", wf.id);
            let second_path = if restore {
                format!("{path}/versions/2/restore")
            } else {
                path.clone()
            };
            let graph = json!({"nodes":[{"id":"saved-X","kind":"manual_trigger"}],"edges":[]});
            let repository = repo(&ctx);
            let input = json!({});
            let (a, b, queued) = tokio::join!(
                version_request_body(
                    &app,
                    "PATCH",
                    &path,
                    Some(json!({"graph":graph,"instructions":"saved X","on_restart":"fail"}))
                ),
                version_request_body(
                    &app,
                    if restore { "POST" } else { "PATCH" },
                    &second_path,
                    Some(if restore {
                        json!({})
                    } else {
                        json!({"instructions":"saved Y"})
                    })
                ),
                repository.create_run(&wf.id, &wf.workspace_id, &input, None),
            );
            assert_eq!(a.0, StatusCode::OK);
            assert_eq!(b.0, StatusCode::OK);
            assert_ne!(a.1["version"], b.1["version"]);
            for row in [&a.1, &b.1] {
                let snapshot = repository
                    .get_version(&wf.id, row["version"].as_i64().unwrap())
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(row["instructions"], snapshot.instructions);
                assert_eq!(row["graph"], json!(snapshot.graph));
                assert_eq!(row["on_restart"], snapshot.on_restart);
            }
            let queued = queued.unwrap();
            assert!(repository.definition_for_run(&queued).await.is_ok());
            let live = repository.get(&wf.id).await.unwrap();
            let next = repository
                .create_run(&wf.id, &wf.workspace_id, &input, None)
                .await
                .unwrap();
            let pinned = repository.definition_for_run(&next).await.unwrap();
            assert_eq!(pinned.version, live.version);
            assert_eq!(pinned.instructions, live.instructions);
            assert_eq!(json!(pinned.graph), json!(live.graph));
            assert_eq!(repository.list_versions(&wf.id).await.unwrap().len(), 4);
        }
    }

    #[tokio::test]
    async fn review4_workflow_route_snapshot_conflict_never_partially_saves() {
        let (_tmp, ctx, app, wf) = version_page_fixture(1).await;
        repo(&ctx)
            .snapshot_version(
                &wf.id,
                2,
                "existing",
                "",
                "collision",
                &wf.graph,
                "import",
                "resume",
                &"root".into(),
            )
            .await
            .unwrap();
        let (status, _) = version_request_body(
            &app,
            "PATCH",
            &format!("/workflows/{}", wf.id),
            Some(json!({"name":"lost name","instructions":"lost instructions"})),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        let live = repo(&ctx).get(&wf.id).await.unwrap();
        assert_eq!(live.version, 1);
        assert_eq!(live.name, wf.name);
        assert_eq!(live.instructions, wf.instructions);
    }

    #[tokio::test]
    async fn review4_version_http_default_and_requested_pages_are_bounded() {
        let (_tmp, _ctx, app, wf) = version_page_fixture(121).await;
        for query in ["", "?summary=true&limit=10000", "?limit=10000"] {
            let (status, body) = version_request(
                &app,
                "GET",
                &format!("/workflows/{}/versions{query}", wf.id),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert!(
                body.as_array().unwrap().len() <= 100,
                "version page must be bounded, query={query}"
            );
        }
    }

    #[tokio::test]
    async fn review4_version_http_summary_excludes_definition_bodies() {
        let (_tmp, _ctx, app, wf) = version_page_fixture(3).await;
        let (status, body) = version_request(
            &app,
            "GET",
            &format!("/workflows/{}/versions?summary=true&limit=2", wf.id),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let rows = body.as_array().unwrap();
        assert!(
            rows.iter()
                .all(|row| row.get("graph").is_none() && row.get("instructions").is_none()),
            "summary pages must omit graph and instructions"
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["version"], 3);
        assert!(
            rows[0].get("id").is_some()
                && rows[0].get("created_at").is_some()
                && rows[0].get("note").is_some()
        );
    }

    #[tokio::test]
    async fn review4_version_http_exclusive_cursor_reaches_oldest_despite_new_version() {
        let (_tmp, ctx, app, wf) = version_page_fixture(5).await;
        let (_, first) = version_request(
            &app,
            "GET",
            &format!("/workflows/{}/versions?summary=true&limit=2", wf.id),
        )
        .await;
        assert_eq!(
            first
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["version"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            vec![5, 4]
        );
        repo(&ctx)
            .snapshot_version(
                &wf.id,
                6,
                "new",
                "",
                "new instructions",
                &wf.graph,
                "intervening save",
                "resume",
                &"root".into(),
            )
            .await
            .unwrap();
        let mut seen = vec![5, 4];
        let mut before = 4;
        loop {
            let (status, page) = version_request(
                &app,
                "GET",
                &format!(
                    "/workflows/{}/versions?summary=true&limit=2&before_version={before}",
                    wf.id
                ),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            let rows = page.as_array().unwrap();
            if rows.is_empty() {
                break;
            }
            assert!(rows.len() <= 2);
            for row in rows {
                let v = row["version"].as_i64().unwrap();
                assert!(v < before, "cursor must be exclusive");
                seen.push(v);
            }
            before = rows.last().unwrap()["version"].as_i64().unwrap();
        }
        assert_eq!(seen, vec![5, 4, 3, 2, 1]);
        assert_eq!(
            repo(&ctx).list_versions(&wf.id).await.unwrap().len(),
            6,
            "paging must never prune history"
        );
    }

    #[tokio::test]
    async fn review4_version_http_full_detail_and_oldest_restore_remain_reachable() {
        let (_tmp, ctx, app, wf) = version_page_fixture(121).await;
        let (status, full) =
            version_request(&app, "GET", &format!("/workflows/{}/versions/1", wf.id)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(full["instructions"], wf.instructions);
        assert_eq!(
            full["graph"]["nodes"][0]["params"]["prompt"]
                .as_str()
                .unwrap()
                .len(),
            4096
        );
        let (status, restored) = version_request(
            &app,
            "POST",
            &format!("/workflows/{}/versions/2/restore", wf.id),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(restored["version"], 122);
        assert_eq!(restored["name"], "Current name");
        assert_eq!(restored["description"], "Current description");
        assert_eq!(restored["instructions"], "instruction-2");
        let snapshot = repo(&ctx).get_version(&wf.id, 122).await.unwrap().unwrap();
        assert_eq!(snapshot.name, "Historical name");
        assert_eq!(snapshot.description, "Historical description");
        assert_eq!(repo(&ctx).list_versions(&wf.id).await.unwrap().len(), 122);
    }

    #[test]
    fn workflow_trigger_validation_accepts_canonical_events_and_rejects_bad_destinations() {
        assert!(validate_trigger_spec(
            "event",
            &json!({"event_kind":"review_changed","filter_json":{"status":"done"}})
        )
        .is_ok());
        assert!(validate_trigger_spec("event", &json!({"event_kind":"ReviewChanged"})).is_err());
        assert!(validate_trigger_spec("schedule", &json!({"cadence":"cron","expr":"0 9 * * 1-5","timezone":"Asia/Jerusalem","result_channel":"slack","result_chat":"C123"})).is_ok());
        for spec in [
            json!({"timezone":"Invalid/Zone"}),
            json!({"result_channel":"slack"}),
            json!({"result_webhook":"file:///tmp/result"}),
        ] {
            assert!(
                validate_trigger_spec("webhook", &spec).is_err(),
                "invalid destination accepted: {spec}"
            );
        }
    }

    #[test]
    fn templates_cover_the_three_games() {
        let ids: Vec<String> = game_templates().into_iter().map(|t| t.id).collect();
        for want in ["game-slots", "game-crash", "game-scratch"] {
            assert!(ids.iter().any(|id| id == want), "missing template {want}");
        }
    }

    #[test]
    fn no_template_node_overlaps_another() {
        // Two nodes at the same (x,y) render stacked — one hides the other (this is
        // how the `git_pr` "Open PR" node disappeared behind "Offer improvements").
        for t in all_templates() {
            let mut seen: Vec<(i64, i64, String)> = Vec::new();
            for n in &t.graph.nodes {
                let key = (n.x as i64, n.y as i64);
                if let Some((_, _, other)) = seen.iter().find(|(x, y, _)| (*x, *y) == key) {
                    panic!(
                        "template '{}': nodes '{}' and '{}' share position {:?}",
                        t.id, other, n.id, key
                    );
                }
                seen.push((key.0, key.1, n.id.clone()));
            }
        }
    }

    #[test]
    fn flow_templates_open_a_pr() {
        // The implement/test templates must end with a git_pr node so a passing run
        // opens the PR (taking repo/branch from the run automatically).
        for id in ["write-tests", "implement-feature"] {
            let t = all_templates().into_iter().find(|t| t.id == id).unwrap();
            assert!(
                t.graph.nodes.iter().any(|n| n.kind == "git_pr"),
                "template '{id}' must contain a git_pr node"
            );
        }
    }

    #[test]
    fn every_template_uses_known_kinds_with_an_agent() {
        for t in game_templates() {
            assert!(!t.graph.nodes.is_empty(), "{} has no nodes", t.id);
            for n in &t.graph.nodes {
                assert!(
                    workflow_engine::is_known_kind(&n.kind),
                    "unknown kind '{}' in {}",
                    n.kind,
                    t.id
                );
            }
            assert!(
                t.graph.nodes.iter().any(|n| n.kind == "agent_prompt"),
                "{} has no agent node",
                t.id
            );
        }
    }

    #[test]
    fn all_new_kinds_are_in_catalog() {
        let new_kinds = [
            "db_query",
            "broker_peek",
            "channel_notify",
            "budget_gate",
            "human_approval",
            "swarm_task",
            "api_run",
            "product_analyze",
            "product_rewrite",
            "product_plan",
            "review_run",
            "condition",
            "loop",
            "product_publish",
            "canvas",
            "git_pr",
        ];
        for kind in new_kinds {
            assert!(
                workflow_engine::is_known_kind(kind),
                "catalog missing new kind '{kind}'"
            );
        }
    }

    #[test]
    fn review4_product_publish_template_has_preview_then_approval_then_live() {
        let template = flow_templates()
            .into_iter()
            .find(|template| template.id == "po-lifecycle")
            .unwrap();
        let preview = template
            .graph
            .nodes
            .iter()
            .find(|node| node.kind == "product_publish" && node.params["dry_run"] == true)
            .expect("preview node");
        let publish = template
            .graph
            .nodes
            .iter()
            .find(|node| node.kind == "product_publish" && node.params["dry_run"] == false)
            .expect("approved live successor");
        let gate = template
            .graph
            .edges
            .iter()
            .filter(|edge| edge.source == preview.id)
            .filter_map(|edge| {
                template
                    .graph
                    .nodes
                    .iter()
                    .find(|node| node.id == edge.target && node.kind == "human_approval")
            })
            .find(|gate| {
                template
                    .graph
                    .edges
                    .iter()
                    .any(|edge| edge.source == gate.id && edge.target == publish.id)
            })
            .expect("preview → review → publish edges");
        assert!(!gate.params["prompt"]
            .as_str()
            .unwrap_or_default()
            .is_empty());
        assert!(
            template.description.contains("account") && template.description.contains("space"),
            "destination prerequisites visible before running"
        );
    }

    #[test]
    fn flow_templates_are_valid() {
        let ids: Vec<String> = flow_templates().into_iter().map(|t| t.id).collect();
        for want in ["write-tests", "implement-feature", "po-lifecycle"] {
            assert!(
                ids.iter().any(|id| id == want),
                "missing flow template {want}"
            );
        }
        for t in flow_templates() {
            assert!(!t.graph.nodes.is_empty(), "{} has no nodes", t.id);
            // Every top-level node kind is known to the executor.
            for n in &t.graph.nodes {
                assert!(
                    workflow_engine::is_known_kind(&n.kind),
                    "unknown kind '{}' in {}",
                    n.kind,
                    t.id
                );
            }
            // Loop steps must also use known kinds.
            for n in &t.graph.nodes {
                if n.kind == "loop" {
                    let steps = n.params.get("steps").and_then(|s| s.as_array());
                    assert!(steps.is_some(), "{} loop has no steps", t.id);
                    for step in steps.unwrap() {
                        let k = step.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                        assert!(
                            workflow_engine::is_known_kind(k),
                            "loop step kind '{k}' unknown in {}",
                            t.id
                        );
                    }
                }
            }
            // Edges reference existing nodes (a precondition for a clean topo sort).
            let ids: std::collections::HashSet<&str> =
                t.graph.nodes.iter().map(|n| n.id.as_str()).collect();
            for e in &t.graph.edges {
                assert!(
                    ids.contains(e.source.as_str()),
                    "{}: dangling edge source",
                    t.id
                );
                assert!(
                    ids.contains(e.target.as_str()),
                    "{}: dangling edge target",
                    t.id
                );
            }
        }
    }

    #[test]
    fn test_flow_templates_shape() {
        let all = flow_templates();
        for id in ["ui-test-authoring", "api-acceptance-test-authoring"] {
            let t = all
                .iter()
                .find(|t| t.id == id)
                .unwrap_or_else(|| panic!("{id} missing"));
            assert!(
                !t.instructions.trim().is_empty(),
                "{id} ships standing instructions"
            );
            assert!(t.graph.nodes.iter().any(|n| n.kind == "prepare_context"));
            assert!(t
                .graph
                .nodes
                .iter()
                .any(|n| n.id == "report" && n.kind == "agent_prompt"));
            for n in &t.graph.nodes {
                assert!(
                    crate::workflow_engine::is_known_kind(&n.kind),
                    "unknown kind {}",
                    n.kind
                );
            }
            // report is reachable from BOTH pr and iterate (runs on failure too)
            assert!(t
                .graph
                .edges
                .iter()
                .any(|e| e.source == "pr" && e.target == "report"));
            assert!(t
                .graph
                .edges
                .iter()
                .any(|e| e.source == "iterate" && e.target == "report"));
        }
    }

    #[test]
    fn webhook_token_is_64_hex_chars() {
        let t = super::generate_webhook_token();
        assert_eq!(t.len(), 64, "token should be 32 bytes as 64 hex chars");
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn run_workflow_seeds_review_mode_into_input() {
        // The engine reads the mode off the RUN input (node inputs lose keys hop
        // by hop), so the route folds it in — overwriting a caller-supplied key.
        let out =
            seed_review_mode(json!({ "repo_id": "r1" }), Some("orchestrator")).expect("seeded");
        assert_eq!(out["review_mode"], json!("orchestrator"));
        assert_eq!(out["repo_id"], json!("r1"));

        let out = seed_review_mode(json!({ "review_mode": "orchestrator" }), Some("fan_out"))
            .expect("seeded");
        assert_eq!(out["review_mode"], json!("fan_out"));
    }

    #[test]
    fn run_workflow_null_input_becomes_object() {
        let out = seed_review_mode(Value::Null, Some("fan_out")).expect("seeded");
        assert_eq!(out, json!({ "review_mode": "fan_out" }));
    }

    #[test]
    fn run_workflow_non_object_input_with_review_mode_is_400() {
        let err = seed_review_mode(json!("just a string"), Some("fan_out")).unwrap_err();
        let want = "input must be a JSON object when review_mode is set";
        assert!(
            matches!(&err, Error::Invalid(m) if m == want),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn run_workflow_rejects_unknown_review_mode_400() {
        let err = seed_review_mode(Value::Null, Some("swarm")).unwrap_err();
        let want = "review_mode must be \"fan_out\" or \"orchestrator\"";
        assert!(
            matches!(&err, Error::Invalid(m) if m == want),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn run_workflow_without_review_mode_leaves_input_untouched() {
        // Every pre-field caller (UI, MCP, triggers) must keep posting bare inputs.
        for input in [Value::Null, json!("scalar"), json!({ "repo_id": "r1" })] {
            assert_eq!(
                seed_review_mode(input.clone(), None).expect("passthrough"),
                input
            );
        }
    }
}
