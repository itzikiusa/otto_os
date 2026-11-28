//! Scheduled active checks (Nuclei): the person-only grant routes and the
//! scheduled worker's active part. The rules live in
//! [`otto_security::active_grant`]; this file reads the live state they are
//! checked against. `PUT/DELETE .../security/dast/schedule` are human-only by
//! the policy's `/security/dast` prefix rule, so no agent, MCP, share-link or
//! phone credential can create, extend or retarget a grant. Anyone with
//! Agents: Edit can still change the schedule or target, which pauses it.

use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use chrono::Utc;
use otto_core::auth::AuthUser;
use otto_core::domain::{Capability, Feature, NoticeSeverity, User};
use otto_core::{Error, Result};
use otto_security::active_grant::{self, ActiveGrant, ActiveSchedule, GrantRequest, Live, Refusal};
use otto_security::passive::{self, Target};
use otto_security::schedule::{self, ScheduleRow};
use otto_security::{
    CancellationToken, EngineNote, EngineRun, Finding, ScanSettings, SecurityScans, Severity,
};
use serde_json::{json, Value};

use super::security::project_roots;
use super::security_dast::candidates;
use super::ApiResult;
use crate::types::IdeTargetSpec;
use crate::{IdeCtx, IdeNotice};

/// Live state a grant is checked against, read fresh for every check.
struct LiveState {
    target: Option<Target>,
    /// The target's saved run configuration: the URL it serves and its root.
    served: Option<(String, PathBuf)>,
    running: bool,
    schedule: Option<ScheduleRow>,
    run_as_allowed: bool,
}

impl LiveState {
    fn live<'a>(&'a self, addresses: Option<&'a [SocketAddr]>) -> Live<'a> {
        Live {
            target: self.target.as_ref(),
            served_url: self.served.as_ref().map(|(url, _)| url.as_str()),
            running: self.running,
            schedule: self.schedule.as_ref(),
            run_as_allowed: self.run_as_allowed,
            addresses,
            now: Utc::now(),
        }
    }
}

async fn live_state<S: IdeCtx>(ctx: &S, project: &str, roots: &[PathBuf]) -> Result<LiveState> {
    let pool = ctx.pool();
    let mut state = LiveState {
        target: passive::get(&pool, project).await?,
        served: None,
        running: false,
        schedule: schedule::row(&pool, project).await?,
        run_as_allowed: false,
    };
    let (Some(target), Some(row)) = (&state.target, &state.schedule) else {
        return Ok(state);
    };
    let user = match otto_state::UsersRepo::new(pool.clone())
        .get(&row.run_as)
        .await
    {
        Ok(user) => user,
        Err(Error::NotFound(_)) => return Ok(state),
        Err(error) => return Err(error),
    };
    state.run_as_allowed = !user.disabled
        && otto_state::GrantsRepo::new(pool)
            .capability_of(&user, Feature::Agents)
            .await?
            >= Capability::Edit;
    let served = candidates(ctx, roots)
        .await?
        .into_iter()
        .find(|entry| entry.candidate.run_config_id == target.run_config_id);
    if let Some(entry) = served {
        let spec = IdeTargetSpec::local(entry.root.to_string_lossy());
        state.running = entry.candidate.url == target.url
            && ctx
                .authorize_local_target(&user, &spec, false)
                .await
                .is_ok()
            && ctx.ide().environments.has_live_security_run(
                &user,
                &spec,
                &target.run_config_id,
                &entry.raw_url,
            );
        state.served = Some((entry.candidate.url, entry.root));
    }
    Ok(state)
}

/// What `user` may allow now, or the one missing condition.
async fn offer<S: IdeCtx>(
    ctx: &S,
    user: &User,
    project: &str,
    roots: &[PathBuf],
) -> Result<std::result::Result<GrantRequest, String>> {
    let state = live_state(ctx, project, roots).await?;
    let Some(target) = &state.target else {
        return Ok(Err("Confirm an audit target first.".into()));
    };
    let addresses = if active_grant::loopback_host(&target.url) {
        match otto_netguard::resolve_checked_for(&target.url, true).await {
            Ok((_, addresses)) => addresses,
            Err(error) => return Ok(Err(error)),
        }
    } else {
        Vec::new()
    };
    Ok(active_grant::offer(
        target,
        state.served.as_ref().map(|(url, _)| url.as_str()),
        state.running,
        state.schedule.as_ref(),
        &user.id,
        &addresses,
        Utc::now(),
    ))
}

