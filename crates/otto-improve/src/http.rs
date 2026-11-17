//! Axum router for self-improvement endpoints. Paths relative to `/api/v1`.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use chrono::Utc;
use otto_core::api::{
    BulkImprovementResp, Problem, RunNowResp, SelfImprovementConfig, SelfImprovementSummary,
    SetSelfImprovementEnabledReq, UpdateSelfImprovementReq,
};
use otto_core::auth::{AuthUser, RoleChecker};
use otto_core::domain::{
    ImprovementEdit, ImprovementEditStatus, ImprovementRun, SessionStatus, Workspace, WorkspaceRole,
};
use otto_core::{Error, Id};
use otto_state::WorkspacesRepo;
use serde::Deserialize;

use crate::config::{effective_config, next_run, write_config};
use crate::engine::{ImprovementEngine, RUN_LIST_LIMIT};

pub trait ImproveCtx: Clone + Send + Sync + 'static {
    fn engine(&self) -> &Arc<ImprovementEngine>;
    fn roles(&self) -> &Arc<dyn RoleChecker>;
    fn workspaces(&self) -> &WorkspacesRepo;
}

struct ApiErr(Error);
impl From<Error> for ApiErr {
    fn from(e: Error) -> Self {
        Self(e)
    }
}
impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Forbidden(_) => StatusCode::FORBIDDEN,
            Error::Conflict(_) => StatusCode::CONFLICT,
            Error::Invalid(_) => StatusCode::BAD_REQUEST,
            Error::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Error::UnsupportedMedia(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Error::Upstream(_) => StatusCode::BAD_GATEWAY,
            Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (
            status,
            Json(Problem {
                code: self.0.code().to_string(),
                message: self.0.to_string(),
            }),
        )
            .into_response()
    }
}
type ApiResult<T> = std::result::Result<T, ApiErr>;

pub fn router<S: ImproveCtx>() -> Router<S> {
    Router::new()
        .route(
            "/workspaces/{id}/self-improvement",
            get(get_config::<S>).put(put_config::<S>),
        )
        // Every workspace at once: a summary, and one on/off switch.
        .route(
            "/self-improvement",
            get(summary::<S>).put(set_enabled_all::<S>),
        )
        .route("/workspaces/{id}/self-improvement/run", post(run_now::<S>))
        .route("/workspaces/{id}/improvement/runs", get(list_runs::<S>))
        .route("/improvement/runs/{run_id}", get(get_run::<S>))
        .route("/workspaces/{id}/improvement/edits", get(list_edits::<S>))
        // Reject every pending suggestion the caller may decide on.
        .route("/improvement/edits/reject", post(reject_all::<S>))
        .route("/improvement/edits/{eid}/approve", post(approve::<S>))
        .route("/improvement/edits/{eid}/reject", post(reject::<S>))
        .route("/improvement/edits/{eid}/rollback", post(rollback::<S>))
        // Trigger a live single-session evolve pass for the given session.
        // Only available when the session is live (waiting/idle); rejected if
        // already running an improvement pass or the session is archived/done.
        .route("/sessions/{id}/evolve", post(evolve_session::<S>))
}

async fn get_config<S: ImproveCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
) -> ApiResult<Json<SelfImprovementConfig>> {
    s.roles()
        .check(&user.0, &ws_id, WorkspaceRole::Viewer)
        .await?;
    let ws = s.workspaces().get(&ws_id).await?;
    Ok(Json(effective_config(&ws.settings)))
}

async fn put_config<S: ImproveCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
    Json(req): Json<UpdateSelfImprovementReq>,
) -> ApiResult<Json<SelfImprovementConfig>> {
    s.roles()
        .check(&user.0, &ws_id, WorkspaceRole::Admin)
        .await?;
    let cfg = s
        .workspaces()
        .edit_settings(&ws_id, |settings| {
            let mut cfg = effective_config(settings);
            let reschedule =
                cfg.enabled != req.enabled || cfg.cadence_minutes != req.cadence_minutes;
            cfg.enabled = req.enabled;
            cfg.cadence_minutes = req.cadence_minutes;
            cfg.lookback_hours = req.lookback_hours;
            cfg.skill_allowlist = req.skill_allowlist;
            cfg.autonomy = req.autonomy;
            cfg.providers = if req.providers.is_empty() {
                vec!["claude".to_string()]
            } else {
                req.providers
            };
            cfg.live_evolve = req.live_evolve;
            // A new cadence (or re-enabling) re-anchors the next run on the last one:
            // `last_run + cadence`, or one cadence from now if it never ran. Before,
            // `next_run_at` was left alone, so dropping "Run every" from 1440 to 60
            // minutes still waited out the old 24h slot. A slot already in the past
            // makes the run due on the scheduler's next pass.
            if reschedule {
                cfg.next_run_at = Some(next_run(&cfg, cfg.last_run_at.unwrap_or_else(Utc::now)));
            }
            *settings = write_config(settings, &cfg);
            cfg
        })
        .await?;
    Ok(Json(cfg))
}

