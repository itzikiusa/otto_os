//! Workflow engine routes: CRUD, the node-type catalog, agent-mode generation
//! (describe a flow → we build the graph), run + run-status, triggers, and
//! the human-approval resume endpoint.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use otto_core::domain::WorkspaceRole;
use otto_core::workflows::{
    CreateWorkflowReq, FromTemplateReq, NodeTypeSpec, RunStatus, RunWorkflowReq, UpdateWorkflowReq,
    Workflow, WorkflowEdge, WorkflowGraph, WorkflowNode, WorkflowRun, WorkflowTemplate,
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
        .create(&wid, name, req.description.as_deref().unwrap_or(""), &graph, &user.id)
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
    let updated = repo(&ctx)
        .update(
            &id,
            req.name.as_deref(),
            req.description.as_deref(),
            req.graph.as_ref(),
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(updated))
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
        return Err(ApiError(Error::Invalid("description must not be empty".into())));
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
        .create(&wid, &name, description, &graph, &user.id)
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
        .map(|s| format!("- {} (in {}, out {}): {}", s.kind, s.inputs, s.outputs, s.description))
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = format!(
        "You are building an automation workflow as a directed graph. Available node kinds:\n{kinds}\n\n\
         Produce ONLY a JSON object of shape \
         {{\"nodes\":[{{\"id\":\"n1\",\"kind\":\"<kind>\",\"name\":\"<label>\",\"params\":{{}}}}],\
         \"edges\":[{{\"id\":\"e1\",\"source\":\"n1\",\"target\":\"n2\"}}]}}. \
         Start with a manual_trigger. Use only the listed kinds. Wire nodes left-to-right to \
         accomplish the goal. No prose, no markdown fences.\n\nGoal: {description}"
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

/// Drop nodes with unknown kinds and edges referencing missing nodes.
fn sanitize(mut g: WorkflowGraph) -> WorkflowGraph {
    g.nodes.retain(|n| workflow_engine::is_known_kind(&n.kind));
    let ids: std::collections::HashSet<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
    g.edges
        .retain(|e| ids.contains(e.source.as_str()) && ids.contains(e.target.as_str()));
    g
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
            },
            WorkflowNode {
                id: "agent".into(),
                kind: "agent_prompt".into(),
                name: "Agent".into(),
                x: 0.0,
                y: 0.0,
                params: serde_json::json!({ "prompt": description }),
            },
        ],
        edges: vec![WorkflowEdge {
            id: "e1".into(),
            source: "trigger".into(),
            target: "agent".into(),
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
        format!("{}…", &s[..s.char_indices().take(57).last().map(|(i, _)| i).unwrap_or(57)])
    } else {
        s
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
    let ws = ctx.workspaces.get(&wf.workspace_id).await.map_err(ApiError)?;

    let input = req.input.unwrap_or(Value::Null);
    let run = repo(&ctx)
        .create_run(&wf.id, &wf.workspace_id, &input)
        .await
        .map_err(ApiError)?;

    // Execute in the background; the UI polls GET /workflow-runs/{id}.
    let ctx2 = ctx.clone();
    let run_id = run.id.clone();
    let start_node = req.start_node.clone();
    let only_node = req.only_node;
    tokio::spawn(async move {
        workflow_engine::run_workflow(ctx2, ws, wf, run_id, input, start_node, only_node).await;
    });

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
        repo(&ctx)
            .update_run(&id, RunStatus::Canceled, &run.nodes, Some("canceled"), true)
            .await
            .map_err(ApiError)?;
    }
    repo(&ctx).get_run(&id).await.map(Json).map_err(ApiError)
}

/// `GET /workflows/{id}/runs`
pub async fn list_runs(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<WorkflowRun>>> {
    let wf = repo(&ctx).get(&id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &wf.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(repo(&ctx).list_runs(&id).await.map_err(ApiError)?))
}

/// `GET /workflow-runs/{id}`
pub async fn get_run(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<WorkflowRun>> {
    let run = repo(&ctx).get_run(&id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &run.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(run))
}

// ---------------------------------------------------------------------------
// Example templates (game pipelines: agent design + engine scaffold)
// ---------------------------------------------------------------------------

/// `GET /workflows/templates`
pub async fn list_templates() -> Json<Vec<WorkflowTemplate>> {
    Json(game_templates())
}

/// `POST /workspaces/{wid}/workflows/from-template`
pub async fn create_from_template(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<FromTemplateReq>,
) -> ApiResult<Json<Workflow>> {
    crate::auth::require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let tpl = game_templates()
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
        .create(&wid, &name, &tpl.description, &tpl.graph, &user.id)
        .await
        .map_err(ApiError)?;
    Ok(Json(wf))
}

/// Build the three game-pipeline templates. Each chains:
/// trigger → agent (design rules) → game_engine (kind) → verifier.
fn game_templates() -> Vec<WorkflowTemplate> {
    fn pipeline(game: &str, design: &str) -> WorkflowGraph {
        let node = |id: &str, kind: &str, name: &str, x: f64, params: serde_json::Value| WorkflowNode {
            id: id.into(),
            kind: kind.into(),
            name: name.into(),
            x,
            y: 70.0,
            params,
        };
        let edge = |s: &str, t: &str| WorkflowEdge {
            id: format!("{s}-{t}"),
            source: s.into(),
            target: t.into(),
        };
        WorkflowGraph {
            nodes: vec![
                node("trigger", "manual_trigger", "Start", 40.0, serde_json::Value::Null),
                node("design", "agent_prompt", "Design game", 320.0, serde_json::json!({ "prompt": design })),
                node("game", "game_engine", "Build game", 600.0, serde_json::json!({ "game": game })),
                node("verify", "verifier", "Verify", 880.0, serde_json::Value::Null),
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
            icon: "ticket".into(),
            graph: pipeline(
                "scratch",
                "Design a scratch-card game: prize tiers and their win probabilities, panel layout, reveal mechanic, and RTP ~95%. Return a structured spec.",
            ),
        },
    ]
}

// ---------------------------------------------------------------------------
// Trigger CRUD: GET/POST/PATCH/DELETE on workflow triggers
// ---------------------------------------------------------------------------

fn triggers(ctx: &ServerCtx) -> TriggersRepo {
    TriggersRepo::new(ctx.pool.clone())
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
    if !matches!(req.kind.as_str(), "schedule" | "webhook" | "event") {
        return Err(ApiError(Error::Invalid(
            "trigger kind must be 'schedule', 'webhook', or 'event'".into(),
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
    let updated = triggers(&ctx)
        .update(&id, req.spec, req.enabled)
        .await
        .map_err(ApiError)?;
    Ok(Json(updated))
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
    let _trigger = triggers(&ctx)
        .find_webhook(&wf_id, &token)
        .await
        .map_err(|_| ApiError(Error::Unauthorized))?;

    let wf = repo(&ctx).get(&wf_id).await.map_err(ApiError)?;
    let ws = ctx.workspaces.get(&wf.workspace_id).await.map_err(ApiError)?;

    // Parse the body as JSON input; fall back to null if empty/invalid.
    let input: Value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap_or(Value::Null)
    };

    let run = repo(&ctx)
        .create_run(&wf.id, &wf.workspace_id, &input)
        .await
        .map_err(ApiError)?;

    let ctx2 = ctx.clone();
    let run_id = run.id.clone();
    tokio::spawn(async move {
        workflow_engine::run_workflow(ctx2, ws, wf, run_id, input, None, false).await;
    });

    Ok(Json(run))
}

// ---------------------------------------------------------------------------
// Human-approval resume: POST /workflow-runs/{id}/approve
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ApproveRunReq {
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
    CurrentUser(user): CurrentUser,
    Json(req): Json<ApproveRunReq>,
) -> ApiResult<Json<Value>> {
    let run = repo(&ctx).get_run(&id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &run.workspace_id, WorkspaceRole::Editor).await?;

    // Confirm the run is actually waiting for approval.
    let row = sqlx::query(
        "SELECT waiting_approval, approval_node_id FROM workflow_runs WHERE id = ?",
    )
    .bind(&id)
    .fetch_optional(&ctx.pool)
    .await
    .map_err(|e| ApiError(Error::Internal(format!("approve_run: {e}"))))?
    .ok_or_else(|| ApiError(Error::NotFound("run".into())))?;

    use sqlx::Row as _;
    let waiting: i64 = row.get("waiting_approval");
    if waiting == 0 {
        return Err(ApiError(Error::Invalid(
            "run is not currently waiting for approval".into(),
        )));
    }
    let node_in_row: Option<String> = row.get("approval_node_id");
    if node_in_row.as_deref() != Some(&req.node_id) {
        return Err(ApiError(Error::Invalid(format!(
            "approval node_id mismatch: run is paused at '{}', not '{}'",
            node_in_row.as_deref().unwrap_or("?"),
            req.node_id
        ))));
    }

    let now = chrono::Utc::now().to_rfc3339();
    if req.approved {
        // Record the approver and clear the pause flag atomically.
        sqlx::query(
            "UPDATE workflow_runs
             SET waiting_approval = 0,
                 approved_by     = ?,
                 approval_note   = ?,
                 approved_at     = ?
             WHERE id = ?",
        )
        .bind(&user.id)
        .bind(req.note.as_deref().unwrap_or(""))
        .bind(&now)
        .bind(&id)
        .execute(&ctx.pool)
        .await
        .map_err(|e| ApiError(Error::Internal(format!("approve_run record: {e}"))))?;

        Ok(Json(json!({
            "approved": true,
            "approved_by": user.id,
            "note": req.note,
        })))
    } else {
        // Rejection: clear `approved_by` (NULL) and clear the pause flag so the
        // engine's poll loop sees `waiting_approval = 0` AND `approved_by = NULL`
        // and errors the node.
        sqlx::query(
            "UPDATE workflow_runs
             SET waiting_approval = 0,
                 approved_by     = NULL,
                 approval_note   = ?,
                 approved_at     = ?
             WHERE id = ?",
        )
        .bind(req.note.as_deref().unwrap_or("rejected"))
        .bind(&now)
        .bind(&id)
        .execute(&ctx.pool)
        .await
        .map_err(|e| ApiError(Error::Internal(format!("reject_run record: {e}"))))?;

        Ok(Json(json!({
            "approved": false,
            "rejected_by": user.id,
            "note": req.note,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_cover_the_three_games() {
        let ids: Vec<String> = game_templates().into_iter().map(|t| t.id).collect();
        for want in ["game-slots", "game-crash", "game-scratch"] {
            assert!(ids.iter().any(|id| id == want), "missing template {want}");
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
            "db_query", "broker_peek", "channel_notify", "budget_gate",
            "human_approval", "swarm_task", "api_run",
            "product_analyze", "product_rewrite", "product_plan", "review_run",
        ];
        for kind in new_kinds {
            assert!(
                workflow_engine::is_known_kind(kind),
                "catalog missing new kind '{kind}'"
            );
        }
    }

    #[test]
    fn webhook_token_is_64_hex_chars() {
        let t = super::generate_webhook_token();
        assert_eq!(t.len(), 64, "token should be 32 bytes as 64 hex chars");
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
