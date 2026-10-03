//! Durable, cancellable automation execution. Inputs are pinned before spawning;
//! each completed step is saved before starting the next outbound request.
use crate::{
    api_secrets,
    auth::{require_ws_role, CurrentAuthContext, CurrentUser},
    error::{ApiError, ApiResult},
    state::ServerCtx,
};
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};
use otto_core::{
    api::{ApiAutomationRun, ApiRunResult, StartApiAutomationRunReq},
    domain::WorkspaceRole,
    new_id, Error, Id,
};
use otto_state::{api_runs::ApiRunsRepo, ApiClientRepo, NewApiHistory};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};
use tokio::sync::watch;

/// A run in flight: its cancel switch and a step-count progress channel. The
/// entry is removed (dropping both senders) only after the final record is
/// saved, so a progress receiver seeing its sender close means "finished".
struct LiveRun {
    cancel: watch::Sender<bool>,
    progress: watch::Sender<usize>,
}

fn live() -> &'static Mutex<HashMap<Id, LiveRun>> {
    static LIVE: OnceLock<Mutex<HashMap<Id, LiveRun>>> = OnceLock::new();
    LIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Progress of a run still in flight (`None` once it finished, or for an
/// unknown id): `changed()` fires per completed step and errors when the run
/// ends — what the synchronous runner waits on instead of polling the DB
/// every 100 ms (perf F2).
pub(crate) fn subscribe(run_id: &Id) -> Option<watch::Receiver<usize>> {
    live()
        .lock()
        .unwrap()
        .get(run_id)
        .map(|r| r.progress.subscribe())
}

fn publish_progress(ctx: &ServerCtx, run: &ApiAutomationRun) {
    let _ = ctx.events.send(otto_core::event::Event::ApiRunProgress {
        workspace_id: run.workspace_id.clone(),
        automation_id: run.automation_id.clone(),
        run_id: run.id.clone(),
        status: run.status.clone(),
        steps_done: run.report.steps.len(),
    });
}
fn db_error(e: sqlx::Error) -> ApiError {
    ApiError(if matches!(e, sqlx::Error::RowNotFound) {
        Error::NotFound("automation run".into())
    } else {
        Error::Internal(e.to_string())
    })
}
fn validate(options: &StartApiAutomationRunReq, step_count: usize) -> Result<(), String> {
    if step_count == 0 {
        return Err("Add at least one saved request before running.".into());
    }
    if step_count.saturating_mul(options.dataset.len().max(1)) > 1000 {
        return Err("A run supports at most 1000 request executions across dataset rows.".into());
    }
    if serde_json::to_vec(&options.dataset)
        .map_err(|e| e.to_string())?
        .len()
        > 1024 * 1024
    {
        return Err("Dataset exceeds 1 MiB.".into());
    }
    Ok(())
}

pub async fn start(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    headers: HeaderMap,
    Json(options): Json<StartApiAutomationRunReq>,
) -> ApiResult<Json<ApiAutomationRun>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let api = ApiClientRepo::new(ctx.pool.clone());
    let automation = api.get_automation(&id).await?;
    if automation.workspace_id != wid {
        return Err(ApiError(Error::NotFound("automation".into())));
    }
    let steps = automation.steps.as_array().cloned().unwrap_or_default();
    validate(&options, steps.len()).map_err(|e| ApiError(Error::Invalid(e)))?;
    let (initial_vars, environment, env_secrets) =
        super::api_client::resolve_environment(&ctx, &api, &wid, options.environment_id.as_ref())
            .await?;
    let mut secret_values: Vec<String> = env_secrets.values().cloned().collect();
    // An agent-started run keeps every stored secret on its bound host (see
    // `run_step`); a person's run is theirs to aim.
    let binding = if super::api_client::is_agent_caller(&headers, &auth) {
        Some(super::api_client::StepSecretBinding {
            bound_hosts: super::api_client::bound_env_secret_hosts(&api, &wid, &initial_vars)
                .await?,
            env_vars: initial_vars.clone(),
            env_blob: env_secrets.clone(),
            secret_keys: environment
                .as_ref()
                .map(|env| env.secret_keys.clone())
                .unwrap_or_default(),
        })
    } else {
        None
    };
    let mut requests = Vec::new();
    let mut snapshot = Vec::new();
    for step in &steps {
        let rid = step.get("request_id").and_then(Value::as_str).unwrap_or("");
        let request = api.get_request(&rid.to_string()).await?;
        if request.workspace_id != wid {
            return Err(ApiError(Error::NotFound("request".into())));
        }
        secret_values.extend(
            api_secrets::load_blob(ctx.secrets.as_ref(), &api_secrets::request_ref(rid))
                .into_values(),
        );
        snapshot.push(json!({"request_id":rid,"name":request.name,"method":request.method,"url":request.url,"updated_at":request.updated_at,
            "assertions":step.get("assertions"),"extract":step.get("extract")}));
        requests.push(request);
    }
    // Dataset and runtime values may carry credentials. Conservatively redact all
    // string values in diagnostic output, without persisting the dataset itself.
    for row in &options.dataset {
        secret_values.extend(row.values().filter_map(Value::as_str).map(str::to_string));
    }
    let mut snapshot = Value::Array(snapshot);
    api_secrets::scrub_json(&mut snapshot, &secret_values);
    let mut run = ApiAutomationRun {
        id: new_id(),
        workspace_id: wid.clone(),
        automation_id: id.clone(),
        environment_id: environment.map(|e| e.id),
        created_by: user.id.clone(),
        status: "running".into(),
        created_at: chrono::Utc::now().to_rfc3339(),
        finished_at: None,
        stop_on_failure: options.stop_on_failure,
        dataset_rows: options.dataset.len().max(1),
        snapshot,
        report: ApiRunResult {
            automation_id: id,
            steps: vec![],
            passed: false,
        },
        result_rows: vec![],
        result_ids: vec![],
        error: None,
        steps_total: None,
        steps_passed: None,
    };
    let reports = ApiRunsRepo(ctx.pool.clone());
    reports.save_header(&run).await.map_err(db_error)?;
    let initial = run.clone();
    let (cancel, mut cancelled) = watch::channel(false);
    let (progress, _) = watch::channel(0usize);
    live()
        .lock()
        .unwrap()
        .insert(run.id.clone(), LiveRun { cancel, progress });
    tokio::spawn(async move {
        let rows = if options.dataset.is_empty() {
            vec![serde_json::Map::new()]
        } else {
            options.dataset
        };
        'rows: for (row_idx, row) in rows.into_iter().enumerate() {
            let mut vars = initial_vars.clone();
            vars.extend(row);
            for (step, request) in steps.iter().zip(&requests) {
                if *cancelled.borrow() {
                    run.status = "cancelled".into();
                    break 'rows;
                }
                let result = tokio::select! {
                    result = super::api_client::run_step(&ctx, &api, &wid, step, &mut vars, Some(request.clone()), &run.created_by, binding.as_ref()) => result,
                    _ = cancelled.changed() => {run.status = "cancelled".into(); break 'rows;}
                };
                let step_id = new_id();
                let failed = !result.ok;
                let mut value = serde_json::to_value(result).expect("serializable step");
                api_secrets::scrub_json(&mut value, &secret_values);
                let result =
                    serde_json::from_value(value.clone()).expect("same step shape after redaction");
                // A correlated request history entry is retained for each completed
                // step, including transport/script failures and dataset row index.
                let inserted = api.insert_history(NewApiHistory {
                    workspace_id: wid.clone(), method: request.method.clone(), url: api_secrets::scrub_str(&request.url, &secret_values),
                    status: value.get("status").and_then(Value::as_i64), duration_ms: value.get("duration_ms").and_then(Value::as_i64),
                    request: json!({"source":"automation_run","automation_run_id":run.id,"automation_id":run.automation_id,"request_id":request.id,"dataset_row":row_idx,"step_result_id":step_id}),
                    response: value,
                }).await;
                // Live History refresh, like an interactive send. The row's
                // `source` is the "automation_run" marker (no `{kind}` object),
                // which the history projection files under "human".
                if let Ok(entry) = inserted {
                    let _ = ctx
                        .events
                        .send(otto_core::event::Event::ApiHistoryAppended {
                            workspace_id: entry.workspace_id,
                            entry_id: entry.id,
                            source: "human".into(),
                            session_id: None,
                            request_id: Some(request.id.clone()),
                        });
                }
                // Same runtime history retention as interactive runs — applied
                // every PRUNE_EVERY steps and once at the end instead of per
                // step (a settings read + DELETE scan per request). Pruning is
                // housekeeping; the per-step durability below is unchanged.
                if (run.report.steps.len() + 1).is_multiple_of(PRUNE_EVERY) {
                    prune_history(&ctx, &api, &wid).await;
                }
                // One step row + two counters per step (perf F2) — the run
                // record used to be re-serialized whole after every step.
                let idx = run.report.steps.len();
                if let Err(e) = reports
                    .append_step(&run.id, idx, row_idx, &step_id, &result)
                    .await
                {
                    run.status = "interrupted".into();
                    run.error = Some(format!("Could not persist step result: {e}"));
                    break 'rows;
                }
                run.report.steps.push(result);
                run.result_rows.push(row_idx);
                run.result_ids.push(step_id);
                if let Some(live) = live().lock().unwrap().get(&run.id) {
                    live.progress.send_replace(run.report.steps.len());
                }
                publish_progress(&ctx, &run);
                if failed && options.stop_on_failure {
                    run.status = "failed".into();
                    break 'rows;
                }
            }
        }
        run.report.passed = run.report.steps.iter().all(|s| s.ok)
            && run.report.steps.len() == steps.len() * run.dataset_rows;
        if run.status == "running" {
            run.status = if run.report.passed {
                "passed"
            } else {
                "failed"
            }
            .into();
        }
        if run.status == "cancelled" {
            run.error = Some(
                "Run cancelled. An in-flight request may already have reached its server.".into(),
            );
        }
        run.finished_at = Some(chrono::Utc::now().to_rfc3339());
        if let Err(e) = reports.save_header(&run).await {
            tracing::error!(run_id=%run.id,error=%e,"could not finalize API automation run");
        }
        if !run.report.steps.is_empty() {
            prune_history(&ctx, &api, &wid).await;
        }
        // Opt-in run retention (`settings.api_client.automation_runs_keep`,
        // default 0 = keep every run — old reports are user data).
        let keep = runs_keep(&ctx, &wid).await;
        if keep > 0 {
            let _ = reports.prune_runs(&wid, &run.automation_id, keep).await;
        }
        live().lock().unwrap().remove(&run.id);
        publish_progress(&ctx, &run);
    });
    Ok(Json(initial))
}

