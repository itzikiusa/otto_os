//! Governed asynchronous SFTP transfers. Partial files have unique names and
//! only become the destination after successful transfer and reauthorization.
use crate::http::{
    check_conn_role, expand_home, open_sftp_with_connection, ApiErr, ConnectionsCtx,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use otto_core::{
    api::{SftpTransfer, SftpTransferReq},
    auth::AuthUser,
    domain::WorkspaceRole,
    Error, Id,
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::sync::watch;

struct Job {
    user: Id,
    connection: Id,
    snapshot: SftpTransfer,
    cancel: watch::Sender<bool>,
    touched: Instant,
}
#[derive(Default)]
pub(crate) struct Transfers(Mutex<HashMap<Id, Job>>);
impl Transfers {
    fn insert(
        &self,
        user: Id,
        connection: Id,
        snapshot: SftpTransfer,
    ) -> Result<watch::Receiver<bool>, Error> {
        let mut jobs = self.0.lock().unwrap();
        jobs.retain(|_, j| {
            j.snapshot.status == "running"
                || j.snapshot.status == "finalizing"
                || j.touched.elapsed() < Duration::from_secs(1800)
        });
        if jobs.len() >= 128 {
            let oldest = jobs
                .iter()
                .filter(|(_, j)| !matches!(j.snapshot.status.as_str(), "running" | "finalizing"))
                .min_by_key(|(_, j)| j.touched)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                jobs.remove(&id);
            } else {
                return Err(Error::Conflict("too many active transfers".into()));
            }
        }
        let (cancel, rx) = watch::channel(false);
        jobs.insert(
            snapshot.id.clone(),
            Job {
                user,
                connection,
                snapshot,
                cancel,
                touched: Instant::now(),
            },
        );
        Ok(rx)
    }
    fn update(&self, snapshot: &SftpTransfer) {
        if let Some(job) = self.0.lock().unwrap().get_mut(&snapshot.id) {
            job.snapshot = snapshot.clone();
            job.touched = Instant::now();
        }
    }
    fn list(&self, user: &str, conn: &str) -> Vec<SftpTransfer> {
        self.0
            .lock()
            .unwrap()
            .values()
            .filter(|j| j.user == user && j.connection == conn)
            .map(|j| j.snapshot.clone())
            .collect()
    }
    fn cancel(&self, user: &str, conn: &str, id: &str) -> Result<(), Error> {
        let jobs = self.0.lock().unwrap();
        let job = jobs
            .get(id)
            .filter(|j| j.user == user && j.connection == conn)
            .ok_or_else(|| Error::NotFound("transfer".into()))?;
        if job.snapshot.status == "running" {
            let _ = job.cancel.send(true);
        }
        Ok(())
    }
}

async fn authorize_transfer<S: ConnectionsCtx>(
    ctx: &S,
    actor: &Id,
    id: &Id,
    params: &serde_json::Value,
    operation: &str,
) -> Result<(), Error> {
    let user = otto_state::UsersRepo::new(ctx.pool()).get(actor).await?;
    if user.disabled {
        return Err(Error::Forbidden("account disabled".into()));
    }
    let conn = ctx.connections().get(id).await?;
    check_conn_role(ctx, &user, &conn, WorkspaceRole::Editor).await?;
    let governed = ctx.connections().is_enforced(id).await?;
    if governed && !user.is_root {
        return Err(Error::Forbidden(
            "daemon-local transfer paths require root for governed connections".into(),
        ));
    }
    if !governed && crate::http::owner_private_enabled(ctx).await {
        crate::http::require_conn_owner_or_root(&user, &conn)?;
    }
    ctx.connections().authorize(id, actor, operation).await?;
    if &conn.params != params {
        return Err(Error::Conflict(
            "connection settings changed during transfer".into(),
        ));
    }
    Ok(())
}

pub(crate) async fn list<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> Result<Json<Vec<SftpTransfer>>, ApiErr> {
    let conn = ctx.connections().get(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor).await?;
    let mut visible = Vec::new();
    for transfer in ctx.connections().transfers.list(&user.id, &id) {
        ctx.connections()
            .authorize(
                &id,
                &user.id,
                if transfer.direction == "upload" {
                    "sftp_write"
                } else {
                    "sftp_read"
                },
            )
            .await?;
        visible.push(transfer);
    }
    Ok(Json(visible))
}
pub(crate) async fn cancel<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, transfer)): Path<(Id, Id)>,
) -> Result<StatusCode, ApiErr> {
    // The initiating actor can always stop its own work, including after access
    // revocation; this endpoint cannot discover another actor's transfer.
    ctx.connections()
        .transfers
        .cancel(&user.id, &id, &transfer)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn start<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<SftpTransferReq>,
) -> Result<(StatusCode, Json<SftpTransfer>), ApiErr> {
    let upload = match req.direction.as_str() {
        "upload" => true,
        "download" => false,
        _ => return Err(Error::Invalid("direction must be upload or download".into()).into()),
    };
    if req.local_path.trim().is_empty() || req.remote_path.trim().is_empty() {
        return Err(Error::Invalid("local_path and remote_path are required".into()).into());
    }
    let timeout = req.timeout_secs.unwrap_or(600);
    if !(1..=600).contains(&timeout) {
        return Err(Error::Invalid("timeout_secs must be between 1 and 600".into()).into());
    }
    if ctx.connections().is_enforced(&id).await? {
        let current = otto_state::UsersRepo::new(ctx.pool()).get(&user.id).await?;
        if !current.is_root || current.disabled {
            return Err(Error::Forbidden(
                "daemon-local transfer paths require root for governed connections".into(),
            )
            .into());
        }
    }
    let operation = if upload { "sftp_write" } else { "sftp_read" };
    let (connection, sftp) =
        open_sftp_with_connection(&ctx, &user, &id, WorkspaceRole::Editor, operation).await?;
    let mut local = PathBuf::from(expand_home(&req.local_path));
    if !upload && local.is_dir() {
        let name = std::path::Path::new(&req.remote_path)
            .file_name()
            .ok_or_else(|| Error::Invalid("remote_path must name a file".into()))?;
        local.push(name);
    }
    let total = if upload {
        let metadata = tokio::fs::metadata(&local)
            .await
            .map_err(|e| Error::Invalid(format!("local file: {e}")))?;
        if !metadata.is_file() {
            return Err(Error::Invalid("upload source must be a file".into()).into());
        }
        Some(metadata.len())
    } else {
        if local.exists() {
            return Err(Error::Conflict(
                "download destination already exists; choose a new filename".into(),
            )
            .into());
        }
        None
    };
    let transfer_id = otto_core::new_id();
    let stage_local = local.with_file_name(format!(".otto-transfer-{transfer_id}.part"));
    let remote_parent = req
        .remote_path
        .rsplit_once('/')
        .map(|(p, _)| if p.is_empty() { "/" } else { p })
        .unwrap_or(".");
    let stage_remote = format!("{remote_parent}/.otto-transfer-{transfer_id}.part");
    let snapshot = SftpTransfer {
        id: transfer_id,
        direction: req.direction,
        local_path: local.to_string_lossy().into_owned(),
        remote_path: req.remote_path,
        status: "running".into(),
        bytes: 0,
        total_bytes: total,
        elapsed_secs: 0,
        error: None,
    };
    let mut cancellation =
        ctx.connections()
            .transfers
            .insert(user.id.clone(), id.clone(), snapshot.clone())?;
    let initial = snapshot.clone();
    tokio::spawn(async move {
        let mut snapshot = snapshot;
        let started = Instant::now();
        let stage_local_str = stage_local.to_string_lossy().into_owned();
        let work = async {
            if !upload {
                if let Some(parent) = local.parent().filter(|p| !p.as_os_str().is_empty()) {
                    tokio::fs::create_dir_all(parent)
                        .await
                        .map_err(|e| Error::Invalid(format!("local directory: {e}")))?;
                }
            }
            let transfer = async {
                if upload {
                    sftp.upload(&snapshot.local_path, &stage_remote).await
                } else {
                    sftp.download(&snapshot.remote_path, &stage_local_str).await
                }
            };
            tokio::pin!(transfer);
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    biased;
                    result = &mut transfer => { result?; break; }
                    _ = interval.tick() => {
                        authorize_transfer(&ctx, &user.id, &id, &connection.params, operation).await?;
                        snapshot.elapsed_secs = started.elapsed().as_secs();
                        snapshot.bytes = if upload {
                            // The partial remote file reports bytes actually received;
                            // an unavailable progress probe never stalls cancellation.
                            let parent = stage_remote.rsplit_once('/').map(|(p, _)| p).unwrap_or(".");
                            let name = stage_remote.rsplit('/').next().unwrap_or("");
                            match tokio::time::timeout(Duration::from_millis(500), sftp.list(parent)).await {
                                Ok(Ok(entries)) => entries.iter().find(|e| e.name == name).map(|e| e.size).unwrap_or(snapshot.bytes),
                                _ => snapshot.bytes,
                            }
                        } else { tokio::fs::metadata(&stage_local).await.map(|m| m.len()).unwrap_or(snapshot.bytes) };
                        ctx.connections().transfers.update(&snapshot);
                    }
                }
            }
            Ok::<(), Error>(())
        };
        let mut result = tokio::select! {
            biased;
            _ = cancellation.changed() => Err(Error::Conflict("transfer cancelled".into())),
            result = tokio::time::timeout(Duration::from_secs(timeout), work) => result.unwrap_or_else(|_| Err(Error::Upstream("transfer timed out".into()))),
        };
        let mut outcome_unknown = false;
        if result.is_ok() {
            // Publication is a commit point. Revalidate immediately beforehand,
            // then await it without cancellation: a stopped await cannot undo a
            // remote rename or an in-flight local filesystem operation.
            result = async {
                authorize_transfer(&ctx, &user.id, &id, &connection.params, operation).await?;
                if *cancellation.borrow() { return Err(Error::Conflict("transfer cancelled".into())) }
                snapshot.status = "finalizing".into();
                ctx.connections().transfers.update(&snapshot);
                if upload {
                    if let Err(error) = sftp.rename(&stage_remote, &snapshot.remote_path).await {
                        outcome_unknown = true;
                        return Err(Error::Upstream(format!("could not confirm remote publication; inspect the destination before retrying: {error}")));
                    }
                    snapshot.bytes = snapshot.total_bytes.unwrap_or(snapshot.bytes);
                } else {
                    snapshot.bytes = tokio::fs::metadata(&stage_local).await.map(|m| m.len()).unwrap_or(snapshot.bytes);
                    snapshot.total_bytes = Some(snapshot.bytes);
                    tokio::fs::hard_link(&stage_local, &local).await.map_err(|e| Error::Conflict(format!("cannot finalize download: {e}")))?;
                }
                Ok(())
            }.await;
        }
        snapshot.elapsed_secs = started.elapsed().as_secs();
        snapshot.status = match &result {
            Ok(()) => "completed",
            Err(_) if outcome_unknown => "outcome_unknown",
            Err(_) if *cancellation.borrow() => "cancelled",
            Err(_) if started.elapsed() >= Duration::from_secs(timeout) => "timed_out",
            Err(_) => "failed",
        }
        .into();
        snapshot.error = result.err().map(|e| e.to_string());
        ctx.connections().transfers.update(&snapshot);
        if upload && snapshot.status != "completed" {
            let _ = tokio::time::timeout(Duration::from_secs(5), sftp.remove(&stage_remote)).await;
        }
        if !upload {
            let _ = tokio::fs::remove_file(stage_local).await;
        }
    });
    Ok((StatusCode::ACCEPTED, Json(initial)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_is_actor_and_connection_scoped() {
        let jobs = Transfers::default();
        let snapshot = SftpTransfer {
            id: "job".into(),
            direction: "download".into(),
            local_path: "local".into(),
            remote_path: "remote".into(),
            status: "running".into(),
            bytes: 0,
            total_bytes: None,
            elapsed_secs: 0,
            error: None,
        };
        let rx = jobs
            .insert("alice".into(), "conn".into(), snapshot)
            .unwrap();
        assert!(jobs.cancel("bob", "conn", "job").is_err());
        assert!(jobs.cancel("alice", "other", "job").is_err());
        assert!(!*rx.borrow());
        assert!(jobs.list("bob", "conn").is_empty());
        jobs.cancel("alice", "conn", "job").unwrap();
        assert!(*rx.borrow());
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use crate::http::open_sftp;
    use crate::service::{ConnectionsService, Spawner};
    use otto_core::{
        auth::{BoxFuture, RoleChecker},
        domain::{Connection, Session, User},
        Result,
    };
    use otto_state::{ConnectionSectionsRepo, ConnectionsRepo, SqlitePool};
    use std::sync::Arc;
    struct Secrets;
    impl otto_core::secrets::SecretStore for Secrets {
        fn put(&self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn get(&self, _: &str) -> Result<Option<String>> {
            Ok(None)
        }
        fn delete(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }
    struct Roles;
    impl RoleChecker for Roles {
        fn check<'a>(
            &'a self,
            _: &'a User,
            _: &'a Id,
            _: WorkspaceRole,
        ) -> BoxFuture<'a, Result<()>> {
            Box::pin(async { Ok(()) })
        }
    }
    struct Spawn;
    impl Spawner for Spawn {
        fn spawn_connection<'a>(
            &'a self,
            _: &'a Id,
            _: &'a Id,
            _: &'a Connection,
            _: otto_pty::CommandSpec,
            _: Option<String>,
            _: Option<String>,
        ) -> BoxFuture<'a, Result<Session>> {
            Box::pin(async { Err(Error::Invalid("unused".into())) })
        }
    }
    #[derive(Clone)]
    struct Ctx {
        svc: Arc<ConnectionsService>,
        pool: SqlitePool,
        roles: Arc<dyn RoleChecker>,
        spawner: Arc<dyn Spawner>,
    }
    impl ConnectionsCtx for Ctx {
        fn connections(&self) -> &Arc<ConnectionsService> {
            &self.svc
        }
        fn roles(&self) -> &Arc<dyn RoleChecker> {
            &self.roles
        }
        fn spawner(&self) -> &Arc<dyn Spawner> {
            &self.spawner
        }
        fn pool(&self) -> SqlitePool {
            self.pool.clone()
        }
    }
    struct Fixture {
        root: PathBuf,
        ctx: Ctx,
        user: User,
        connection: Connection,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    impl Fixture {
        async fn new() -> Self {
            use std::os::unix::fs::PermissionsExt;
            let root =
                std::env::temp_dir().join(format!("otto-sftp-fixture-{}", otto_core::new_id()));
            std::fs::create_dir(&root).unwrap();
            let program = root.join("sftp-fixture");
            // Only the batch-language adapter is faked. Production subprocess,
            // transfer task, registry, progress and authorization paths are real.
            std::fs::write(&program, r#"#!/usr/bin/env python3
import sys, shlex, pathlib, time, os
for line in sys.stdin:
    args = shlex.split(line)
    if not args: continue
    cmd = args[0]
    if cmd in ('get', 'put'):
        src, dst = map(pathlib.Path, args[-2:])
        with src.open('rb') as source, dst.open('xb') as dest:
            while True:
                block = source.read(4096)
                if not block: break
                dest.write(block); dest.flush(); time.sleep(0.12)
    elif cmd == 'ls':
        for entry in pathlib.Path(args[-1]).iterdir():
            print('-rw-r--r-- 1 fixture fixture %d Sep 13 12:00 %s' % (entry.stat().st_size, entry.name), flush=True)
    elif cmd == 'rename':
        src, dst = map(pathlib.Path, args[-2:])
        if dst.exists(): raise RuntimeError('destination exists')
        src.rename(dst)
        if dst.name == "slowpublish": time.sleep(1)
    elif cmd == 'rm': pathlib.Path(args[-1]).unlink(missing_ok=True)
    elif cmd == 'pwd': print('Remote working directory: /fixture')
"#).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(
                    sqlx::sqlite::SqliteConnectOptions::new()
                        .filename(root.join("fixture.db"))
                        .create_if_missing(true)
                        .foreign_keys(true),
                )
                .await
                .unwrap();
            sqlx::migrate!("../otto-state/migrations")
                .run(&pool)
                .await
                .unwrap();
            let id = otto_core::new_id();
            sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,disabled,created_at) VALUES(?,?, 'hash','Fixture',1,0,strftime('%Y-%m-%dT%H:%M:%fZ','now'))")
                .bind(&id).bind(&id).execute(&pool).await.unwrap();
            let user = otto_state::UsersRepo::new(pool.clone())
                .get(&id)
                .await
                .unwrap();
            let mut svc = ConnectionsService::new(
                ConnectionsRepo::new(pool.clone()),
                ConnectionSectionsRepo::new(pool.clone()),
                Arc::new(Secrets),
            );
            svc.sftp_pool = Arc::new(crate::sftp_pool::SftpPool::with_program(program));
            let connection = svc
                .create(
                    None,
                    &id,
                    otto_core::api::UpsertConnectionReq {
                        name: "Fixture".into(),
                        kind: otto_core::domain::ConnectionKind::Ssh,
                        params: serde_json::json!({"host":"isolated-fixture"}),
                        secret: None,
                        first_command: None,
                        section_id: None,
                        environment: None,
                        read_only: None,
                    },
                )
                .await
                .unwrap();
            Self {
                root,
                ctx: Ctx {
                    svc: Arc::new(svc),
                    pool,
                    roles: Arc::new(Roles),
                    spawner: Arc::new(Spawn),
                },
                user,
                connection,
            }
        }
        async fn start(
            &self,
            direction: &str,
            source: &str,
            destination: &str,
            timeout: u64,
        ) -> SftpTransfer {
            let (status, Json(job)) = start(
                State(self.ctx.clone()),
                Extension(AuthUser(self.user.clone())),
                Path(self.connection.id.clone()),
                Json(SftpTransferReq {
                    direction: direction.into(),
                    local_path: self
                        .root
                        .join(if direction == "upload" {
                            source
                        } else {
                            destination
                        })
                        .to_string_lossy()
                        .into_owned(),
                    remote_path: self
                        .root
                        .join(if direction == "upload" {
                            destination
                        } else {
                            source
                        })
                        .to_string_lossy()
                        .into_owned(),
                    timeout_secs: Some(timeout),
                }),
            )
            .await
            .map_err(|e| e.0)
            .unwrap();
            assert_eq!(status, StatusCode::ACCEPTED);
            job
        }
        async fn wait(&self, id: &str, predicate: impl Fn(&SftpTransfer) -> bool) -> SftpTransfer {
            tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    let Json(jobs) = list(
                        State(self.ctx.clone()),
                        Extension(AuthUser(self.user.clone())),
                        Path(self.connection.id.clone()),
                    )
                    .await
                    .map_err(|e| e.0)
                    .unwrap();
                    if let Some(job) = jobs.into_iter().find(|j| j.id == id && predicate(j)) {
                        return job;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await
            .unwrap()
        }
    }
    /// Manual local timing only: this fixture has no SSH handshake or network.
    /// It quantifies allocation/authorization overhead and repeated batch-client
    /// invocation, not an expected speedup on user hosts.
    #[tokio::test]
    #[ignore = "manual synthetic SFTP latency measurement; timing is not a CI assertion"]
    async fn synthetic_sftp_browse_latency() {
        let fixture = Fixture::new().await;
        const SAMPLES: usize = 20;
        for cold in [true, false] {
            let mut acquisition_us = Vec::new();
            let mut browse_us = Vec::new();
            let mut previous = None;
            // Warm-up the process adapter and filesystem caches, excluding it.
            let session = open_sftp(
                &fixture.ctx,
                &fixture.user,
                &fixture.connection.id,
                WorkspaceRole::Viewer,
                "sftp_read",
            )
            .await
            .map_err(|e| e.0)
            .unwrap();
            assert!(session
                .list(fixture.root.to_str().unwrap())
                .await
                .unwrap()
                .iter()
                .any(|entry| entry.name == "sftp-fixture"));
            drop(session);
            for _ in 0..SAMPLES {
                if cold {
                    fixture.ctx.svc.sftp_pool.clear_for_benchmark().await;
                }
                let start = Instant::now();
                let session = open_sftp(
                    &fixture.ctx,
                    &fixture.user,
                    &fixture.connection.id,
                    WorkspaceRole::Viewer,
                    "sftp_read",
                )
                .await
                .map_err(|e| e.0)
                .unwrap();
                acquisition_us.push(start.elapsed().as_micros());
                if !cold {
                    if let Some(previous) = &previous {
                        assert!(Arc::ptr_eq(previous, &session));
                    }
                    previous = Some(session.clone());
                }
                assert!(session
                    .list(fixture.root.to_str().unwrap())
                    .await
                    .unwrap()
                    .iter()
                    .any(|entry| entry.name == "sftp-fixture"));
                browse_us.push(start.elapsed().as_micros());
            }
            acquisition_us.sort_unstable();
            browse_us.sort_unstable();
            println!("SFTP_SYNTHETIC mode={} samples={} acquire_us_median={} acquire_us_p95={} browse_us_median={} browse_us_p95={}",
                if cold { "new_transport" } else { "cached_transport" }, SAMPLES,
                acquisition_us[SAMPLES / 2], acquisition_us[(SAMPLES * 95 / 100) - 1],
                browse_us[SAMPLES / 2], browse_us[(SAMPLES * 95 / 100) - 1]);
        }
    }

    #[tokio::test]
    async fn download_and_upload_report_actual_progress_then_publish_exact_content() {
        let fixture = Fixture::new().await;
        let content = vec![42; 128 * 1024];
        std::fs::write(fixture.root.join("source"), &content).unwrap();
        for (direction, destination) in [("download", "downloaded"), ("upload", "uploaded")] {
            let job = fixture.start(direction, "source", destination, 15).await;
            let progress = fixture
                .wait(&job.id, |j| j.bytes > 0 && j.status == "running")
                .await;
            assert!(progress.bytes < content.len() as u64);
            assert!(
                !fixture.root.join(destination).exists(),
                "partial file must not publish early"
            );
            let done = fixture
                .wait(&job.id, |j| {
                    !matches!(j.status.as_str(), "running" | "finalizing")
                })
                .await;
            assert_eq!(done.status, "completed", "{:?}", done.error);
            assert_eq!(done.bytes, content.len() as u64);
            assert_eq!(
                std::fs::read(fixture.root.join(destination)).unwrap(),
                content
            );
        }
    }
    #[tokio::test]
    async fn cancellation_during_publication_does_not_misreport_a_completed_remote_file() {
        let fixture = Fixture::new().await;
        std::fs::write(fixture.root.join("small"), b"published exactly once").unwrap();
        let job = fixture.start("upload", "small", "slowpublish", 15).await;
        fixture.wait(&job.id, |j| j.status == "finalizing").await;
        cancel(
            State(fixture.ctx.clone()),
            Extension(AuthUser(fixture.user.clone())),
            Path((fixture.connection.id.clone(), job.id.clone())),
        )
        .await
        .map_err(|e| e.0)
        .unwrap();
        let completed = fixture.wait(&job.id, |j| j.status == "completed").await;
        assert!(completed.error.is_none());
        assert_eq!(
            std::fs::read(fixture.root.join("slowpublish")).unwrap(),
            b"published exactly once"
        );
    }

    #[tokio::test]
    async fn cancellation_and_timeout_kill_transfer_and_clean_only_owned_partial_files() {
        let fixture = Fixture::new().await;
        std::fs::write(fixture.root.join("large"), vec![1; 1024 * 1024]).unwrap();
        let cancelled = fixture.start("download", "large", "cancelled", 15).await;
        fixture.wait(&cancelled.id, |j| j.bytes > 0).await;
        cancel(
            State(fixture.ctx.clone()),
            Extension(AuthUser(fixture.user.clone())),
            Path((fixture.connection.id.clone(), cancelled.id.clone())),
        )
        .await
        .map_err(|e| e.0)
        .unwrap();
        assert_eq!(
            fixture
                .wait(&cancelled.id, |j| !matches!(
                    j.status.as_str(),
                    "running" | "finalizing"
                ))
                .await
                .status,
            "cancelled"
        );
        let timed = fixture.start("upload", "large", "timed", 1).await;
        assert_eq!(
            fixture
                .wait(&timed.id, |j| !matches!(
                    j.status.as_str(),
                    "running" | "finalizing"
                ))
                .await
                .status,
            "timed_out"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!fixture.root.join("cancelled").exists());
        assert!(!fixture.root.join("timed").exists());
        assert!(fixture.root.join("large").exists());
        assert!(!std::fs::read_dir(&fixture.root).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".part")));
    }
    #[tokio::test]
    async fn revocation_stops_an_existing_authorized_transfer_and_cached_session_cannot_bypass_it()
    {
        let fixture = Fixture::new().await;
        std::fs::write(fixture.root.join("large"), vec![1; 1024 * 1024]).unwrap();
        let job = fixture.start("download", "large", "revoked", 15).await;
        fixture.wait(&job.id, |j| j.bytes > 0).await;
        sqlx::query("UPDATE users SET disabled = 1 WHERE id = ?")
            .bind(&fixture.user.id)
            .execute(&fixture.ctx.pool)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let state = fixture
                    .ctx
                    .svc
                    .transfers
                    .list(&fixture.user.id, &fixture.connection.id)
                    .into_iter()
                    .find(|j| j.id == job.id)
                    .unwrap();
                if state.status != "running" {
                    assert_eq!(state.status, "failed");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        assert!(open_sftp(
            &fixture.ctx,
            &fixture.user,
            &fixture.connection.id,
            WorkspaceRole::Editor,
            "sftp_read"
        )
        .await
        .is_err());
        assert!(!fixture.root.join("revoked").exists());
        sqlx::query("UPDATE resource_access_policies SET mode = 'legacy' WHERE resource_id = ?")
            .bind(&fixture.connection.id)
            .execute(&fixture.ctx.pool)
            .await
            .unwrap();
        assert!(
            authorize_transfer(
                &fixture.ctx,
                &fixture.user.id,
                &fixture.connection.id,
                &fixture.connection.params,
                "sftp_read"
            )
            .await
            .is_err(),
            "legacy mode must still reject a disabled initiating actor"
        );
    }
}