pub(super) async fn view<S: IdeCtx>(
    ctx: &S,
    user: &User,
    project: &str,
    roots: &[PathBuf],
) -> Result<ActiveSchedule> {
    let mut grant = active_grant::get(&ctx.pool(), project).await?;
    // Show a change at once: the pause itself is recorded by the next run.
    if let Some(grant) = grant
        .as_mut()
        .filter(|grant| grant.invalid_reason.is_none())
    {
        let state = live_state(ctx, project, roots).await?;
        if let Err(refusal) = active_grant::check(grant, &state.live(None)) {
            grant.invalid_reason = refusal.pauses().then(|| refusal.message());
        }
    }
    Ok(match offer(ctx, user, project, roots).await? {
        Ok(offer) => ActiveSchedule {
            grant,
            offer: Some(offer),
            blocked: None,
        },
        Err(blocked) => ActiveSchedule {
            grant,
            offer: None,
            blocked: Some(blocked),
        },
    })
}

async fn audit<S: IdeCtx>(ctx: &S, user: Option<&str>, action: &str, project: &str, detail: Value) {
    let entry = otto_state::NewAuditEntry {
        user_id: user.map(str::to_string),
        action: format!("security.active_schedule.{action}"),
        target: Some(format!("ide_project:{project}")),
        detail: Some(detail),
        ip: None,
    };
    if let Err(error) = otto_state::AuditRepo::new(ctx.pool()).insert(entry).await {
        tracing::warn!(%error, "could not audit a scheduled active check");
    }
}

/// `kind`: `paused`, `findings` or `failed` (one notice per failure streak).
async fn notice<S: IdeCtx>(
    ctx: &S,
    grant: &ActiveGrant,
    project: &str,
    kind: &str,
    title: &str,
    body: String,
) {
    let name: Option<String> =
        sqlx::query_scalar("SELECT name FROM ide_named_projects WHERE id = ?")
            .bind(project)
            .fetch_optional(&ctx.pool())
            .await
            .ok()
            .flatten();
    let name = name
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "this project".into());
    let streak = kind == "failed";
    let notice = IdeNotice {
        key: notice_key(project, kind),
        severity: if streak {
            NoticeSeverity::Error
        } else {
            NoticeSeverity::Warn
        },
        title: format!("{title} {name}"),
        body,
        user_id: grant.granted_by.clone(),
    };
    ctx.post_notice(notice, streak).await;
}

fn notice_key(project: &str, kind: &str) -> String {
    format!("security:active:{project}:{kind}")
}

/// `PUT .../security/dast/schedule`: allow (or renew) scheduled active checks
/// for exactly the identity the person was shown.
pub(super) async fn allow<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(request): Json<GrantRequest>,
) -> ApiResult<Json<ActiveGrant>> {
    let roots = project_roots(&ctx, &user, &id).await?;
    let current = offer(&ctx, &user, &id, &roots)
        .await?
        .map_err(Error::Invalid)?;
    if current != request {
        return Err(Error::Conflict(
            "The target or schedule changed. Review and allow again.".into(),
        )
        .into());
    }
    let pool = ctx.pool();
    let renew = active_grant::get(&pool, &id)
        .await?
        .is_some_and(|grant| grant.invalidated_at.is_none());
    let grant = active_grant::grant(&pool, &id, request, &user.id, Utc::now()).await?;
    let detail = json!({
        "url": grant.target_url,
        "addresses": grant.addresses,
        "cron": grant.schedule_cron,
        "timezone": grant.schedule_timezone,
        "template_policy": grant.template_policy,
        "expires_at": grant.expires_at,
    });
    audit(
        &ctx,
        Some(&user.id),
        if renew { "renew" } else { "grant" },
        &id,
        detail,
    )
    .await;
    Ok(Json(grant))
}

