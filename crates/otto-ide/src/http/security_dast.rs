use std::path::PathBuf;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use otto_core::auth::AuthUser;
use otto_core::domain::{Capability, Feature, User};
use otto_core::{Error, Result};
use otto_security::passive::{
    self, Candidate, ConfirmTarget, ProjectPassive, RunDastRequest, Target,
};
use otto_security::{JobOutput, ScanSummary, ScanTrigger};

use super::{security::project_roots, ApiResult};
use crate::types::IdeTargetSpec;
use crate::IdeCtx;

pub(super) struct ProjectCandidate {
    pub(super) root: PathBuf,
    pub(super) raw_url: String,
    pub(super) candidate: Candidate,
}

pub(super) async fn candidates<S: IdeCtx>(
    ctx: &S,
    roots: &[PathBuf],
) -> Result<Vec<ProjectCandidate>> {
    let mut candidates = Vec::new();
    for root in roots {
        let spec = IdeTargetSpec::local(root.to_string_lossy());
        for config in ctx
            .ide()
            .environments
            .list_run_configs(&ctx.pool(), &spec)
            .await?
        {
            if let Some(raw) = config.env_overrides.get(passive::CONFIG_KEY) {
                if let Ok((url, public)) = passive::parse_url(raw) {
                    candidates.push(ProjectCandidate {
                        root: root.clone(),
                        raw_url: raw.clone(),
                        candidate: Candidate {
                            run_config_id: config.id,
                            run_config_name: config.name,
                            url: url.to_string(),
                            public,
                        },
                    });
                }
            }
        }
    }
    Ok(candidates)
}

async fn eligible<S: IdeCtx>(
    ctx: &S,
    user: &User,
    project: &str,
    roots: &[PathBuf],
    scheduled: bool,
) -> Result<Option<(Target, PathBuf)>> {
    let Some(target) = passive::get(&ctx.pool(), project).await? else {
        return Ok(None);
    };
    if scheduled && !target.scheduled {
        return Ok(None);
    }
    let current = otto_state::UsersRepo::new(ctx.pool()).get(&user.id).await?;
    if current.disabled
        || otto_state::GrantsRepo::new(ctx.pool())
            .capability_of(&current, Feature::Agents)
            .await?
            < Capability::Edit
    {
        return Ok(None);
    }
    let match_run = candidates(ctx, roots).await?.into_iter().find(|candidate| {
        candidate.candidate.run_config_id == target.run_config_id
            && candidate.candidate.url == target.url
            && candidate.candidate.public == target.allow_public
    });
    let Some(matched) = match_run else {
        return Ok(None);
    };
    let spec = IdeTargetSpec::local(matched.root.to_string_lossy());
    ctx.authorize_local_target(&current, &spec, false).await?;
    if !ctx.ide().environments.has_live_security_run(
        &current,
        &spec,
        &target.run_config_id,
        &matched.raw_url,
    ) {
        return Ok(None);
    }
    Ok(Some((target, matched.root)))
}

pub(super) async fn config<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
) -> ApiResult<Json<ProjectPassive>> {
    let roots = project_roots(&ctx, &user, &id).await?;
    let confirmed = passive::get(&ctx.pool(), &id).await?;
    let candidates = candidates(&ctx, &roots)
        .await?
        .into_iter()
        .map(|entry| entry.candidate)
        .collect();
    let active_schedule = super::security_active::view(&ctx, &user, &id, &roots).await?;
    Ok(Json(ProjectPassive {
        confirmed,
        candidates,
        active_schedule,
    }))
}

pub(super) async fn confirm<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(request): Json<ConfirmTarget>,
) -> ApiResult<Json<Target>> {
    let roots = project_roots(&ctx, &user, &id).await?;
    let candidate = candidates(&ctx, &roots)
        .await?
        .into_iter()
        .find(|entry| entry.candidate.run_config_id == request.run_config_id)
        .ok_or_else(|| Error::NotFound("project run configuration with an audit URL".into()))?;
    Ok(Json(
        passive::confirm(&ctx.pool(), &id, candidate.candidate, request).await?,
    ))
}