/// Steps between history-retention passes during a run (plus one at the end).
/// Each completed step is still durable before the next request goes out — as
/// one `api_automation_run_steps` row, not a rewrite of the whole record.
const PRUNE_EVERY: usize = 50;

/// `settings.api_client.automation_runs_keep`: newest finished runs kept per
/// automation (`0`/absent = keep all, the default —
/// [`otto_state::api_runs::RUNS_KEEP_DEFAULT`]).
async fn runs_keep(ctx: &ServerCtx, wid: &Id) -> i64 {
    ctx.workspaces
        .get(wid)
        .await
        .ok()
        .and_then(|ws| {
            ws.settings
                .get("api_client")
                .and_then(|a| a.get("automation_runs_keep"))
                .and_then(Value::as_i64)
        })
        .filter(|v| *v > 0)
        .unwrap_or(otto_state::api_runs::RUNS_KEEP_DEFAULT)
}

async fn prune_history(ctx: &ServerCtx, api: &ApiClientRepo, wid: &Id) {
    let (max_rows, max_days) = super::api_client::history_retention(ctx, wid).await;
    let _ = api.prune_history(wid, max_rows, max_days).await;
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub automation_id: Option<String>,
    pub before: Option<String>,
}
pub async fn list(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<ApiAutomationRun>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    Ok(Json(
        ApiRunsRepo(ctx.pool)
            .list(&wid, q.automation_id.as_deref(), q.before.as_deref())
            .await
            .map_err(db_error)?,
    ))
}
#[derive(Deserialize)]
pub struct GetQuery {
    /// Delta poll: return only the step results after the first `after` (and
    /// no `snapshot`) — the running view already holds the rest.
    pub after: Option<usize>,
}
pub async fn get(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<GetQuery>,
) -> ApiResult<Json<ApiAutomationRun>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = ApiRunsRepo(ctx.pool);
    let run = match q.after {
        Some(after) => repo.get_after(&wid, &id, after).await,
        None => repo.get(&wid, &id).await,
    }
    .map_err(db_error)?;
    Ok(Json(run))
}
pub async fn cancel(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<ApiAutomationRun>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let run = ApiRunsRepo(ctx.pool)
        .get(&wid, &id)
        .await
        .map_err(db_error)?;
    if let Some(live) = live().lock().unwrap().get(&id) {
        let _ = live.cancel.send(true);
    }
    Ok(Json(run))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_dataset_work_and_rejects_empty_automation() {
        assert!(validate(&StartApiAutomationRunReq::default(), 0).is_err());
        let options = StartApiAutomationRunReq {
            dataset: vec![serde_json::Map::new(); 11],
            ..Default::default()
        };
        assert!(validate(&options, 100).is_err());
        assert!(validate(&options, 90).is_ok());
    }
}