/// The caller's unarchived workspaces (the scheduler's set) where they hold
/// `min`, plus how many more they can see but not change.
async fn workspaces_with<S: ImproveCtx>(
    s: &S,
    user: &AuthUser,
    min: WorkspaceRole,
) -> Result<(Vec<Workspace>, u32), ApiErr> {
    let (mut allowed, mut skipped) = (Vec::new(), 0);
    for ws in s.workspaces().list_user_all().await? {
        if ws.archived {
            continue;
        }
        match s.roles().check(&user.0, &ws.id, min).await {
            Ok(()) => allowed.push(ws),
            Err(Error::Forbidden(_)) => {
                if min > WorkspaceRole::Viewer
                    && s.roles()
                        .check(&user.0, &ws.id, WorkspaceRole::Viewer)
                        .await
                        .is_ok()
                {
                    skipped += 1;
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok((allowed, skipped))
}

async fn summary<S: ImproveCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
) -> ApiResult<Json<SelfImprovementSummary>> {
    let (visible, _) = workspaces_with(&s, &user, WorkspaceRole::Viewer).await?;
    let pending = s.engine().improvements.pending_counts().await?;
    let mut out = SelfImprovementSummary {
        workspaces: visible.len() as u32,
        ..Default::default()
    };
    for ws in &visible {
        out.enabled += effective_config(&ws.settings).enabled as u32;
        if let Some(&n) = pending.get(&ws.id) {
            out.pending += n as u32;
            out.pending_workspaces += 1;
        }
    }
    Ok(Json(out))
}

/// `PUT /self-improvement`: the per-workspace `enabled` switch (which also
/// gates live evolve) flipped in every workspace the caller administers.
async fn set_enabled_all<S: ImproveCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Json(req): Json<SetSelfImprovementEnabledReq>,
) -> ApiResult<Json<BulkImprovementResp>> {
    let (admin, skipped) = workspaces_with(&s, &user, WorkspaceRole::Admin).await?;
    let mut changed = 0;
    for ws in admin {
        let flipped = s
            .workspaces()
            .edit_settings(&ws.id, |settings| {
                let mut cfg = effective_config(settings);
                if cfg.enabled == req.enabled {
                    return false;
                }
                cfg.enabled = req.enabled;
                // Re-anchor like `put_config` does on a switch.
                cfg.next_run_at = Some(next_run(&cfg, cfg.last_run_at.unwrap_or_else(Utc::now)));
                *settings = write_config(settings, &cfg);
                true
            })
            .await?;
        changed += flipped as u32;
    }
    Ok(Json(BulkImprovementResp {
        changed,
        workspaces: changed,
        skipped,
    }))
}

/// `POST /improvement/edits/reject`: reject every pending suggestion in the
/// workspaces where the caller may decide them. Applied edits are untouched.
async fn reject_all<S: ImproveCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
) -> ApiResult<Json<BulkImprovementResp>> {
    let (editable, skipped) = workspaces_with(&s, &user, WorkspaceRole::Editor).await?;
    let mut out = BulkImprovementResp {
        skipped,
        ..Default::default()
    };
    for ws in editable {
        let n = s
            .engine()
            .improvements
            .reject_all_pending(&ws.id, &user.0.id)
            .await?;
        if n > 0 {
            out.changed += n as u32;
            out.workspaces += 1;
        }
    }
    Ok(Json(out))
}

async fn run_now<S: ImproveCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
) -> ApiResult<Json<RunNowResp>> {
    s.roles()
        .check(&user.0, &ws_id, WorkspaceRole::Editor)
        .await?;
    if s.engine().improvements.has_running(&ws_id).await? {
        return Err(ApiErr(Error::Conflict(
            "a run is already in progress".into(),
        )));
    }
    // Mirror the PR-review pattern (otto-server `modules/review/`, `start_review`): create the run
    // row synchronously so we can return its id immediately, then run the heavy
    // analysis in the background.
    let run = s
        .engine()
        .improvements
        .create_run(&ws_id, otto_core::domain::ImprovementTrigger::Manual)
        .await?;
    let run_id = run.id.clone();
    let engine = Arc::clone(s.engine());
    let ws = ws_id.clone();
    let bg_run_id = run_id.clone();
    tokio::spawn(async move {
        if let Err(e) = engine
            .execute_run(
                &bg_run_id,
                &ws,
                otto_core::domain::ImprovementTrigger::Manual,
            )
            .await
        {
            tracing::warn!(workspace = %ws, "self-improvement manual run failed: {e}");
        }
    });
    Ok(Json(RunNowResp { run_id }))
}