/// `DELETE .../security/dast/schedule`: stop scheduled active checks now; a
/// job already preparing is refused at its probe-boundary check.
pub(super) async fn revoke<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    project_roots(&ctx, &user, &id).await?;
    if active_grant::revoke(&ctx.pool(), &id).await? {
        audit(&ctx, Some(&user.id), "revoke", &id, json!({})).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// A refusal and the grant it refused (none once revoked).
type Refused = (Option<ActiveGrant>, Refusal);
type Checked = std::result::Result<(ActiveGrant, LiveState), Refused>;

/// Re-reads the grant and the live state and checks one against the other.
async fn recheck<S: IdeCtx>(
    ctx: &S,
    project: &str,
    roots: &[PathBuf],
    addresses: Option<&[SocketAddr]>,
) -> Result<Checked> {
    let Some(grant) = active_grant::get(&ctx.pool(), project).await? else {
        return Ok(Err((None, Refusal::Revoked)));
    };
    let state = live_state(ctx, project, roots).await?;
    Ok(match active_grant::check(&grant, &state.live(addresses)) {
        Ok(()) => Ok((grant, state)),
        Err(refusal) => Err((Some(grant), refusal)),
    })
}

/// Records a refusal: a pausing one invalidates the grant (audited and
/// announced once); the scan always carries the reason.
async fn refuse<S: IdeCtx>(
    ctx: &S,
    project: &str,
    grant: Option<ActiveGrant>,
    refusal: Refusal,
) -> Result<EngineNote> {
    let reason = refusal.message();
    match grant {
        Some(grant) if refusal.pauses() => {
            if active_grant::pause(&ctx.pool(), project, &grant.granted_at, &reason).await? {
                audit(
                    ctx,
                    None,
                    "invalidated",
                    project,
                    json!({ "reason": reason }),
                )
                .await;
                notice(
                    ctx,
                    &grant,
                    project,
                    "paused",
                    "Scheduled active checks paused for",
                    reason.clone(),
                )
                .await;
            }
        }
        _ => audit(ctx, None, "refused", project, json!({ "reason": reason })).await,
    }
    Ok(EngineNote {
        engine: "nuclei".into(),
        reason,
        failed: false,
    })
}

/// The scheduled worker's active part, after source checks and the passive
/// audit: Nuclei runs only while a person's grant still matches live state,
/// checked here and again right before the probe spawns. A refusal or an
/// engine failure leaves the rest of the scan intact and says why.
pub(super) async fn scheduled<S: IdeCtx, Fut>(
    ctx: &S,
    project: &str,
    roots: &[PathBuf],
    settings: &ScanSettings,
    cancel: &CancellationToken,
    busy: &impl Fn() -> Fut,
    runner: &SecurityScans,
) -> Result<(Option<EngineRun>, Vec<Finding>, Option<EngineNote>)>
where
    Fut: Future<Output = bool>,
{
    let Some(grant) = active_grant::get(&ctx.pool(), project).await? else {
        return Ok((None, Vec::new(), None));
    };
    if let Some(reason) = grant.invalid_reason {
        let note = EngineNote {
            engine: "nuclei".into(),
            reason,
            failed: false,
        };
        return Ok((None, Vec::new(), Some(note)));
    }
    otto_security::wait_for_idle(busy, cancel).await?;
    let (grant, state) = match recheck(ctx, project, roots, None).await? {
        Ok(checked) => checked,
        Err((grant, refusal)) => {
            return Ok((
                None,
                Vec::new(),
                Some(refuse(ctx, project, grant, refusal).await?),
            ));
        }
    };
    let (Some(target), Some((_, root))) = (state.target, state.served) else {
        return Err(Error::Internal("checked grant without a target".into()));
    };
    let refused: Arc<Mutex<Option<Refused>>> = Arc::default();
    let verify = {
        let (ctx, project, roots) = (ctx.clone(), project.to_string(), roots.to_vec());
        let (target, refused) = (target.clone(), refused.clone());
        move |addresses: Vec<SocketAddr>| {
            let (ctx, project, roots) = (ctx.clone(), project.clone(), roots.clone());
            let (target, refused) = (target.clone(), refused.clone());
            async move {
                let outcome = match recheck(&ctx, &project, &roots, Some(&addresses)).await? {
                    Ok((grant, state)) if state.target.as_ref() != Some(&target) => {
                        Err((Some(grant), Refusal::TargetChanged))
                    }
                    other => other.map(|_| ()),
                };
                outcome.map_err(|refusal| {
                    let message = refusal.1.message();
                    *refused.lock().expect("refusal slot") = Some(refusal);
                    Error::Conflict(message)
                })
            }
        }
    };
    let result = runner
        .run_dast(&target, &root, settings, true, cancel, verify)
        .await;
    let refusal = refused.lock().expect("refusal slot").take();
    match result {
        Err(error) if cancel.is_cancelled() => Err(error),
        Err(_) if refusal.is_some() => {
            let (grant, refusal) = refusal.expect("checked above");
            Ok((
                None,
                Vec::new(),
                Some(refuse(ctx, project, grant, refusal).await?),
            ))
        }
        Err(error) => {
            let reason = format!("Active checks failed: {}", error.public_message());
            audit(
                ctx,
                None,
                "run",
                project,
                json!({ "url": target.url, "error": reason }),
            )
            .await;
            notice(
                ctx,
                &grant,
                project,
                "failed",
                "Scheduled active checks failed for",
                reason.clone(),
            )
            .await;
            let note = EngineNote {
                engine: "nuclei".into(),
                reason,
                failed: true,
            };
            Ok((None, Vec::new(), Some(note)))
        }
        Ok((run, findings)) => {
            ctx.end_notice_streak(&notice_key(project, "failed"));
            let serious = findings
                .iter()
                .filter(|finding| matches!(finding.severity, Severity::High | Severity::Critical))
                .count();
            audit(
                ctx,
                None,
                "run",
                project,
                json!({
                    "url": target.url,
                    "addresses": grant.addresses,
                    "nuclei": run.version.version,
                    "nuclei_checked_at": run.version.checked_at,
                    "findings": findings.len(),
                    "high_or_critical": serious,
                }),
            )
            .await;
            if serious > 0 {
                let body = format!("{serious} high or critical findings at {}.", target.url);
                notice(
                    ctx,
                    &grant,
                    project,
                    "findings",
                    "Scheduled active scan found issues in",
                    body,
                )
                .await;
            }
            Ok((Some(run), findings, None))
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    //! The scheduled worker end to end with stub scanners: the real routes,
    //! the real tick, the real pinned proxy and Seatbelt. Each test fails if
    //! the rule it names is removed.
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use axum::http::{Method, StatusCode};
    use otto_core::domain::User;
    use otto_security::{ScanDetail, ScanStatus, ScanTrigger};
    use serde_json::{json, Value};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use crate::http::security_dast::tests::{
        create_project_and_live_run, seed_stub_scanners, send_json, setup_fixture, TestSecurityCtx,
    };
    use crate::IdeCtx;

    /// A dev server that counts passive HEADs and anything else (active probes).
    struct Target {
        url: String,
        heads: Arc<AtomicUsize>,
        probes: Arc<AtomicUsize>,
        server: tokio::task::JoinHandle<()>,
    }

    async fn target() -> Target {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/app", listener.local_addr().unwrap());
        let (heads, probes) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (h, p) = (heads.clone(), probes.clone());
        let server = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let (h, p) = (h.clone(), p.clone());
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let Ok(n) = stream.read(&mut buf).await else {
                        return;
                    };
                    if buf[..n].starts_with(b"HEAD ") {
                        h.fetch_add(1, Ordering::SeqCst);
                    } else if n > 0 {
                        p.fetch_add(1, Ordering::SeqCst);
                    }
                    let _ = stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .await;
                });
            }
        });
        Target {
            url,
            heads,
            probes,
            server,
        }
    }

    struct Fixture {
        _tmp: tempfile::TempDir,
        ctx: TestSecurityCtx,
        app: axum::Router,
        user: User,
        project: String,
        config: String,
        run: String,
        target: Target,
    }

    impl Fixture {
        fn uri(&self, tail: &str) -> String {
            format!("/ide/projects/{}/security{tail}", self.project)
        }

        async fn call(
            &self,
            method: Method,
            tail: &str,
            body: Option<Value>,
        ) -> (StatusCode, Value) {
            send_json(&self.app, &self.user, method, &self.uri(tail), body).await
        }

        /// Saves a schedule through the route an agent token may use, then
        /// moves its arming three hours back so the hourly slot is due.
        async fn set_schedule(&self, cron: &str) {
            let body =
                json!({"action": "set_schedule", "schedule": {"cron": cron, "timezone": "UTC"}});
            let (status, body) = self.call(Method::POST, "/manage", Some(body)).await;
            assert_eq!(status, StatusCode::OK, "set schedule: {body}");
            let past = (chrono::Utc::now() - chrono::Duration::hours(3)).to_rfc3339();
            sqlx::query("UPDATE ide_security_schedules SET armed_at = ? WHERE project_id = ?")
                .bind(past)
                .bind(&self.project)
                .execute(&self.ctx.pool())
                .await
                .unwrap();
        }

        async fn confirm(&self) {
            let body = json!({"run_config_id": self.config, "url": self.target.url, "allow_public": false, "scheduled": true});
            let (status, body) = self.call(Method::PUT, "/dast", Some(body)).await;
            assert_eq!(status, StatusCode::OK, "confirm: {body}");
        }

        async fn view(&self) -> Value {
            let (status, body) = self.call(Method::GET, "/dast", None).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            body["active_schedule"].clone()
        }

        async fn allow(&self) -> Value {
            let offer = self.view().await["offer"].clone();
            assert!(offer.is_object(), "no offer: {}", self.view().await);
            let (status, body) = self
                .call(Method::PUT, "/dast/schedule", Some(offer.clone()))
                .await;
            assert_eq!(status, StatusCode::OK, "allow: {body}");
            offer
        }

        /// Runs the production scheduled tick once and waits for its scan.
        async fn due_scan(&self) -> ScanDetail {
            sqlx::query("UPDATE ide_security_schedules SET last_slot = NULL WHERE project_id = ?")
                .bind(&self.project)
                .execute(&self.ctx.pool())
                .await
                .unwrap();
            let before = self
                .ctx
                .ide()
                .security
                .project(&self.ctx.pool(), &self.project)
                .await
                .unwrap()
                .scans
                .len();
            crate::http::security::scheduled_tick(&self.ctx)
                .await
                .unwrap();
            let security = &self.ctx.ide().security;
            tokio::time::timeout(Duration::from_secs(20), async {
                loop {
                    let scans = security
                        .project(&self.ctx.pool(), &self.project)
                        .await
                        .unwrap()
                        .scans;
                    if let Some(scan) = scans.first().filter(|_| scans.len() > before) {
                        if !matches!(scan.status, ScanStatus::Queued | ScanStatus::Running) {
                            return security
                                .detail(&self.ctx.pool(), &self.project, &scan.id)
                                .await
                                .unwrap();
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            })
            .await
            .expect("the scheduled scan did not finish")
        }

        async fn audit(&self) -> Vec<String> {
            sqlx::query_scalar("SELECT action FROM audit_log WHERE target = ? ORDER BY ts, rowid")
                .bind(format!("ide_project:{}", self.project))
                .fetch_all(&self.ctx.pool())
                .await
                .unwrap()
        }

        async fn finish(self) {
            let _ = self
                .ctx
                .ide()
                .environments
                .stop_run_for_user(&self.user, &self.run)
                .await;
            self.target.server.abort();
        }
    }

    /// A live loopback target confirmed for scheduled passive audits, an
    /// hourly schedule that is due, and stub Trivy + Nuclei.
    async fn fixture() -> Fixture {
        let (tmp, ws, ctx, app, user) = setup_fixture().await;
        seed_stub_scanners(&ctx.data_dir().join("toolchains"));
        let target = target().await;
        let (project, config, run) =
            create_project_and_live_run(&app, &user, &ws, None, "Dev server", &target.url).await;
        let settings = json!({"checks": ["secret"], "excludes": [], "parallelism": 1, "memory_mib": 512, "timeout_secs": 30});
        let (status, _) = send_json(
            &app,
            &user,
            Method::PUT,
            &format!("/ide/projects/{project}/security"),
            Some(settings),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let f = Fixture {
            _tmp: tmp,
            ctx,
            app,
            user,
            project,
            config,
            run,
            target,
        };
        f.confirm().await;
        f.set_schedule("0 * * * *").await;
        f
    }

    fn engines(detail: &ScanDetail) -> Vec<&str> {
        detail
            .scan
            .engines
            .iter()
            .map(|run| run.name.as_str())
            .collect()
    }

    fn skipped(detail: &ScanDetail) -> Vec<(&str, bool)> {
        detail
            .scan
            .skipped
            .iter()
            .map(|note| (note.reason.as_str(), note.failed))
            .collect()
    }

    #[tokio::test]
    async fn a_granted_schedule_runs_active_checks_until_an_agent_changes_the_schedule() {
        let f = fixture().await;
        f.allow().await;

        // A1: the due run is passive + active, recorded and audited.
        let first = f.due_scan().await;
        assert_eq!(
            first.scan.status,
            ScanStatus::Completed,
            "{:?}",
            first.scan.error
        );
        assert_eq!(first.scan.trigger, ScanTrigger::Scheduled);
        assert_eq!(engines(&first), ["trivy", "otto-passive-http", "nuclei"]);
        assert!(first
            .findings
            .iter()
            .any(|finding| finding.rule == "nuclei-ran"));
        assert!(f.target.heads.load(Ordering::SeqCst) >= 1);
        let probes = f.target.probes.load(Ordering::SeqCst);
        assert!(probes >= 1, "Nuclei never reached the target");

        // SEC-1 / A2: the schedule route stays open to agents (Agents: Edit);
        // retiming it pauses the grant instead of carrying it over.
        f.set_schedule("30 * * * *").await;
        let second = f.due_scan().await;
        assert_eq!(
            second.scan.status,
            ScanStatus::Completed,
            "{:?}",
            second.scan.error
        );
        assert_eq!(engines(&second), ["trivy", "otto-passive-http"]);
        assert_eq!(
            skipped(&second),
            [(
                "The schedule changed after you allowed scheduled active checks.",
                false
            )]
        );
        assert_eq!(
            f.target.probes.load(Ordering::SeqCst),
            probes,
            "no active probe after the change"
        );
        let view = f.view().await;
        assert_eq!(
            view["grant"]["invalid_reason"],
            "The schedule changed after you allowed scheduled active checks."
        );
        assert!(
            view["offer"].is_object(),
            "the person can allow the new schedule again"
        );

        // Paused stays paused, quietly: same reason, no second audit row.
        let third = f.due_scan().await;
        assert_eq!(skipped(&third), skipped(&second));
        assert_eq!(
            f.audit().await,
            [
                "security.active_schedule.grant",
                "security.active_schedule.run",
                "security.active_schedule.invalidated",
            ]
        );
        f.finish().await;
    }

    #[tokio::test]
    async fn a_target_confirmed_again_after_the_grant_runs_passive_only() {
        // SEC-2: the grant names the confirmation it was given for.
        let f = fixture().await;
        let offer = f.allow().await;
        f.confirm().await;
        // Settings shows the pause at once; the next run records it.
        assert_eq!(
            f.view().await["grant"]["invalid_reason"],
            "The audit target changed after you allowed scheduled active checks."
        );
        let detail = f.due_scan().await;
        assert_eq!(
            detail.scan.status,
            ScanStatus::Completed,
            "{:?}",
            detail.scan.error
        );
        assert_eq!(engines(&detail), ["trivy", "otto-passive-http"]);
        assert_eq!(
            skipped(&detail),
            [(
                "The audit target changed after you allowed scheduled active checks.",
                false
            )]
        );
        assert_eq!(f.target.probes.load(Ordering::SeqCst), 0);
        // The identity shown before the change can no longer be allowed.
        let (status, body) = f.call(Method::PUT, "/dast/schedule", Some(offer)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        f.finish().await;
    }

    #[tokio::test]
    async fn revoking_while_templates_refresh_keeps_scheduled_nuclei_from_starting() {
        // CONC-1: valid when the active part starts, revoked during preparation.
        let f = fixture().await;
        f.allow().await;
        let nuclei = f.ctx.data_dir().join("toolchains/security-nuclei");
        let (started, release) = (
            nuclei.join("templates/prep-started"),
            nuclei.join("templates/prep-release"),
        );
        std::fs::remove_file(nuclei.join("templates.json")).unwrap();
        std::fs::write(
            nuclei.join("releases/3.11.1/nuclei"),
            format!(
                "#!/bin/sh\ncase \" $* \" in *\" -ut \"*)\n  touch '{}'\n  i=0\n  while [ ! -f '{}' ] && [ \"$i\" -lt 500 ]; do sleep 0.02; i=$((i+1)); done\n  exit 0 ;;\nesac\nexit 7\n",
                started.display(),
                release.display()
            ),
        )
        .unwrap();
        let scan = {
            let f = &f;
            async move { f.due_scan().await }
        };
        let revoke = async {
            tokio::time::timeout(Duration::from_secs(15), async {
                while !started.exists() {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("the template refresh never started");
            let (status, _) = f.call(Method::DELETE, "/dast/schedule", None).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
            std::fs::write(&release, b"go").unwrap();
        };
        let (detail, ()) = tokio::join!(scan, revoke);
        assert_eq!(
            detail.scan.status,
            ScanStatus::Completed,
            "{:?}",
            detail.scan.error
        );
        assert_eq!(engines(&detail), ["trivy", "otto-passive-http"]);
        assert_eq!(
            skipped(&detail),
            [("Scheduled active checks were turned off.", false)]
        );
        assert_eq!(f.target.probes.load(Ordering::SeqCst), 0);
        assert!(f
            .audit()
            .await
            .contains(&"security.active_schedule.refused".to_string()));
        f.finish().await;
    }

    #[tokio::test]
    async fn a_failed_scheduled_active_run_is_shown_as_failed() {
        // OBS-1: the scan keeps its other results and says the active part failed.
        let f = fixture().await;
        f.allow().await;
        let nuclei = f
            .ctx
            .data_dir()
            .join("toolchains/security-nuclei/releases/3.11.1/nuclei");
        std::fs::write(
            &nuclei,
            "#!/bin/sh\necho 'template engine crashed' >&2\nexit 3\n",
        )
        .unwrap();
        let detail = f.due_scan().await;
        assert_eq!(
            detail.scan.status,
            ScanStatus::Completed,
            "{:?}",
            detail.scan.error
        );
        assert_eq!(engines(&detail), ["trivy", "otto-passive-http"]);
        let notes = skipped(&detail);
        assert_eq!(notes.len(), 1);
        assert!(
            notes[0].1 && notes[0].0.starts_with("Active checks failed: "),
            "{notes:?}"
        );
        // A failure is not a reason to pause the grant.
        assert!(f.view().await["grant"]["invalid_reason"].is_null());
        f.finish().await;
    }

    #[tokio::test]
    async fn allowing_needs_loopback_an_hourly_schedule_and_the_exact_offer() {
        // A7: nothing faster than hourly; the offer must match the live state.
        let f = fixture().await;
        f.set_schedule("*/15 * * * *").await;
        let view = f.view().await;
        assert!(view["offer"].is_null());
        assert_eq!(
            view["blocked"],
            "Scheduled active checks can run at most once an hour."
        );
        f.set_schedule("0 * * * *").await;
        let mut offer = f.view().await["offer"].clone();
        offer["addresses"] = json!(["127.0.0.1:1"]);
        let (status, body) = f.call(Method::PUT, "/dast/schedule", Some(offer)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert!(f.view().await["grant"].is_null());
        f.finish().await;

        let (_tmp, ws, ctx, app, user) = setup_fixture().await;
        let (project, config, run) =
            create_project_and_live_run(&app, &user, &ws, None, "LAN", "http://192.168.1.10:8080/")
                .await;
        let uri = format!("/ide/projects/{project}/security/dast");
        let body = json!({"run_config_id": config, "url": "http://192.168.1.10:8080/", "allow_public": false, "scheduled": true});
        assert_eq!(
            send_json(&app, &user, Method::PUT, &uri, Some(body))
                .await
                .0,
            StatusCode::OK
        );
        let (_, view) = send_json(&app, &user, Method::GET, &uri, None).await;
        assert!(view["active_schedule"]["blocked"]
            .as_str()
            .unwrap()
            .starts_with("Only localhost targets"));
        let _ = ctx.ide().environments.stop_run_for_user(&user, &run).await;
    }
}