pub(super) async fn revoke<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    project_roots(&ctx, &user, &id).await?;
    passive::revoke(&ctx.pool(), &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn verify_dast_consent<S: IdeCtx>(
    ctx: &S,
    user: &User,
    project: &str,
    request: &RunDastRequest,
    expected: Option<&Target>,
) -> Result<(Target, PathBuf)> {
    let roots = project_roots(ctx, user, project).await?;
    let (target, root) = eligible(ctx, user, project, &roots, false)
        .await?
        .ok_or_else(|| {
            Error::Conflict(
                "Confirm this URL and start its saved project run configuration.".into(),
            )
        })?;
    request.verify_target(&target)?;
    if let Some(expected) = expected {
        if &target != expected {
            return Err(Error::Conflict(
                "The confirmed target or its running configuration changed.".into(),
            ));
        }
    }
    Ok((target, root))
}

pub(super) async fn start<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<ScanSummary>)> {
    let request: RunDastRequest = if body.is_empty() {
        RunDastRequest::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|_| Error::Invalid("Invalid DAST run options.".into()))?
    };
    let active = request.active;
    let (expected, _) = verify_dast_consent(&ctx, &user, &id, &request, None).await?;
    let service = ctx.ide().security.clone();
    let runner = service.clone();
    let job_ctx = ctx.clone();
    let project = id.clone();
    let scan = service
        .enqueue_job(
            ctx.pool(),
            id,
            ScanTrigger::Manual,
            move || {
                let ctx = ctx.clone();
                async move { ctx.security_scan_busy().await }
            },
            move |settings, cancel, _| async move {
                let (target, root) =
                    verify_dast_consent(&job_ctx, &user, &project, &request, Some(&expected))
                        .await?;
                let verify_ctx = job_ctx.clone();
                let verify_user = user.clone();
                let verify_project = project.clone();
                let verify_request = request.clone();
                let verify_expected = expected.clone();
                let verify_root = root.clone();
                let (run, findings) = runner
                    .run_dast(&target, &root, &settings, active, &cancel, move |_| {
                        let ctx = verify_ctx.clone();
                        let user = verify_user.clone();
                        let project = verify_project.clone();
                        let request = verify_request.clone();
                        let expected = verify_expected.clone();
                        let expected_root = verify_root.clone();
                        async move {
                            let (_, current_root) = verify_dast_consent(
                                &ctx,
                                &user,
                                &project,
                                &request,
                                Some(&expected),
                            )
                            .await?;
                            if current_root != expected_root {
                                return Err(Error::Conflict(
                                    "The confirmed target or its running configuration changed."
                                        .into(),
                                ));
                            }
                            Ok(())
                        }
                    })
                    .await?;
                Ok((vec![run], findings))
            },
        )
        .await?;
    Ok((StatusCode::ACCEPTED, Json(scan)))
}

/// Scheduled scans run source checks, the passive HTTP header audit when the
/// target opted in, and active Nuclei checks only under a person's grant that
/// still matches live state ([`super::security_active::scheduled`]).
pub(super) async fn enqueue_scheduled<S: IdeCtx>(
    ctx: &S,
    user: User,
    id: String,
) -> Result<ScanSummary> {
    let roots = project_roots(ctx, &user, &id).await?;
    let expected = eligible(ctx, &user, &id, &roots, true).await?;
    let service = ctx.ide().security.clone();
    let worker = service.clone();
    let load_ctx = ctx.clone();
    let job_ctx = ctx.clone();
    let project = id.clone();
    service
        .enqueue_job(
            ctx.pool(),
            id,
            ScanTrigger::Scheduled,
            move || {
                let ctx = load_ctx.clone();
                async move { ctx.security_scan_busy().await }
            },
            move |settings, cancel, busy| async move {
                let roots = project_roots(&job_ctx, &user, &project).await?;
                let (mut runs, mut findings) = worker
                    .scan_sources(&roots, &settings, &cancel, &busy)
                    .await?;
                if let Some((expected, _)) = expected {
                    otto_security::wait_for_idle(&busy, &cancel).await?;
                    if let Some((target, root)) = eligible(&job_ctx, &user, &project, &roots, true)
                        .await?
                        .filter(|(target, _)| {
                            target.confirmed_at == expected.confirmed_at
                                && target.url == expected.url
                        })
                    {
                        match passive::run(&target, &root, &cancel).await {
                            Ok((run, mut extra)) => {
                                runs.push(run);
                                findings.append(&mut extra);
                            }
                            Err(error) => {
                                if cancel.is_cancelled() {
                                    return Err(error);
                                }
                                tracing::warn!(%error, "scheduled passive audit did not complete")
                            }
                        }
                    }
                }
                let (run, mut extra, note) = super::security_active::scheduled(
                    &job_ctx, &project, &roots, &settings, &cancel, &busy, &worker,
                )
                .await?;
                runs.extend(run);
                findings.append(&mut extra);
                Ok(JobOutput {
                    runs,
                    findings,
                    skipped: note.into_iter().collect(),
                })
            },
        )
        .await
}