async fn list_runs<S: ImproveCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
) -> ApiResult<Json<Vec<ImprovementRun>>> {
    s.roles()
        .check(&user.0, &ws_id, WorkspaceRole::Viewer)
        .await?;
    Ok(Json(
        s.engine()
            .improvements
            .list_runs(&ws_id, RUN_LIST_LIMIT)
            .await?,
    ))
}

async fn get_run<S: ImproveCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(run_id): Path<Id>,
) -> ApiResult<Json<serde_json::Value>> {
    let run = s.engine().improvements.get_run(&run_id).await?;
    s.roles()
        .check(&user.0, &run.workspace_id, WorkspaceRole::Viewer)
        .await?;
    let edits = s.engine().improvements.list_edits_by_run(&run_id).await?;
    Ok(Json(serde_json::json!({ "run": run, "edits": edits })))
}

#[derive(Deserialize)]
struct EditQuery {
    status: Option<String>,
}

async fn list_edits<S: ImproveCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
    Query(q): Query<EditQuery>,
) -> ApiResult<Json<Vec<ImprovementEdit>>> {
    s.roles()
        .check(&user.0, &ws_id, WorkspaceRole::Viewer)
        .await?;
    let status = q
        .status
        .as_deref()
        .and_then(ImprovementEditStatus::parse)
        .unwrap_or(ImprovementEditStatus::Pending);
    Ok(Json(
        s.engine()
            .improvements
            .list_edits_by_status(&ws_id, status)
            .await?,
    ))
}

async fn check_edit_ws<S: ImproveCtx>(s: &S, user: &AuthUser, edit_id: &Id) -> Result<(), ApiErr> {
    let edit = s.engine().improvements.get_edit(edit_id).await?;
    s.roles()
        .check(&user.0, &edit.workspace_id, WorkspaceRole::Editor)
        .await?;
    Ok(())
}

async fn approve<S: ImproveCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(eid): Path<Id>,
) -> ApiResult<Json<ImprovementEdit>> {
    check_edit_ws(&s, &user, &eid).await?;
    Ok(Json(s.engine().approve_edit(&eid, &user.0.id).await?))
}

async fn reject<S: ImproveCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(eid): Path<Id>,
) -> ApiResult<Json<ImprovementEdit>> {
    check_edit_ws(&s, &user, &eid).await?;
    Ok(Json(s.engine().reject_edit(&eid, &user.0.id).await?))
}

async fn rollback<S: ImproveCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(eid): Path<Id>,
) -> ApiResult<Json<ImprovementEdit>> {
    check_edit_ws(&s, &user, &eid).await?;
    Ok(Json(s.engine().rollback_edit(&eid, &user.0.id).await?))
}