#[cfg(test)]
pub(super) mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use otto_core::auth::{AuthUser, BoxFuture, RoleChecker};
    use otto_core::domain::{User, WorkspaceRole};
    use otto_core::secrets::SecretStore;
    use otto_core::{Error, Id, Result};
    use otto_security::passive::Target;
    use otto_state::{DbPool, UsersRepo};
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use crate::types::IdeTargetSpec;
    use crate::{IdeCtx, IdeEngine, IdeSshSpec};

    struct NoopSecrets;
    impl SecretStore for NoopSecrets {
        fn put(&self, _k: &str, _v: &str) -> Result<()> {
            Ok(())
        }
        fn get(&self, _k: &str) -> Result<Option<String>> {
            Ok(None)
        }
        fn delete(&self, _k: &str) -> Result<()> {
            Ok(())
        }
    }

    struct AllowRoles;
    impl RoleChecker for AllowRoles {
        fn check<'a>(
            &'a self,
            _user: &'a User,
            _workspace_id: &'a Id,
            _min: WorkspaceRole,
        ) -> BoxFuture<'a, Result<()>> {
            Box::pin(async { Ok(()) })
        }
    }

    #[derive(Clone)]
    pub(crate) struct TestSecurityCtx {
        ide: Arc<IdeEngine>,
        pool: DbPool,
        secrets: Arc<dyn SecretStore>,
        roles: Arc<dyn RoleChecker>,
        data_dir: PathBuf,
    }

    impl IdeCtx for TestSecurityCtx {
        fn ide(&self) -> &Arc<IdeEngine> {
            &self.ide
        }
        fn pool(&self) -> DbPool {
            self.pool.clone()
        }
        fn secrets(&self) -> &Arc<dyn SecretStore> {
            &self.secrets
        }
        fn data_dir(&self) -> &Path {
            &self.data_dir
        }
        fn roles(&self) -> &Arc<dyn RoleChecker> {
            &self.roles
        }
        fn resolve_ssh_connection<'a>(
            &'a self,
            _user: &'a User,
            _connection_id: &'a Id,
        ) -> BoxFuture<'a, Result<IdeSshSpec>> {
            Box::pin(async { Err(Error::Invalid("ssh unused in test".into())) })
        }
        fn authorize_local_target<'a>(
            &'a self,
            _user: &'a User,
            _spec: &'a IdeTargetSpec,
            _write: bool,
        ) -> BoxFuture<'a, Result<()>> {
            Box::pin(async { Ok(()) })
        }
    }

    pub(crate) async fn send_json(
        app: &axum::Router,
        user: &User,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let builder = Request::builder()
            .method(method)
            .uri(uri)
            .extension(AuthUser(user.clone()));
        let req = match body {
            Some(payload) => builder
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let parsed = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, parsed)
    }

    pub(crate) async fn setup_fixture() -> (
        tempfile::TempDir,
        PathBuf,
        TestSecurityCtx,
        axum::Router,
        User,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let ws = root.join("workspace");
        let data_dir = root.join("data");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(ws.join("sample.txt"), "fixture\n").unwrap();
        let db_path = data_dir.join("test.sqlite");
        let pool = otto_state::open(&db_path).await.unwrap();
        let user = UsersRepo::new(pool.clone())
            .create("owner", "unused", "Owner", true)
            .await
            .unwrap();
        let ctx = TestSecurityCtx {
            ide: Arc::new(IdeEngine::new(data_dir.clone())),
            pool,
            secrets: Arc::new(NoopSecrets),
            roles: Arc::new(AllowRoles),
            data_dir,
        };
        let app = crate::http::api_router::<TestSecurityCtx>().with_state(ctx.clone());
        (tmp, ws, ctx, app, user)
    }

    pub(crate) async fn create_project_and_live_run(
        app: &axum::Router,
        user: &User,
        ws: &Path,
        project_id: Option<&str>,
        config_name: &str,
        url: &str,
    ) -> (String, String, String) {
        let pid = match project_id {
            Some(existing) => existing.to_string(),
            None => {
                let (status, body) = send_json(
                    app,
                    user,
                    Method::POST,
                    "/ide/projects",
                    Some(json!({
                        "name": "Security Test Project",
                        "primary_root": ws.to_string_lossy(),
                    })),
                )
                .await;
                assert_eq!(status, StatusCode::OK, "create project failed: {body}");
                body["project"]["id"].as_str().unwrap().to_string()
            }
        };

        let target = json!({ "kind": "local", "root": ws.to_string_lossy() });
        let (status, cfg_body) = send_json(
            app,
            user,
            Method::POST,
            "/ide/run-configs",
            Some(json!({
                "target": target,
                "name": config_name,
                "runner": "shell",
                "command": "/bin/sh",
                "args": ["-c", "sleep 30"],
                "env_overrides": {
                    "OTTO_SECURITY_URL": url,
                },
            })),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "create run config failed: {cfg_body}"
        );
        let config_id = cfg_body["config"]["id"].as_str().unwrap().to_string();

        let (status, run_body) = send_json(
            app,
            user,
            Method::POST,
            "/ide/runs",
            Some(json!({
                "target": target,
                "config_id": config_id,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "start run failed: {run_body}");
        let run_id = run_body["run"]["id"].as_str().unwrap().to_string();
        (pid, config_id, run_id)
    }

    #[tokio::test]
    async fn start_handler_rejects_stale_target_a_identity_after_b_replaces_a() {
        // SEC-2: exercise the actual HTTP start handler. Window 1 displays
        // confirmed target A; window 2 confirms replacement target B; window 1
        // posts active DAST with A's displayed identity. Must return HTTP 409
        // and enqueue zero scans.
        let (_tmp, ws, ctx, app, user) = setup_fixture().await;
        let (project_id, cfg_a, run_a) = create_project_and_live_run(
            &app,
            &user,
            &ws,
            None,
            "Target A",
            "http://127.0.0.1:41001/a",
        )
        .await;
        let (_, cfg_b, run_b) = create_project_and_live_run(
            &app,
            &user,
            &ws,
            Some(&project_id),
            "Target B",
            "http://127.0.0.1:41002/b",
        )
        .await;

        let dast_uri = format!("/ide/projects/{project_id}/security/dast");
        let run_uri = format!("/ide/projects/{project_id}/security/dast/run");

        // Confirm target A and capture the identity displayed in window 1's prompt.
        let (status, body_a) = send_json(
            &app,
            &user,
            Method::PUT,
            &dast_uri,
            Some(json!({
                "run_config_id": cfg_a,
                "url": "http://127.0.0.1:41001/a",
                "allow_public": false,
                "scheduled": false,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "confirm A failed: {body_a}");
        let target_a: Target = serde_json::from_value(body_a).unwrap();

        // Another window confirms replacement target B while window 1's prompt is open.
        let (status, body_b) = send_json(
            &app,
            &user,
            Method::PUT,
            &dast_uri,
            Some(json!({
                "run_config_id": cfg_b,
                "url": "http://127.0.0.1:41002/b",
                "allow_public": false,
                "scheduled": false,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "confirm B failed: {body_b}");

        // Window 1 submits active DAST carrying target A's displayed confirmation.
        let (status, err_body) = send_json(
            &app,
            &user,
            Method::POST,
            &run_uri,
            Some(json!({
                "active": true,
                "run_config_id": target_a.run_config_id,
                "url": target_a.url,
                "confirmed_at": target_a.confirmed_at,
            })),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "stale target A identity after B replaces A must return 409 Conflict, got: {err_body}"
        );
        assert!(
            err_body["message"]
                .as_str()
                .is_some_and(|m| m.contains("The confirmed target changed")),
            "409 must come from target identity mismatch (not missing live run), got: {err_body}"
        );

        // Bare { "active": true } without identity must return 400 Bad Request.
        let (status, bad_body) = send_json(
            &app,
            &user,
            Method::POST,
            &run_uri,
            Some(json!({ "active": true })),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "bare active request without identity must return 400 Bad Request, got: {bad_body}"
        );

        // Neither rejected request may enqueue a scan.
        let scans = ctx
            .ide()
            .security
            .project(&ctx.pool(), &project_id)
            .await
            .unwrap()
            .scans;
        assert!(
            scans.is_empty(),
            "rejected active DAST requests must enqueue zero scans, found: {scans:?}"
        );

        let _ = ctx
            .ide()
            .environments
            .stop_run_for_user(&user, &run_a)
            .await;
        let _ = ctx
            .ide()
            .environments
            .stop_run_for_user(&user, &run_b)
            .await;
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn seed_stub_scanners(toolchains: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let stamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        // Cached Trivy stub producing an empty JSON report.
        let trivy_dir = toolchains.join("security-trivy");
        let trivy_bin = trivy_dir.join("releases/1.2.3/trivy");
        std::fs::create_dir_all(trivy_bin.parent().unwrap()).unwrap();
        std::fs::write(
            &trivy_bin,
            "#!/bin/sh\nwhile [ \"$1\" != --output ]; do shift; done\nprintf '%s' '{\"Results\":[]}' > \"$2\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&trivy_bin, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(
            trivy_dir.join("engine.json"),
            serde_json::to_vec(&json!({
                "version": "1.2.3",
                "updated_at": stamp,
                "checked_at": stamp,
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            trivy_dir.join("release-attempt.json"),
            serde_json::to_vec(&stamp).unwrap(),
        )
        .unwrap();

        // Cached Nuclei stub + allowlisted templates that would send GET via
        // -proxy and emit a finding if the scheduled worker ever launched Nuclei.
        let nuclei_dir = toolchains.join("security-nuclei");
        let nuclei_bin = nuclei_dir.join("releases/3.11.1/nuclei");
        let templates = nuclei_dir.join("templates");
        std::fs::create_dir_all(nuclei_bin.parent().unwrap()).unwrap();
        for sub in [
            "http/exposures/configs",
            "http/exposures/files",
            "http/exposures/apis",
            "http/exposures/logs",
        ] {
            let dir = templates.join(sub);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("stub.yaml"), "id: stub\n").unwrap();
        }
        std::fs::write(
            &nuclei_bin,
            r#"#!/bin/sh
TARGET=""
PROXY=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -u) TARGET="$2"; shift 2 ;;
    -proxy) PROXY="$2"; shift 2 ;;
    *) shift ;;
  esac
done
PROXY_PORT="${PROXY##*:}"
printf 'GET %s HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n' "$TARGET" | nc -w 2 127.0.0.1 "$PROXY_PORT" >/dev/null 2>&1 || true
printf '%s\n' '{"template-id":"nuclei-ran","matched-at":"'"$TARGET"'/nuclei","info":{"name":"Nuclei Ran","severity":"low","description":"stub"}}'
"#,
        )
        .unwrap();
        std::fs::set_permissions(&nuclei_bin, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(
            nuclei_dir.join("engine.json"),
            serde_json::to_vec(&json!({
                "version": "3.11.1",
                "updated_at": stamp,
                "checked_at": stamp,
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            nuclei_dir.join("release-attempt.json"),
            serde_json::to_vec(&stamp).unwrap(),
        )
        .unwrap();
        std::fs::write(
            nuclei_dir.join("templates.json"),
            serde_json::to_vec(&json!({ "checked_at": stamp })).unwrap(),
        )
        .unwrap();
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn scheduled_tick_runs_passive_only_and_never_starts_nuclei_even_with_legacy_flag() {
        // SEC-1 & OBS-1: drive a real retimed/due schedule with a live passive
        // target whose SQLite row has legacy scheduled_active = 1 and cached
        // Nuclei ready. Assert passive HTTP HEAD audit runs and Nuclei never starts.
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let (_tmp, ws, ctx, app, user) = setup_fixture().await;
        seed_stub_scanners(&ctx.data_dir().join("toolchains"));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let head_hits = Arc::new(AtomicUsize::new(0));
        let non_head_hits = Arc::new(AtomicUsize::new(0));
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
        let server = {
            let head_hits = head_hits.clone();
            let non_head_hits = non_head_hits.clone();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        biased;
                        _ = &mut stop_rx => break,
                        accepted = listener.accept() => {
                            let Ok((mut stream, _)) = accepted else { break };
                            let head_hits = head_hits.clone();
                            let non_head_hits = non_head_hits.clone();
                            tokio::spawn(async move {
                                let mut buf = [0u8; 1024];
                                let Ok(n) = stream.read(&mut buf).await else { return };
                                let req = String::from_utf8_lossy(&buf[..n]);
                                if req.starts_with("HEAD ") {
                                    head_hits.fetch_add(1, Ordering::SeqCst);
                                } else if n > 0 {
                                    non_head_hits.fetch_add(1, Ordering::SeqCst);
                                }
                                let _ = stream
                                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                                    .await;
                            });
                        }
                    }
                }
            })
        };

        let url = format!("http://{addr}/app");
        let (project_id, config_id, run_id) =
            create_project_and_live_run(&app, &user, &ws, None, "Scheduled Target", &url).await;

        // Restrict source checks to Secret (stubbed Trivy) so the scheduled job is fast and offline.
        let (status, body) = send_json(
            &app,
            &user,
            Method::PUT,
            &format!("/ide/projects/{project_id}/security"),
            Some(json!({
                "checks": ["secret"],
                "excludes": [],
                "parallelism": 1,
                "memory_mib": 512,
                "timeout_secs": 30,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "configure scan failed: {body}");

        // Confirm target for scheduled passive audits, then set legacy scheduled_active = 1 directly.
        let (status, body) = send_json(
            &app,
            &user,
            Method::PUT,
            &format!("/ide/projects/{project_id}/security/dast"),
            Some(json!({
                "run_config_id": config_id,
                "url": url,
                "allow_public": false,
                "scheduled": true,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "confirm passive failed: {body}");
        sqlx::query(
            "UPDATE ide_security_passive_targets SET scheduled_active = 1 WHERE project_id = ?",
        )
        .bind(&project_id)
        .execute(&ctx.pool())
        .await
        .unwrap();

        // Arm/retime a schedule through the real schedule-management route and make it due now.
        let (status, body) = send_json(
            &app,
            &user,
            Method::POST,
            &format!("/ide/projects/{project_id}/security/manage"),
            Some(json!({
                "action": "set_schedule",
                "schedule": {
                    "cron": "*/5 * * * *",
                    "timezone": "UTC",
                },
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "set schedule failed: {body}");
        let past = (chrono::Utc::now() - chrono::Duration::minutes(10)).to_rfc3339();
        sqlx::query(
            "UPDATE ide_security_schedules SET armed_at = ?, last_slot = NULL WHERE project_id = ?",
        )
        .bind(&past)
        .bind(&project_id)
        .execute(&ctx.pool())
        .await
        .unwrap();

        // Drive the real production scheduled worker.
        crate::http::security::scheduled_tick(&ctx).await.unwrap();

        let scan_id = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let state = ctx
                    .ide()
                    .security
                    .project(&ctx.pool(), &project_id)
                    .await
                    .unwrap();
                if let Some(scan) = state.scans.first() {
                    if scan.status != otto_security::ScanStatus::Queued
                        && scan.status != otto_security::ScanStatus::Running
                    {
                        return scan.id.clone();
                    }
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("scheduled scan did not finish within 15s");

        let detail = ctx
            .ide()
            .security
            .detail(&ctx.pool(), &project_id, &scan_id)
            .await
            .unwrap();
        let _ = stop_tx.send(());
        let _ = server.await;
        let _ = ctx
            .ide()
            .environments
            .stop_run_for_user(&user, &run_id)
            .await;

        assert_eq!(
            detail.scan.status,
            otto_security::ScanStatus::Completed,
            "scheduled scan failed: {:?}",
            detail.scan.error
        );
        assert!(
            detail
                .scan
                .engines
                .iter()
                .any(|run| run.name == "otto-passive-http"),
            "scheduled scan must run the passive HTTP HEAD audit; engines: {:?}",
            detail.scan.engines
        );
        assert!(
            head_hits.load(Ordering::SeqCst) >= 1,
            "passive audit must send at least one HEAD request to the live target"
        );
        assert!(
            !detail.scan.engines.iter().any(|run| run.name == "nuclei"),
            "scheduled scan must NEVER launch Nuclei even with legacy scheduled_active = 1; engines: {:?}",
            detail.scan.engines
        );
        assert_eq!(
            non_head_hits.load(Ordering::SeqCst),
            0,
            "scheduled scan must never send active non-HEAD probes to the target"
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn revoking_through_the_route_during_template_refresh_blocks_the_active_nuclei_probe() {
        // RC5-006 / CONC-1 end to end: a human starts an active scan for the
        // exact confirmed live target, then revokes it through DELETE
        // /security/dast while the queued job is refreshing templates. The
        // handler's probe-boundary consent check must stop Nuclei from
        // spawning its probe: the scan fails with the consent error and the
        // target sees no request.
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;

        let (_tmp, ws, ctx, app, user) = setup_fixture().await;
        let toolchains = ctx.data_dir().join("toolchains");
        seed_stub_scanners(&toolchains);
        let nuclei_dir = toolchains.join("security-nuclei");
        let templates = nuclei_dir.join("templates");
        let prep_started = templates.join("prep-started");
        let prep_release = templates.join("prep-release");
        // A stale template marker makes the job refresh (-ut) first; the stub
        // holds the refresh open until the test has revoked consent.
        std::fs::remove_file(nuclei_dir.join("templates.json")).unwrap();
        std::fs::write(
            nuclei_dir.join("releases/3.11.1/nuclei"),
            format!(
                r#"#!/bin/sh
case " $* " in *" -ut "*)
  touch '{started}'
  i=0
  while [ ! -f '{release}' ] && [ "$i" -lt 500 ]; do sleep 0.02; i=$((i+1)); done
  exit 0 ;;
esac
TARGET=""
PROXY=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -u) TARGET="$2"; shift 2 ;;
    -proxy) PROXY="$2"; shift 2 ;;
    *) shift ;;
  esac
done
printf 'GET %s HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n' "$TARGET" | nc -w 2 127.0.0.1 "${{PROXY##*:}}" >/dev/null 2>&1 || true
printf '%s\n' '{{"template-id":"nuclei-ran","matched-at":"'"$TARGET"'/nuclei","info":{{"name":"Nuclei Ran","severity":"low","description":"stub"}}}}'
"#,
                started = prep_started.display(),
                release = prep_release.display(),
            ),
        )
        .unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let server = {
            let hits = hits.clone();
            tokio::spawn(async move {
                while let Ok((_stream, _)) = listener.accept().await {
                    hits.fetch_add(1, Ordering::SeqCst);
                }
            })
        };

        let url = format!("http://{addr}/app");
        let (project_id, config_id, run_id) =
            create_project_and_live_run(&app, &user, &ws, None, "Active Target", &url).await;
        let dast_uri = format!("/ide/projects/{project_id}/security/dast");
        let (status, body) = send_json(
            &app,
            &user,
            Method::PUT,
            &dast_uri,
            Some(json!({
                "run_config_id": config_id,
                "url": url,
                "allow_public": false,
                "scheduled": false,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "confirm failed: {body}");
        let target: Target = serde_json::from_value(body).unwrap();

        let (status, body) = send_json(
            &app,
            &user,
            Method::POST,
            &format!("{dast_uri}/run"),
            Some(json!({
                "active": true,
                "run_config_id": target.run_config_id,
                "url": target.url,
                "confirmed_at": target.confirmed_at,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "start failed: {body}");
        let scan_id = body["id"].as_str().unwrap().to_string();

        tokio::time::timeout(Duration::from_secs(15), async {
            while !prep_started.exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the queued active job never started its template refresh");
        let (status, body) = send_json(&app, &user, Method::DELETE, &dast_uri, None).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "revoke failed: {body}");
        std::fs::write(&prep_release, b"go").unwrap();

        let detail = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let detail = ctx
                    .ide()
                    .security
                    .detail(&ctx.pool(), &project_id, &scan_id)
                    .await
                    .unwrap();
                if detail.scan.status != otto_security::ScanStatus::Queued
                    && detail.scan.status != otto_security::ScanStatus::Running
                {
                    return detail;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("active scan did not finish within 15s");
        let _ = ctx
            .ide()
            .environments
            .stop_run_for_user(&user, &run_id)
            .await;
        server.abort();

        assert_eq!(detail.scan.status, otto_security::ScanStatus::Failed);
        assert!(
            detail
                .scan
                .error
                .as_deref()
                .is_some_and(|error| error.contains("Confirm this URL")),
            "the scan must stop at the consent check, got: {:?}",
            detail.scan.error
        );
        assert_eq!(
            hits.load(Ordering::SeqCst),
            0,
            "a revoked target must receive no active probe"
        );
    }
}