/// `POST /sessions/{id}/evolve` — manually trigger a single-session live
/// evolve pass for the given session.
///
/// Guards:
/// - Caller must have `SelfImprovement:Edit` on the session's workspace.
/// - The session must be in a live status (waiting/idle); archived/done sessions
///   are rejected so we don't evolve on stale transcripts.
/// - If a run is already in progress for the workspace, we return `409 Conflict`.
///
/// The evolve runs in the background (fire-and-forget); the run id is returned
/// immediately so the caller can poll `GET /improvement/runs/{run_id}` for the
/// result, or subscribe to the `improvement_updated` WS event.
async fn evolve_session<S: ImproveCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(session_id): Path<Id>,
) -> ApiResult<Json<RunNowResp>> {
    // Resolve the session via the engine's sessions repo (avoids adding a new
    // field to ImproveCtx — ImprovementEngine already carries SessionsRepo).
    let session = s.engine().sessions.get(&session_id).await?;
    s.roles()
        .check(&user.0, &session.workspace_id, WorkspaceRole::Editor)
        .await?;

    // Reject sessions that are no longer live: evolving an archived transcript
    // produces low-quality edits and conflicts with the live-evolver's logic.
    // Running / Working / Idle are the "live" states; Exited and Reconnectable
    // mean the child is gone and an evolve pass would operate on a stale transcript.
    let live = matches!(
        session.status,
        SessionStatus::Running | SessionStatus::Working | SessionStatus::Idle
    );
    if !live {
        return Err(ApiErr(Error::Invalid(
            "session must be live (running/working/idle) to trigger an evolve pass".into(),
        )));
    }

    // Conflict guard: mirror `run_now`'s check so a queued workspace pass and a
    // per-session pass don't run concurrently.
    if s.engine()
        .improvements
        .has_running(&session.workspace_id)
        .await?
    {
        return Err(ApiErr(Error::Conflict(
            "an improvement run is already in progress for this workspace".into(),
        )));
    }

    let engine = Arc::clone(s.engine());
    let sid = session_id.clone();
    // The run row is created inside `evolve_session`, so we spawn first and
    // return the run id from the background task result.  Instead, mirror the
    // pattern used by `run_now`: create the run row here, spawn background work.
    let run = s
        .engine()
        .improvements
        .create_run(
            &session.workspace_id,
            otto_core::domain::ImprovementTrigger::Live,
        )
        .await?;
    let run_id = run.id.clone();
    let ws_id = session.workspace_id.clone();
    let bg_run_id = run_id.clone();
    tokio::spawn(async move {
        if let Err(e) = engine
            .execute_evolve_session(&bg_run_id, &ws_id, &sid)
            .await
        {
            tracing::warn!(session = %sid, "manual evolve-session failed: {e}");
        }
    });
    Ok(Json(RunNowResp { run_id }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::producer::ProposalProducer;
    use crate::proposal::ImprovementProposal;
    use otto_core::auth::BoxFuture;
    use otto_core::domain::{
        ImprovementEditKind, ImprovementRisk, ImprovementTarget, ImprovementTrigger, User,
    };
    use otto_state::{ImprovementsRepo, NewEdit, SessionsRepo, UsersRepo};

    struct NoProducer;
    impl ProposalProducer for NoProducer {
        fn produce<'a>(
            &'a self,
            _: &'a str,
            _: &'a str,
            _: &'a str,
        ) -> BoxFuture<'a, otto_core::Result<ImprovementProposal>> {
            Box::pin(async { Err(Error::Internal("unused".into())) })
        }
    }

    /// `RbacRoleChecker`'s rule (root everywhere, else membership role).
    struct Roles(WorkspacesRepo);
    impl RoleChecker for Roles {
        fn check<'a>(
            &'a self,
            user: &'a User,
            ws: &'a Id,
            min: WorkspaceRole,
        ) -> BoxFuture<'a, otto_core::Result<()>> {
            Box::pin(async move {
                match self.0.role_of(user, ws).await? {
                    Some(r) if r >= min => Ok(()),
                    _ => Err(Error::Forbidden("no".into())),
                }
            })
        }
    }

    #[derive(Clone)]
    struct Ctx {
        engine: Arc<ImprovementEngine>,
        roles: Arc<dyn RoleChecker>,
        workspaces: WorkspacesRepo,
    }
    impl ImproveCtx for Ctx {
        fn engine(&self) -> &Arc<ImprovementEngine> {
            &self.engine
        }
        fn roles(&self) -> &Arc<dyn RoleChecker> {
            &self.roles
        }
        fn workspaces(&self) -> &WorkspacesRepo {
            &self.workspaces
        }
    }

    async fn edit(ctx: &Ctx, ws: &Id, run_id: &Id, status: ImprovementEditStatus) {
        ctx.engine
            .improvements
            .create_edit(NewEdit {
                run_id: run_id.clone(),
                workspace_id: ws.clone(),
                target: ImprovementTarget::Memory,
                target_ref: "MEMORY.md".into(),
                target_path: "/dev/null".into(),
                kind: ImprovementEditKind::Add,
                risk: ImprovementRisk::Low,
                status,
                rationale: String::new(),
                evidence: vec![],
                before_content: None,
                after_content: "x".into(),
                actor: None,
            })
            .await
            .unwrap();
    }

    /// Bob administers `a`, edits `b`, views `c`, and isn't in `d`; every
    /// workspace has self-improvement on and two pending suggestions (plus one
    /// applied in `a`). The bulk routes act only where his role allows and
    /// count the rest as skipped; `d` stays invisible to him.
    #[tokio::test]
    async fn bulk_actions_follow_each_workspace_role() {
        let dir = tempfile::tempdir().unwrap();
        let pool = otto_state::open(&dir.path().join("t.db")).await.unwrap();
        let workspaces = WorkspacesRepo::new(pool.clone());
        let users = UsersRepo::new(pool.clone());
        let root = users.create("root", "pw", "root", true).await.unwrap();
        let bob = users.create("bob", "pw", "bob", false).await.unwrap();
        let (events, _) = tokio::sync::broadcast::channel(4);
        let ctx = Ctx {
            engine: Arc::new(ImprovementEngine {
                improvements: ImprovementsRepo::new(pool.clone()),
                sessions: SessionsRepo::new(pool.clone()),
                workspaces: workspaces.clone(),
                producer: Arc::new(NoProducer),
                events,
                library_root: dir.path().join("library"),
            }),
            roles: Arc::new(Roles(workspaces.clone())),
            workspaces: workspaces.clone(),
        };
        let mut ids = Vec::new();
        for (name, role) in [
            ("a", Some(WorkspaceRole::Admin)),
            ("b", Some(WorkspaceRole::Editor)),
            ("c", Some(WorkspaceRole::Viewer)),
            ("d", None),
        ] {
            let ws = workspaces
                .create(name, dir.path().to_str().unwrap(), &root.id)
                .await
                .unwrap();
            if let Some(role) = role {
                workspaces.set_member(&ws.id, &bob.id, role).await.unwrap();
            }
            workspaces
                .edit_settings(&ws.id, |s| {
                    let cfg = SelfImprovementConfig {
                        enabled: true,
                        ..Default::default()
                    };
                    *s = write_config(s, &cfg);
                })
                .await
                .unwrap();
            let run = ctx
                .engine
                .improvements
                .create_run(&ws.id, ImprovementTrigger::Manual)
                .await
                .unwrap();
            for status in [
                ImprovementEditStatus::Pending,
                ImprovementEditStatus::Pending,
            ] {
                edit(&ctx, &ws.id, &run.id, status).await;
            }
            if name == "a" {
                edit(&ctx, &ws.id, &run.id, ImprovementEditStatus::Applied).await;
            }
            ids.push(ws.id);
        }
        let bob = AuthUser(bob);
        let Json(s) = summary(State(ctx.clone()), Extension(bob.clone()))
            .await
            .ok()
            .unwrap();
        assert_eq!(
            (s.workspaces, s.enabled, s.pending, s.pending_workspaces),
            (3, 3, 6, 3),
            "d is not Bob's to see"
        );

        let Json(r) = reject_all(State(ctx.clone()), Extension(bob.clone()))
            .await
            .ok()
            .unwrap();
        assert_eq!((r.changed, r.workspaces, r.skipped), (4, 2, 1));
        let imp = &ctx.engine.improvements;
        for (ws, left) in ids.iter().zip([0, 0, 2, 2]) {
            let pending = imp
                .list_edits_by_status(ws, ImprovementEditStatus::Pending)
                .await
                .unwrap();
            assert_eq!(pending.len(), left);
        }
        let rejected = imp
            .list_edits_by_status(&ids[0], ImprovementEditStatus::Rejected)
            .await
            .unwrap();
        assert!(rejected
            .iter()
            .all(|e| e.actor.as_deref() == Some(&bob.0.id)));
        let applied = imp
            .list_edits_by_status(&ids[0], ImprovementEditStatus::Applied)
            .await
            .unwrap();
        assert_eq!(applied.len(), 1, "applied edits are left alone");

        let off = SetSelfImprovementEnabledReq { enabled: false };
        let Json(r) = set_enabled_all(
            State(ctx.clone()),
            Extension(bob.clone()),
            Json(off.clone()),
        )
        .await
        .ok()
        .unwrap();
        assert_eq!((r.changed, r.workspaces, r.skipped), (1, 1, 2));
        for (ws, on) in ids.iter().zip([false, true, true, true]) {
            let cfg = effective_config(&workspaces.get(ws).await.unwrap().settings);
            assert_eq!(cfg.enabled, on);
        }
        // Already off: nothing to change.
        let Json(r) = set_enabled_all(State(ctx.clone()), Extension(bob.clone()), Json(off))
            .await
            .ok()
            .unwrap();
        assert_eq!(r.changed, 0);

        // Root turns it off everywhere — and back on, with a fresh schedule.
        let root = AuthUser(root);
        let set = |enabled| {
            set_enabled_all(
                State(ctx.clone()),
                Extension(root.clone()),
                Json(SetSelfImprovementEnabledReq { enabled }),
            )
        };
        let Json(r) = set(false).await.ok().unwrap();
        assert_eq!((r.changed, r.skipped), (3, 0));
        let Json(r) = set(true).await.ok().unwrap();
        assert_eq!(r.changed, 4);
        let cfg = effective_config(&workspaces.get(&ids[3]).await.unwrap().settings);
        assert!(cfg.enabled && cfg.next_run_at.is_some());
    }
}
