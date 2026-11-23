//! Unit & mock-daemon integration tests for `otto-containers`.
//!
//! Spins up an in-process HTTP/1.1 mock Docker Engine API v1.47 server over a
//! temporary Unix domain socket and exercises every Axum route via `tower::ServiceExt`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Extension;
use bytes::Bytes;
use chrono::Utc;
use http_body_util::{BodyExt, Full};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use otto_connections::Spawner;
use otto_core::auth::{
    with_request_credential, AuthUser, BoxFuture, RequestCredential, RoleChecker,
};
use otto_core::domain::{Connection, Session, SessionKind, SessionStatus, User, WorkspaceRole};
use otto_core::{Error, Id, Result};
use otto_pty::CommandSpec;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::net::UnixListener;
use tower::ServiceExt;

use crate::client::{
    classify_kiri_vm_state, decode_docker_log_stream, is_secret_env, parse_container_stats,
    parse_env_entries,
};
use crate::contexts::{
    classify_engine, load_saved_selection, normalize_socket_path, save_selection,
    SavedContextSelection,
};
use crate::routes::{api_router, exec_spec, DEFAULT_SHELL};
use crate::service::{ContainersConfig, ContainersService};
use crate::types::{
    ContainerActionResp, ContainerDetail, ContainerExecReq, ContainerLogsResp, ContainerStatsResp,
    ContainersContextsResp, ContainersListResp, ContainersStatusResp, EngineKind, ImageInspectResp,
    ImagesListResp, NetworksListResp, PruneResp, PullImageResp, RunContainerResp,
    RuntimeStatusResp, StackActionResp, SystemPruneResp, VolumesListResp,
};
use crate::ContainersCtx;

#[derive(Debug, Clone)]
struct RecordedRequest {
    method: String,
    uri: String,
    body: Option<Value>,
    background_header: Option<String>,
    kiri_background_header: Option<String>,
}

#[derive(Default)]
struct MockDockerState {
    recorded: Vec<RecordedRequest>,
    create_404_once: bool,
    fail_stop_for_id: Option<String>,
    fail_containers_list: bool,
    large_pull_mode: Option<bool>, // Some(true) = >8 MiB ok, Some(false) = >8 MiB trailing error
    container_running: bool,
    tty_logs: bool,
    kiri_mode: bool,
    kiri_stopped: bool,
}

struct MockDockerServer {
    socket_path: PathBuf,
    state: Arc<Mutex<MockDockerState>>,
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for MockDockerServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

impl MockDockerServer {
    async fn start(dir: &Path) -> Self {
        let socket_path = dir.join("mock-docker.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind mock unix socket");
        let state = Arc::new(Mutex::new(MockDockerState {
            container_running: true,
            ..Default::default()
        }));
        let state_bg = state.clone();

        let handle = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let st = state_bg.clone();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let svc = service_fn(move |req| {
                        let st = st.clone();
                        async move { handle_mock_docker(req, st).await }
                    });
                    let _ = http1::Builder::new().serve_connection(io, svc).await;
                });
            }
        });

        Self {
            socket_path,
            state,
            handle,
        }
    }

    fn recorded(&self) -> Vec<RecordedRequest> {
        self.state.lock().unwrap().recorded.clone()
    }
}

async fn handle_mock_docker(
    req: hyper::Request<hyper::body::Incoming>,
    state: Arc<Mutex<MockDockerState>>,
) -> std::result::Result<hyper::Response<Full<Bytes>>, std::convert::Infallible> {
    let method = req.method().as_str().to_string();
    let uri = req.uri().to_string();
    let background_header = req
        .headers()
        .get("x-otto-background")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let kiri_background_header = req
        .headers()
        .get("x-kiri-background")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body_bytes = req
        .into_body()
        .collect()
        .await
        .map(|b| b.to_bytes())
        .unwrap_or_default();
    let parsed_body = if body_bytes.is_empty() {
        None
    } else {
        serde_json::from_slice::<Value>(&body_bytes).ok()
    };

    let (
        create_404,
        fail_stop_id,
        fail_containers_list,
        large_pull_mode,
        running,
        tty_logs,
        kiri_mode,
        kiri_stopped,
    ) = {
        let mut guard = state.lock().unwrap();
        guard.recorded.push(RecordedRequest {
            method: method.clone(),
            uri: uri.clone(),
            body: parsed_body,
            background_header,
            kiri_background_header,
        });
        if method == "POST" && uri == "/kiri/stop" {
            guard.kiri_stopped = true;
        } else if method == "POST" && uri == "/kiri/wake" {
            guard.kiri_stopped = false;
        }
        let c404 = if uri.starts_with("/v1.47/containers/create") && guard.create_404_once {
            guard.create_404_once = false;
            true
        } else {
            false
        };
        (
            c404,
            guard.fail_stop_for_id.clone(),
            guard.fail_containers_list,
            guard.large_pull_mode,
            guard.container_running,
            guard.tty_logs,
            guard.kiri_mode,
            guard.kiri_stopped,
        )
    };

    let (status, resp_body) = match (method.as_str(), uri.as_str()) {
        ("POST", "/kiri/wake") | ("POST", "/kiri/stop") => (StatusCode::NO_CONTENT, Bytes::new()),
        ("GET", "/v1.47/_ping") => (StatusCode::OK, Bytes::from_static(b"OK")),
        ("GET", "/v1.47/info") if kiri_mode => (
            StatusCode::OK,
            Bytes::from(
                json!({
                    "OperatingSystem": "Kiri",
                    "ServerVersion": "27.4.0",
                    "KiriVersion": "0.2.1",
                    "Architecture": "aarch64",
                    "ContainersRunning": 1,
                    "ContainersPaused": 0,
                    "ContainersStopped": 0,
                    "Images": 2,
                    "MemTotal": 4_294_967_296u64,
                    "NCPU": 2,
                    "KiriVmAwake": !kiri_stopped,
                    "KiriStopped": kiri_stopped,
                    "KiriMemoryUsed": 268_435_456u64,
                    "KiriMemoryTarget": 536_870_912u64,
                    "KiriMemoryCeiling": 4_294_967_296u64,
                    "KiriCpuPercent": 4.2
                })
                .to_string(),
            ),
        ),
        ("GET", "/v1.47/info") => (
            StatusCode::OK,
            Bytes::from(
                json!({
                    "OperatingSystem": "Otto",
                    "ServerVersion": "27.4.0",
                    "Architecture": "aarch64",
                    "ContainersRunning": 2,
                    "ContainersPaused": 0,
                    "ContainersStopped": 1,
                    "Images": 3,
                    "MemTotal": 8_589_934_592u64,
                    "NCPU": 4,
                    "OttoVmState": "running",
                    "OttoVmCpus": 4,
                    "OttoVmMemoryUsedBytes": 536_870_912u64,
                    "OttoVmMemoryTargetBytes": 805_306_368u64,
                    "OttoVmMemoryCeilingBytes": 8_589_934_592u64,
                    "OttoVmCpuPercent": 12.5
                })
                .to_string(),
            ),
        ),
        ("GET", "/v1.47/version") => (
            StatusCode::OK,
            Bytes::from(json!({ "Version": "27.4.0", "ApiVersion": "1.47", "Arch": "arm64" }).to_string()),
        ),
        ("GET", u) if u.starts_with("/v1.47/containers/json") && fail_containers_list => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Bytes::from(json!({ "message": "injected container list failure" }).to_string()),
        ),
        ("GET", u) if u.starts_with("/v1.47/containers/json") => (
            StatusCode::OK,
            Bytes::from(
                json!([
                    {
                        "Id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                        "Names": ["/db-stopped"],
                        "Image": "postgres:16",
                        "ImageID": "sha256:img-pg",
                        "Command": "postgres",
                        "Created": 1_700_000_200,
                        "State": "exited",
                        "Status": "Exited (0) 5 minutes ago",
                        "Ports": [],
                        "Labels": {
                            "com.docker.compose.project": "myapp",
                            "com.docker.compose.service": "db",
                            "com.docker.compose.project.working_dir": "/Users/dev/myapp"
                        },
                        "NetworkSettings": { "Networks": { "myapp_default": {} } },
                        "Mounts": [{}]
                    },
                    {
                        "Id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                        "Names": ["/web-running"],
                        "Image": "nginx:1.27",
                        "ImageID": "sha256:img-nginx",
                        "Command": "nginx -g 'daemon off;'",
                        "Created": 1_700_000_100,
                        "State": "running",
                        "Status": "Up 2 hours (healthy)",
                        "Ports": [
                            { "IP": "0.0.0.0", "PrivatePort": 80, "PublicPort": 8080, "Type": "tcp" },
                            { "IP": "::", "PrivatePort": 80, "PublicPort": 8080, "Type": "tcp" }
                        ],
                        "Labels": {
                            "com.docker.compose.project": "myapp",
                            "com.docker.compose.service": "web",
                            "com.docker.compose.project.working_dir": "/Users/dev/myapp"
                        },
                        "NetworkSettings": { "Networks": { "myapp_default": {} } },
                        "Mounts": [{}, {}]
                    }
                ])
                .to_string(),
            ),
        ),
        ("GET", "/v1.47/containers/missing/json") => (
            StatusCode::NOT_FOUND,
            Bytes::from(json!({ "message": "No such container: missing" }).to_string()),
        ),
        ("GET", u) if u.starts_with("/v1.47/containers/") && u.ends_with("/json") => (
            StatusCode::OK,
            Bytes::from(
                json!({
                    "Id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "Name": "/web-running",
                    "Image": "sha256:img-nginx",
                    "Created": "2026-11-19T10:00:00Z",
                    "Config": {
                        "Image": "nginx:1.27",
                        "Hostname": "web-host",
                        "WorkingDir": "/app",
                        "Entrypoint": ["/docker-entrypoint.sh"],
                        "Cmd": ["nginx", "-g", "daemon off;"],
                        "Env": [
                            "APP_ENV=development",
                            "DB_PASSWORD=super-secret-pw",
                            "STRIPE_API_KEY=sk-live-12345",
                            "PLAIN_TOKEN_VAR=tok_abc",
                            "PORT=8080"
                        ],
                        "Labels": {
                            "com.docker.compose.project": "myapp",
                            "com.docker.compose.service": "web"
                        }
                    },
                    "State": {
                        "Status": if running { "running" } else { "exited" },
                        "Running": running,
                        "Paused": false,
                        "Restarting": false,
                        "OOMKilled": false,
                        "Pid": if running { 4242 } else { 0 },
                        "ExitCode": 0,
                        "StartedAt": "2026-11-19T10:00:01Z",
                        "FinishedAt": "0001-01-01T00:00:00Z",
                        "Health": { "Status": "healthy" }
                    },
                    "HostConfig": {
                        "RestartPolicy": { "Name": "unless-stopped" }
                    },
                    "Mounts": [
                        {
                            "Type": "bind",
                            "Source": "/Users/dev/myapp",
                            "Destination": "/app",
                            "Mode": "rw",
                            "RW": true
                        }
                    ],
                    "NetworkSettings": {
                        "Ports": {
                            "80/tcp": [{ "HostIp": "0.0.0.0", "HostPort": "8080" }]
                        },
                        "Networks": {
                            "myapp_default": {
                                "NetworkID": "net123",
                                "IPAddress": "172.18.0.2",
                                "Gateway": "172.18.0.1",
                                "MacAddress": "02:42:ac:12:00:02"
                            }
                        }
                    }
                })
                .to_string(),
            ),
        ),
        ("POST", u) if u.starts_with("/v1.47/containers/create") => {
            if create_404 {
                (
                    StatusCode::NOT_FOUND,
                    Bytes::from(json!({ "message": "No such image: alpine:3.20" }).to_string()),
                )
            } else {
                (
                    StatusCode::CREATED,
                    Bytes::from(
                        json!({ "Id": "cccccccccccccccccccccccccccccccc", "Warnings": ["test warning"] })
                            .to_string(),
                    ),
                )
            }
        }
        ("POST", u) if u.contains("/containers/") && u.ends_with("/start") => {
            (StatusCode::NO_CONTENT, Bytes::new())
        }
        ("POST", u) if u.contains("/containers/") && u.contains("/stop") => {
            if let Some(fail_id) = fail_stop_id {
                if u.contains(&fail_id) {
                    return Ok(hyper::Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Full::new(Bytes::from(
                            json!({ "message": "stop failed intentionally" }).to_string(),
                        )))
                        .unwrap());
                }
            }
            (StatusCode::NOT_MODIFIED, Bytes::new())
        }
        ("POST", u)
            if u.contains("/containers/")
                && (u.contains("/restart")
                    || u.ends_with("/pause")
                    || u.ends_with("/unpause")
                    || u.contains("/kill")) =>
        {
            (StatusCode::NO_CONTENT, Bytes::new())
        }
        ("DELETE", u) if u.starts_with("/v1.47/containers/") => {
            (StatusCode::NO_CONTENT, Bytes::new())
        }
        ("GET", u) if u.contains("/containers/") && u.contains("/logs") => {
            if tty_logs {
                (
                    StatusCode::OK,
                    Bytes::from_static(b"hello tty line 1\nhello tty line 2\n"),
                )
            } else {
                let mut payload = Vec::new();
                let line1 = b"2026-11-19T10:00:01.000Z server started\n";
                payload.extend_from_slice(&[1, 0, 0, 0]);
                payload.extend_from_slice(&(line1.len() as u32).to_be_bytes());
                payload.extend_from_slice(line1);
                let line2 = b"2026-11-19T10:00:02.000Z warning: slow query\n";
                payload.extend_from_slice(&[2, 0, 0, 0]);
                payload.extend_from_slice(&(line2.len() as u32).to_be_bytes());
                payload.extend_from_slice(line2);
                (StatusCode::OK, Bytes::from(payload))
            }
        }
        ("GET", u) if u.contains("/containers/") && u.contains("/stats") => (
            StatusCode::OK,
            Bytes::from(
                json!({
                    "read": "2026-11-19T10:05:00Z",
                    "cpu_stats": {
                        "cpu_usage": { "total_usage": 200_000_000u64 },
                        "system_cpu_usage": 2_000_000_000u64,
                        "online_cpus": 2
                    },
                    "precpu_stats": {
                        "cpu_usage": { "total_usage": 100_000_000u64 },
                        "system_cpu_usage": 1_000_000_000u64
                    },
                    "memory_stats": {
                        "usage": 268_435_456u64,
                        "limit": 1_073_741_824u64,
                        "stats": { "inactive_file": 67_108_864u64 }
                    },
                    "networks": {
                        "eth0": { "rx_bytes": 1200, "tx_bytes": 3400 }
                    },
                    "blkio_stats": {
                        "io_service_bytes_recursive": [
                            { "op": "Read", "value": 4096 },
                            { "op": "Write", "value": 8192 }
                        ]
                    },
                    "pids_stats": { "current": 14 }
                })
                .to_string(),
            ),
        ),
        ("GET", "/v1.47/images/json") => (
            StatusCode::OK,
            Bytes::from(
                json!([
                    {
                        "Id": "sha256:img-nginx",
                        "RepoTags": ["nginx:1.27"],
                        "RepoDigests": ["nginx@sha256:abc"],
                        "Created": 1_700_000_500,
                        "Size": 67_108_864,
                        "Containers": 1
                    },
                    {
                        "Id": "sha256:img-dangling",
                        "RepoTags": ["<none>:<none>"],
                        "RepoDigests": [],
                        "Created": 1_700_000_100,
                        "Size": 10_000_000,
                        "Containers": 0
                    }
                ])
                .to_string(),
            ),
        ),
        ("GET", u) if u.starts_with("/v1.47/images/") && u.ends_with("/json") => {
            if u.contains("missing") {
                (
                    StatusCode::NOT_FOUND,
                    Bytes::from(json!({ "message": "No such image: missing:1" }).to_string()),
                )
            } else {
                (
                    StatusCode::OK,
                    Bytes::from(
                        json!({
                            "Id": "sha256:img-pg",
                            "Config": {
                                "Env": ["POSTGRES_PASSWORD=never-sent-to-the-ui"],
                                "ExposedPorts": { "8080/tcp": {}, "5432/tcp": {}, "53/udp": {} }
                            }
                        })
                        .to_string(),
                    ),
                )
            }
        }
        ("POST", u) if u.starts_with("/v1.47/images/create") => {
            if let Some(succeed) = large_pull_mode {
                // Emit >8.5 MiB of NDJSON progress frames before a trailing status/error.
                let pad_line = format!("{{\"status\":\"Downloading\",\"progress\":\"{}\"}}\n", "=".repeat(512));
                let mut payload = String::with_capacity(9 * 1024 * 1024);
                while payload.len() < 8_600_000 {
                    payload.push_str(&pad_line);
                }
                if succeed {
                    payload.push_str("{\"status\":\"Digest: sha256:largepullok\"}\n");
                    payload.push_str("{\"status\":\"Status: Downloaded newer image\"}\n");
                } else {
                    payload.push_str("{\"error\":\"no space left on device\"}\n");
                }
                (StatusCode::OK, Bytes::from(payload))
            } else {
                (
                    StatusCode::OK,
                    Bytes::from(
                        "{\"status\":\"Pulling from library/alpine\"}\n\
                         {\"status\":\"Digest: sha256:deadbeef\"}\n\
                         {\"status\":\"Status: Downloaded newer image for alpine:3.20\"}\n",
                    ),
                )
            }
        }
        ("DELETE", u) if u.starts_with("/v1.47/images/") => {
            (StatusCode::OK, Bytes::from(json!([{"Deleted": "sha256:img-dangling"}]).to_string()))
        }
        ("POST", u) if u.starts_with("/v1.47/images/prune") => (
            StatusCode::OK,
            Bytes::from(
                json!({
                    "ImagesDeleted": [{"Deleted": "sha256:img-dangling"}],
                    "SpaceReclaimed": 10_000_000u64
                })
                .to_string(),
            ),
        ),
        ("GET", "/v1.47/volumes") => (
            StatusCode::OK,
            Bytes::from(
                json!({
                    "Volumes": [
                        {
                            "Name": "pg_data",
                            "Driver": "local",
                            "Mountpoint": "/var/lib/docker/volumes/pg_data/_data",
                            "CreatedAt": "2026-11-18T00:00:00Z",
                            "Scope": "local",
                            "Labels": {},
                            "UsageData": { "Size": 52_428_800, "RefCount": 1 }
                        }
                    ]
                })
                .to_string(),
            ),
        ),
        ("DELETE", u) if u.starts_with("/v1.47/volumes/") => {
            (StatusCode::NO_CONTENT, Bytes::new())
        }
        ("POST", "/v1.47/volumes/prune") => (
            StatusCode::OK,
            Bytes::from(
                json!({ "VolumesDeleted": ["old_vol"], "SpaceReclaimed": 4096u64 }).to_string(),
            ),
        ),
        ("GET", "/v1.47/networks") => (
            StatusCode::OK,
            Bytes::from(
                json!([
                    {
                        "Id": "net_bridge_000000",
                        "Name": "bridge",
                        "Driver": "bridge",
                        "Scope": "local",
                        "Internal": false,
                        "Attachable": false,
                        "IPAM": { "Config": [{ "Subnet": "172.17.0.0/16", "Gateway": "172.17.0.1" }] },
                        "Containers": {},
                        "Labels": {}
                    },
                    {
                        "Id": "net_myapp_111111",
                        "Name": "myapp_default",
                        "Driver": "bridge",
                        "Scope": "local",
                        "Internal": false,
                        "Attachable": true,
                        "IPAM": { "Config": [{ "Subnet": "172.18.0.0/16", "Gateway": "172.18.0.1" }] },
                        "Containers": {
                            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa": {
                                "Name": "web-running",
                                "IPv4Address": "172.18.0.2/16"
                            }
                        },
                        "Labels": { "com.docker.compose.project": "myapp" }
                    }
                ])
                .to_string(),
            ),
        ),
        ("DELETE", u) if u.starts_with("/v1.47/networks/") => {
            (StatusCode::NO_CONTENT, Bytes::new())
        }
        ("POST", "/v1.47/networks/prune") => (
            StatusCode::OK,
            Bytes::from(json!({ "NetworksDeleted": ["stale_net"] }).to_string()),
        ),
        ("POST", "/v1.47/containers/prune") => (
            StatusCode::OK,
            Bytes::from(
                json!({ "ContainersDeleted": ["stopped_1"], "SpaceReclaimed": 1024u64 }).to_string(),
            ),
        ),
        _ => (
            StatusCode::NOT_FOUND,
            Bytes::from(json!({ "message": format!("unhandled mock route: {method} {uri}") }).to_string()),
        ),
    };

    Ok(hyper::Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Full::new(resp_body))
        .unwrap())
}

#[derive(Default)]
struct RecordingSpawner {
    last_spec: Mutex<Option<(String, String, CommandSpec)>>,
}

impl Spawner for RecordingSpawner {
    fn spawn_connection<'a>(
        &'a self,
        _ws_id: &'a Id,
        _user_id: &'a Id,
        _conn: &'a Connection,
        _spec: CommandSpec,
        _first_command: Option<String>,
        _title: Option<String>,
    ) -> BoxFuture<'a, Result<Session>> {
        Box::pin(async { Err(Error::Internal("unused in test".into())) })
    }

    fn spawn_command<'a>(
        &'a self,
        ws_id: &'a Id,
        user_id: &'a Id,
        provider: &'a str,
        spec: CommandSpec,
        title: String,
        meta: Option<Value>,
    ) -> BoxFuture<'a, Result<Session>> {
        let ws = ws_id.clone();
        let uid = user_id.clone();
        let prov = provider.to_string();
        Box::pin(async move {
            *self.last_spec.lock().unwrap() = Some((prov.clone(), title.clone(), spec));
            let now = Utc::now();
            Ok(Session {
                id: "sess_test_01".into(),
                workspace_id: ws,
                kind: SessionKind::Connection,
                provider: prov,
                title,
                status: SessionStatus::Running,
                cwd: "/".into(),
                provider_session_id: None,
                connection_id: None,
                created_by: uid,
                created_at: now,
                last_active_at: now,
                archived: false,
                meta: meta.unwrap_or(Value::Null),
            })
        })
    }
}

struct MockRoleChecker {
    allow: bool,
}

impl RoleChecker for MockRoleChecker {
    fn check<'a>(
        &'a self,
        _user: &'a User,
        _workspace_id: &'a Id,
        _min: WorkspaceRole,
    ) -> BoxFuture<'a, Result<()>> {
        let ok = self.allow;
        Box::pin(async move {
            if ok {
                Ok(())
            } else {
                Err(Error::Forbidden("workspace editor role required".into()))
            }
        })
    }
}

#[derive(Clone)]
struct TestCtxWrapper {
    containers: Arc<ContainersService>,
    spawner_dyn: Arc<dyn Spawner>,
    spawner_rec: Arc<RecordingSpawner>,
    roles: Arc<dyn RoleChecker>,
}

impl TestCtxWrapper {
    fn new(data_dir: PathBuf, home_dir: Option<PathBuf>, socket_override: Option<PathBuf>) -> Self {
        let mut cfg = ContainersConfig::new(data_dir);
        cfg.home_dir_override = home_dir;
        cfg.docker_socket_override = socket_override;
        let rec = Arc::new(RecordingSpawner::default());
        Self {
            containers: Arc::new(ContainersService::with_config(cfg)),
            spawner_dyn: rec.clone(),
            spawner_rec: rec,
            roles: Arc::new(MockRoleChecker { allow: true }),
        }
    }

    fn with_kiri(self, bin: Option<PathBuf>, sock: Option<PathBuf>) -> Self {
        let mut cfg = self.containers.config().clone();
        cfg.kiri_bin_override = bin;
        cfg.kiri_socket_override = sock;
        Self {
            containers: Arc::new(ContainersService::with_config(cfg)),
            ..self
        }
    }

    fn with_role_allowed(mut self, allow: bool) -> Self {
        self.roles = Arc::new(MockRoleChecker { allow });
        self
    }
}

impl ContainersCtx for TestCtxWrapper {
    fn containers(&self) -> &Arc<ContainersService> {
        &self.containers
    }

    fn spawner(&self) -> &Arc<dyn Spawner> {
        &self.spawner_dyn
    }

    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
}

fn test_user() -> AuthUser {
    AuthUser(User {
        id: "user_1".into(),
        username: "dev".into(),
        display_name: "Dev".into(),
        is_root: true,
        disabled: false,
        created_at: Utc::now(),
    })
}

async fn send_json(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let req = builder.body(body).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

#[tokio::test]
async fn status_online_and_offline_and_background_header() {
    let tmp = TempDir::new().unwrap();
    let mock = MockDockerServer::start(tmp.path()).await;
    let ctx = TestCtxWrapper::new(
        tmp.path().join("data"),
        Some(tmp.path().join("home")),
        Some(mock.socket_path.clone()),
    );
    let app = api_router::<TestCtxWrapper>().with_state(ctx);

    let (status, body) = send_json(app, Method::GET, "/containers/status", None).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: ContainersStatusResp = serde_json::from_value(body).unwrap();
    assert!(parsed.available);
    assert_eq!(parsed.engine_kind, EngineKind::Otto);
    assert_eq!(parsed.engine_version.as_deref(), Some("27.4.0"));
    assert_eq!(parsed.api_version.as_deref(), Some("1.47"));
    assert_eq!(parsed.containers_running, 2);
    assert_eq!(parsed.vm.as_ref().unwrap().state, "running");

    // Verify every status read sent both X-Otto-Background and X-Kiri-Background.
    let recorded = mock.recorded();
    assert!(recorded.len() >= 3);
    assert!(recorded.iter().all(|r| {
        r.background_header.as_deref() == Some("1")
            && r.kiri_background_header.as_deref() == Some("1")
    }));

    // Offline socket produces available: false with error, still 200.
    let offline_ctx = TestCtxWrapper::new(
        tmp.path().join("data2"),
        Some(tmp.path().join("home")),
        Some(tmp.path().join("does-not-exist.sock")),
    );
    let offline_app = api_router::<TestCtxWrapper>().with_state(offline_ctx);
    let (status, body) = send_json(offline_app, Method::GET, "/containers/status", None).await;
    assert_eq!(status, StatusCode::OK);
    let offline: ContainersStatusResp = serde_json::from_value(body).unwrap();
    assert!(!offline.available);
    assert!(offline.error.as_deref().unwrap().contains("unreachable"));
}

#[tokio::test]
async fn list_containers_groups_compose_stacks_and_deduplicates_ports() {
    let tmp = TempDir::new().unwrap();
    let mock = MockDockerServer::start(tmp.path()).await;
    let ctx = TestCtxWrapper::new(
        tmp.path().join("data"),
        Some(tmp.path().join("home")),
        Some(mock.socket_path.clone()),
    );
    let app = api_router::<TestCtxWrapper>().with_state(ctx);

    let (status, body) = send_json(app, Method::GET, "/containers", None).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: ContainersListResp = serde_json::from_value(body).unwrap();
    assert_eq!(parsed.containers.len(), 2);
    // Running container sorts before stopped even if created earlier.
    assert_eq!(parsed.containers[0].name, "web-running");
    assert_eq!(parsed.containers[0].health.as_deref(), Some("healthy"));
    assert_eq!(parsed.containers[0].ports.len(), 1);
    assert_eq!(
        parsed.containers[0].ports[0].url.as_deref(),
        Some("http://localhost:8080")
    );
    assert_eq!(parsed.stacks.len(), 1);
    assert_eq!(parsed.stacks[0].project, "myapp");
    assert_eq!(parsed.stacks[0].running, 1);
    assert_eq!(parsed.stacks[0].total, 2);
}

#[tokio::test]
async fn inspect_masks_secrets_and_requires_secret_reveal_endpoint() {
    let tmp = TempDir::new().unwrap();
    let mock = MockDockerServer::start(tmp.path()).await;
    let ctx = TestCtxWrapper::new(
        tmp.path().join("data"),
        Some(tmp.path().join("home")),
        Some(mock.socket_path.clone()),
    );
    let app = api_router::<TestCtxWrapper>().with_state(ctx);

    let (status, body) = send_json(
        app.clone(),
        Method::GET,
        "/containers/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let masked: ContainerDetail = serde_json::from_value(body).unwrap();
    let pw = masked.env.iter().find(|e| e.key == "DB_PASSWORD").unwrap();
    assert!(pw.secret && pw.masked);
    assert_eq!(pw.value, "••••••••");
    let plain = masked.env.iter().find(|e| e.key == "APP_ENV").unwrap();
    assert!(!plain.secret && !plain.masked);
    assert_eq!(plain.value, "development");

    // Passing ?reveal_secrets=true on GET /containers/{id} is rejected with 403.
    let (status_get_reveal, _) = send_json(
        app.clone(),
        Method::GET,
        "/containers/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa?reveal_secrets=true",
        None,
    )
    .await;
    assert_eq!(status_get_reveal, StatusCode::FORBIDDEN);

    // POST /containers/{id}/secrets/reveal rejects agent / outside-request credentials.
    let (status_agent_reveal, _) = send_json(
        app.clone(),
        Method::POST,
        "/containers/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/secrets/reveal",
        None,
    )
    .await;
    assert_eq!(status_agent_reveal, StatusCode::FORBIDDEN);

    // POST /containers/{id}/secrets/reveal succeeds for a human credential.
    let human = RequestCredential {
        agent: false,
        root_withheld: false,
    };
    let (status2, body2) = with_request_credential(
        human,
        send_json(
            app,
            Method::POST,
            "/containers/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/secrets/reveal",
            None,
        ),
    )
    .await;
    assert_eq!(status2, StatusCode::OK);
    let revealed: ContainerDetail = serde_json::from_value(body2).unwrap();
    let pw2 = revealed
        .env
        .iter()
        .find(|e| e.key == "DB_PASSWORD")
        .unwrap();
    assert!(pw2.secret && !pw2.masked);
    assert_eq!(pw2.value, "super-secret-pw");
}

#[tokio::test]
async fn run_pull_if_missing_and_lifecycle_mutations_omit_background_header() {
    let tmp = TempDir::new().unwrap();
    let mock = MockDockerServer::start(tmp.path()).await;
    mock.state.lock().unwrap().create_404_once = true;

    let ctx = TestCtxWrapper::new(
        tmp.path().join("data"),
        Some(tmp.path().join("home")),
        Some(mock.socket_path.clone()),
    );
    let app = api_router::<TestCtxWrapper>().with_state(ctx);

    let run_body = json!({
        "image": "alpine:3.20",
        "name": "test-alpine",
        "cmd": ["sleep", "60"],
        "env": ["FOO=bar"],
        "ports": [{ "host_ip": "127.0.0.1", "host_port": 9090, "container_port": 80, "protocol": "tcp" }],
        "volumes": ["/tmp/data:/data:ro"],
        "working_dir": "/app",
        "restart_policy": "unless-stopped",
        "pull_if_missing": true
    });

    // Agent credential / outside request cannot run containers (prevents bind-mount escapes).
    let (s_agent, _) = send_json(
        app.clone(),
        Method::POST,
        "/containers/run",
        Some(run_body.clone()),
    )
    .await;
    assert_eq!(s_agent, StatusCode::FORBIDDEN);

    let human = RequestCredential {
        agent: false,
        root_withheld: false,
    };
    let (status, body) = with_request_credential(
        human,
        send_json(app.clone(), Method::POST, "/containers/run", Some(run_body)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let run: RunContainerResp = serde_json::from_value(body).unwrap();
    assert_eq!(run.id, "cccccccccccccccccccccccccccccccc");
    assert_eq!(run.warnings, vec!["test warning"]);

    // Stop (mock returns 304 -> treated as idempotent ok) & kill & remove.
    let (s_stop, b_stop) = send_json(
        app.clone(),
        Method::POST,
        "/containers/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/stop?t=5",
        None,
    )
    .await;
    assert_eq!(s_stop, StatusCode::OK);
    let stop_resp: ContainerActionResp = serde_json::from_value(b_stop).unwrap();
    assert!(stop_resp.ok);

    let (s_bad_sig, _) = send_json(
        app.clone(),
        Method::POST,
        "/containers/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/kill?signal=SIG;rm",
        None,
    )
    .await;
    assert_eq!(s_bad_sig, StatusCode::BAD_REQUEST);

    let (s_del, _) = send_json(
        app,
        Method::DELETE,
        "/containers/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa?force=true&v=true",
        None,
    )
    .await;
    assert_eq!(s_del, StatusCode::OK);

    // Every mutation must omit X-Otto-Background AND serialize exact Docker API URIs + JSON payloads.
    let recorded = mock.recorded();
    assert!(recorded
        .iter()
        .filter(|r| r.method != "GET")
        .all(|r| r.background_header.is_none()));

    let creates: Vec<&RecordedRequest> = recorded
        .iter()
        .filter(|r| r.method == "POST" && r.uri == "/v1.47/containers/create?name=test-alpine")
        .collect();
    assert_eq!(
        creates.len(),
        2,
        "expected initial 404 create + retry after pull"
    );
    let create_payload = creates[0].body.as_ref().expect("create JSON body");
    assert_eq!(create_payload["Image"], "alpine:3.20");
    assert_eq!(create_payload["Cmd"], json!(["sleep", "60"]));
    assert_eq!(create_payload["Env"], json!(["FOO=bar"]));
    assert_eq!(create_payload["WorkingDir"], "/app");
    assert_eq!(create_payload["ExposedPorts"], json!({ "80/tcp": {} }));
    assert_eq!(
        create_payload["HostConfig"]["PortBindings"]["80/tcp"],
        json!([{ "HostIp": "127.0.0.1", "HostPort": "9090" }])
    );
    assert_eq!(
        create_payload["HostConfig"]["Binds"],
        json!(["/tmp/data:/data:ro"])
    );
    assert_eq!(
        create_payload["HostConfig"]["RestartPolicy"]["Name"],
        "unless-stopped"
    );

    assert!(recorded.iter().any(|r| {
        r.method == "POST" && r.uri == "/v1.47/images/create?fromImage=alpine&tag=3.20"
    }));
    assert!(recorded.iter().any(|r| {
        r.method == "POST" && r.uri == "/v1.47/containers/cccccccccccccccccccccccccccccccc/start"
    }));
    assert!(recorded.iter().any(|r| {
        r.method == "POST" && r.uri == "/v1.47/containers/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/stop?t=5"
    }));
    assert!(recorded.iter().any(|r| {
        r.method == "DELETE"
            && r.uri == "/v1.47/containers/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa?force=true&v=true"
    }));
}

#[tokio::test]
async fn stack_actions_report_partial_failures() {
    let tmp = TempDir::new().unwrap();
    let mock = MockDockerServer::start(tmp.path()).await;
    mock.state.lock().unwrap().fail_stop_for_id =
        Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string());

    let ctx = TestCtxWrapper::new(
        tmp.path().join("data"),
        Some(tmp.path().join("home")),
        Some(mock.socket_path.clone()),
    );
    let app = api_router::<TestCtxWrapper>().with_state(ctx);

    let (status, body) = send_json(
        app.clone(),
        Method::POST,
        "/containers/stacks/myapp/stop",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let resp: StackActionResp = serde_json::from_value(body).unwrap();
    assert_eq!(resp.succeeded.len(), 1);
    assert_eq!(resp.failed.len(), 1);
    assert_eq!(resp.failed[0].name, "db-stopped");

    let (s_missing, _) = send_json(app, Method::POST, "/containers/stacks/nope/start", None).await;
    assert_eq!(s_missing, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn logs_demux_and_tty_and_stats_computation() {
    let tmp = TempDir::new().unwrap();
    let mock = MockDockerServer::start(tmp.path()).await;
    let ctx = TestCtxWrapper::new(
        tmp.path().join("data"),
        Some(tmp.path().join("home")),
        Some(mock.socket_path.clone()),
    );
    let app = api_router::<TestCtxWrapper>().with_state(ctx);

    // Multiplexed Docker log stream with timestamps.
    let (status, body) = send_json(
        app.clone(),
        Method::GET,
        "/containers/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/logs?tail=50&timestamps=true",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let logs: ContainerLogsResp = serde_json::from_value(body).unwrap();
    assert_eq!(logs.lines.len(), 2);
    assert_eq!(logs.lines[0].stream, "stdout");
    assert_eq!(
        logs.lines[0].timestamp.as_deref(),
        Some("2026-11-19T10:00:01.000Z")
    );
    assert_eq!(logs.lines[0].text, "server started");
    assert_eq!(logs.lines[1].stream, "stderr");
    assert_eq!(logs.lines[1].text, "warning: slow query");

    // Plain TTY log output.
    mock.state.lock().unwrap().tty_logs = true;
    let (status_tty, body_tty) = send_json(
        app.clone(),
        Method::GET,
        "/containers/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/logs?tail=50",
        None,
    )
    .await;
    assert_eq!(status_tty, StatusCode::OK);
    let tty_logs: ContainerLogsResp = serde_json::from_value(body_tty).unwrap();
    assert_eq!(tty_logs.lines.len(), 2);
    assert_eq!(tty_logs.lines[0].stream, "tty");
    assert_eq!(tty_logs.lines[0].text, "hello tty line 1");

    // Stats calculation.
    let (s_stats, b_stats) = send_json(
        app,
        Method::GET,
        "/containers/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/stats",
        None,
    )
    .await;
    assert_eq!(s_stats, StatusCode::OK);
    let stats: ContainerStatsResp = serde_json::from_value(b_stats).unwrap();
    // (100M / 1000M) * 2 CPUs * 100 = 20.0%
    assert!((stats.cpu_percent - 20.0).abs() < 0.01);
    // (256MiB - 64MiB) = 192MiB out of 1024MiB = 18.75%
    assert_eq!(stats.memory_usage_bytes, 201_326_592);
    assert!((stats.memory_percent - 18.75).abs() < 0.01);
    assert_eq!(stats.net_rx_bytes, 1200);
    assert_eq!(stats.net_tx_bytes, 3400);
    assert_eq!(stats.block_read_bytes, 4096);
    assert_eq!(stats.block_write_bytes, 8192);
    assert_eq!(stats.pids, 14);
}

#[tokio::test]
async fn exec_spawns_pty_with_docker_host_and_rejects_stopped_container() {
    let tmp = TempDir::new().unwrap();
    let mock = MockDockerServer::start(tmp.path()).await;
    // Create a fake docker CLI in <home>/.otto/bin/docker so locate_docker_cli finds it.
    let home = tmp.path().join("home");
    let bin_dir = home.join(".otto/bin");
    tokio::fs::create_dir_all(&bin_dir).await.unwrap();
    let fake_docker = bin_dir.join("docker");
    tokio::fs::write(&fake_docker, b"#!/bin/sh\n")
        .await
        .unwrap();

    let ctx = TestCtxWrapper::new(
        tmp.path().join("data"),
        Some(home),
        Some(mock.socket_path.clone()),
    );
    let spawner_rec = ctx.spawner_rec.clone();
    let app = api_router::<TestCtxWrapper>()
        .layer(Extension(test_user()))
        .with_state(ctx);

    let human = RequestCredential {
        agent: false,
        root_withheld: false,
    };

    // Without a human credential (agent / outside request) exec fails closed with 403.
    let (s_agent, _) = send_json(
        app.clone(),
        Method::POST,
        "/containers/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/exec",
        Some(json!({ "workspace_id": "ws_1" })),
    )
    .await;
    assert_eq!(s_agent, StatusCode::FORBIDDEN);

    let (status, body) = with_request_credential(
        human,
        send_json(
            app.clone(),
            Method::POST,
            "/containers/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/exec",
            Some(json!({ "workspace_id": "ws_1", "user": "root", "working_dir": "/app" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.get("provider").and_then(|v| v.as_str()),
        Some("containers")
    );

    let (prov, title, spec) = spawner_rec.last_spec.lock().unwrap().clone().unwrap();
    assert_eq!(prov, "containers");
    assert_eq!(title, "web-running · shell");
    assert_eq!(spec.program, fake_docker.to_string_lossy());
    assert_eq!(spec.args[0], "-H");
    assert!(spec.args[1].starts_with("unix://"));
    assert!(spec.args.contains(&"exec".to_string()));
    assert!(spec.args.contains(&"-it".to_string()));
    assert!(spec.args.contains(&"--user".to_string()));
    assert!(spec.args.contains(&"--workdir".to_string()));
    assert!(spec.args.contains(&DEFAULT_SHELL.to_string()));

    // Stopped container returns 409 Conflict before spawning PTY.
    mock.state.lock().unwrap().container_running = false;
    let (s_stopped, _) = with_request_credential(
        human,
        send_json(
            app,
            Method::POST,
            "/containers/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/exec",
            Some(json!({ "workspace_id": "ws_1" })),
        ),
    )
    .await;
    assert_eq!(s_stopped, StatusCode::CONFLICT);
}

#[tokio::test]
async fn images_volumes_networks_and_system_prune() {
    let tmp = TempDir::new().unwrap();
    let mock = MockDockerServer::start(tmp.path()).await;
    let ctx = TestCtxWrapper::new(
        tmp.path().join("data"),
        Some(tmp.path().join("home")),
        Some(mock.socket_path.clone()),
    );
    let app = api_router::<TestCtxWrapper>().with_state(ctx);

    // Images list + pull + remove + prune
    let (s, b) = send_json(app.clone(), Method::GET, "/containers/images", None).await;
    assert_eq!(s, StatusCode::OK);
    let imgs: ImagesListResp = serde_json::from_value(b).unwrap();
    assert_eq!(imgs.images.len(), 2);
    assert!(imgs.images.iter().any(|i| i.in_use && !i.dangling));
    assert!(imgs.images.iter().any(|i| i.dangling && !i.in_use));

    let (s, b) = send_json(
        app.clone(),
        Method::POST,
        "/containers/images/pull",
        Some(json!({ "reference": "alpine:3.20" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let pull: PullImageResp = serde_json::from_value(b).unwrap();
    assert_eq!(pull.digest.as_deref(), Some("sha256:deadbeef"));

    let (s, _) = send_json(
        app.clone(),
        Method::DELETE,
        "/containers/images/sha256:img-dangling?force=true",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let (s, b) = send_json(
        app.clone(),
        Method::POST,
        "/containers/images/prune?dangling_only=true",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let pruned: PruneResp = serde_json::from_value(b).unwrap();
    assert_eq!(pruned.space_reclaimed_bytes, 10_000_000);

    // Volumes list + remove + prune
    let (s, b) = send_json(app.clone(), Method::GET, "/containers/volumes", None).await;
    assert_eq!(s, StatusCode::OK);
    let vols: VolumesListResp = serde_json::from_value(b).unwrap();
    assert_eq!(vols.volumes.len(), 1);
    assert!(vols.volumes[0].in_use);

    let (s, _) = send_json(
        app.clone(),
        Method::DELETE,
        "/containers/volumes/pg_data?force=false",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let (s, b) = send_json(app.clone(), Method::POST, "/containers/volumes/prune", None).await;
    assert_eq!(s, StatusCode::OK);
    let vp: PruneResp = serde_json::from_value(b).unwrap();
    assert_eq!(vp.deleted, vec!["old_vol"]);

    // Networks list + remove + prune
    let (s, b) = send_json(app.clone(), Method::GET, "/containers/networks", None).await;
    assert_eq!(s, StatusCode::OK);
    let nets: NetworksListResp = serde_json::from_value(b).unwrap();
    assert_eq!(nets.networks.len(), 2);
    // Custom networks sort before built-in ones.
    assert!(!nets.networks[0].built_in);
    assert_eq!(nets.networks[0].name, "myapp_default");
    assert_eq!(nets.networks[0].containers.len(), 1);
    assert!(nets.networks[1].built_in);

    let (s, _) = send_json(
        app.clone(),
        Method::DELETE,
        "/containers/networks/net_myapp_111111",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let (s, b) = send_json(
        app.clone(),
        Method::POST,
        "/containers/networks/prune",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let np: PruneResp = serde_json::from_value(b).unwrap();
    assert_eq!(np.deleted, vec!["stale_net"]);

    // System prune with volumes enabled.
    let (s, b) = send_json(
        app,
        Method::POST,
        "/containers/system/prune",
        Some(json!({ "include_volumes": true, "all_unused_images": true })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let sp: SystemPruneResp = serde_json::from_value(b).unwrap();
    assert_eq!(sp.containers_deleted, vec!["stopped_1"]);
    assert_eq!(sp.images_deleted, vec!["sha256:img-dangling"]);
    assert_eq!(sp.networks_deleted, vec!["stale_net"]);
    assert_eq!(sp.volumes_deleted, vec!["old_vol"]);
    assert_eq!(sp.space_reclaimed_bytes, 10_005_120);

    // Verify exact recorded Docker API URIs and query parameters.
    let recorded = mock.recorded();
    assert!(recorded.iter().any(|r| {
        r.method == "POST" && r.uri == "/v1.47/images/create?fromImage=alpine&tag=3.20"
    }));
    assert!(recorded.iter().any(|r| {
        r.method == "DELETE" && r.uri == "/v1.47/images/sha256%3Aimg-dangling?force=true"
    }));
    assert!(recorded.iter().any(|r| {
        r.method == "POST"
            && r.uri == "/v1.47/images/prune?filters=%7B%22dangling%22%3A%5B%22true%22%5D%7D"
    }));
    assert!(recorded.iter().any(|r| {
        r.method == "POST"
            && r.uri == "/v1.47/images/prune?filters=%7B%22dangling%22%3A%5B%22false%22%5D%7D"
    }));
    assert!(recorded
        .iter()
        .any(|r| { r.method == "DELETE" && r.uri == "/v1.47/volumes/pg_data?force=false" }));
    assert!(recorded
        .iter()
        .any(|r| { r.method == "DELETE" && r.uri == "/v1.47/networks/net_myapp_111111" }));
}

#[tokio::test]
async fn context_discovery_and_persisted_selection() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let data = tmp.path().join("data");
    let meta_dir = home.join(".docker/contexts/meta/abc123");
    tokio::fs::create_dir_all(&meta_dir).await.unwrap();

    // Bind a real socket for colima so it is marked reachable.
    let colima_dir = home.join(".colima/default");
    tokio::fs::create_dir_all(&colima_dir).await.unwrap();
    let colima_sock = colima_dir.join("docker.sock");
    let _colima_listener = UnixListener::bind(&colima_sock).unwrap();

    tokio::fs::write(
        home.join(".docker/config.json"),
        json!({ "currentContext": "colima" }).to_string(),
    )
    .await
    .unwrap();
    tokio::fs::write(
        meta_dir.join("meta.json"),
        json!({
            "Name": "colima",
            "Metadata": { "Description": "Colima Linux VM" },
            "Endpoints": {
                "docker": { "Host": format!("unix://{}", colima_sock.display()) }
            }
        })
        .to_string(),
    )
    .await
    .unwrap();

    let ctx = TestCtxWrapper::new(data.clone(), Some(home.clone()), None);
    let app = api_router::<TestCtxWrapper>().with_state(ctx);

    let (s, b) = send_json(app.clone(), Method::GET, "/containers/contexts", None).await;
    assert_eq!(s, StatusCode::OK);
    let list: ContainersContextsResp = serde_json::from_value(b).unwrap();
    assert_eq!(list.active_context, "colima");
    let colima = list.contexts.iter().find(|c| c.name == "colima").unwrap();
    assert!(colima.reachable);
    assert!(colima.is_docker_default);
    assert_eq!(colima.engine_kind, EngineKind::Colima);

    // Select a custom socket path and verify persistence.
    let custom_sock = tmp.path().join("custom.sock");
    let (s2, b2) = send_json(
        app.clone(),
        Method::POST,
        "/containers/contexts/select",
        Some(json!({ "name": "lab", "socket_path": format!("unix://{}", custom_sock.display()) })),
    )
    .await;
    assert_eq!(s2, StatusCode::OK);
    let updated: ContainersContextsResp = serde_json::from_value(b2).unwrap();
    assert_eq!(updated.active_context, "lab");
    assert_eq!(updated.active_socket, custom_sock.to_string_lossy());

    // Reject path traversal.
    let (s_bad, _) = send_json(
        app,
        Method::POST,
        "/containers/contexts/select",
        Some(json!({ "socket_path": "/tmp/../etc/passwd" })),
    )
    .await;
    assert_eq!(s_bad, StatusCode::BAD_REQUEST);
}

#[test]
fn pure_helpers_cover_secret_masking_validation_and_classification() {
    assert!(is_secret_env("POSTGRES_PASSWORD", "1234"));
    assert!(is_secret_env("AWS_SECRET_ACCESS_KEY", "xyz"));
    assert!(is_secret_env("CUSTOM_VAR", "ghp_secret123456"));
    assert!(!is_secret_env("NODE_ENV", "production"));

    let entries = parse_env_entries(&["API_TOKEN=secret".into(), "PORT=3000".into()], false);
    assert_eq!(entries[0].key, "API_TOKEN");
    assert!(entries[0].masked);
    assert_eq!(entries[0].value, "••••••••");
    assert_eq!(entries[1].key, "PORT");
    assert!(!entries[1].masked);

    assert!(normalize_socket_path("relative/docker.sock").is_err());
    assert!(normalize_socket_path("/var/run/../docker.sock").is_err());
    assert_eq!(
        normalize_socket_path("unix:///var/run/docker.sock").unwrap(),
        PathBuf::from("/var/run/docker.sock")
    );

    assert_eq!(
        classify_engine(
            "orbstack",
            Path::new("/Users/u/.orbstack/run/docker.sock"),
            None
        ),
        EngineKind::Orbstack
    );
    assert_eq!(
        classify_engine(
            "default",
            Path::new("/var/run/docker.sock"),
            Some("Docker Desktop")
        ),
        EngineKind::DockerDesktop
    );

    // exec_spec rejects flag-shaped container id or user.
    let bad_req = ContainerExecReq {
        workspace_id: "ws_1".into(),
        command: None,
        user: Some("--privileged".into()),
        working_dir: None,
    };
    assert!(exec_spec(
        Path::new("/usr/local/bin/docker"),
        Path::new("/var/run/docker.sock"),
        "abc123",
        &bad_req
    )
    .is_err());

    let empty_stats = parse_container_stats("c1", &json!({}));
    assert_eq!(empty_stats.cpu_percent, 0.0);
    assert_eq!(empty_stats.memory_percent, 0.0);

    assert!(decode_docker_log_stream(b"", false).is_empty());
}

#[tokio::test]
async fn inspect_image_returns_only_declared_ports_and_404_when_not_local() {
    let tmp = TempDir::new().unwrap();
    let mock = MockDockerServer::start(tmp.path()).await;
    let ctx = TestCtxWrapper::new(
        tmp.path().join("data"),
        Some(tmp.path().join("home")),
        Some(mock.socket_path.clone()),
    );
    let app = api_router::<TestCtxWrapper>().with_state(ctx);

    // A namespaced, tagged reference travels as one encoded path segment.
    let (s, b) = send_json(
        app.clone(),
        Method::GET,
        "/containers/images/bitnami%2Fpostgresql%3A16",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(b.get("env").is_none() && !b.to_string().contains("never-sent"));
    let inspect: ImageInspectResp = serde_json::from_value(b).unwrap();
    assert_eq!(inspect.id, "sha256:img-pg");
    assert_eq!(
        inspect.exposed_ports,
        vec!["53/udp", "5432/tcp", "8080/tcp"]
    );
    assert!(mock.recorded().iter().any(|r| r.method == "GET"
        && r.uri == "/v1.47/images/bitnami/postgresql:16/json"
        && r.background_header.as_deref() == Some("1")));

    let (s, b) = send_json(app, Method::GET, "/containers/images/missing%3A1", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(b.get("code").and_then(|v| v.as_str()), Some("not_found"));
}

#[tokio::test]
async fn error_mapping_covers_400_404_and_502() {
    let tmp = TempDir::new().unwrap();
    let mock = MockDockerServer::start(tmp.path()).await;
    let ctx = TestCtxWrapper::new(
        tmp.path().join("data"),
        Some(tmp.path().join("home")),
        Some(mock.socket_path.clone()),
    );
    let app = api_router::<TestCtxWrapper>().with_state(ctx);

    // 404 from Docker daemon -> 404 Not Found
    let (s404, b404) = send_json(app.clone(), Method::GET, "/containers/missing", None).await;
    assert_eq!(s404, StatusCode::NOT_FOUND);
    assert_eq!(b404.get("code").and_then(|v| v.as_str()), Some("not_found"));

    // Flag-shaped container ID -> 400 Bad Request (rejected before hitting daemon)
    let (s400, b400) = send_json(app, Method::GET, "/containers/--bad-id", None).await;
    assert_eq!(s400, StatusCode::BAD_REQUEST);
    assert_eq!(b400.get("code").and_then(|v| v.as_str()), Some("invalid"));

    // Unreachable socket -> 502 Bad Gateway with clean message
    let dead_ctx = TestCtxWrapper::new(
        tmp.path().join("data-dead"),
        Some(tmp.path().join("home")),
        Some(tmp.path().join("absent.sock")),
    );
    let dead_app = api_router::<TestCtxWrapper>().with_state(dead_ctx);
    let (s502, b502) = send_json(dead_app, Method::GET, "/containers", None).await;
    assert_eq!(s502, StatusCode::BAD_GATEWAY);
    assert_eq!(b502.get("code").and_then(|v| v.as_str()), Some("upstream"));
}

#[tokio::test]
async fn runtime_not_installed_and_external_kiri_wake_stop() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let data = tmp.path().join("data");
    let missing_sock = tmp.path().join("no-kiri.sock");

    // 1. Not installed -> installed: false, can_start: false, install_hint populated.
    let ctx_missing = TestCtxWrapper::new(data.clone(), Some(home.clone()), None).with_kiri(
        Some(tmp.path().join("nonexistent-kiri")),
        Some(missing_sock),
    );
    let app_missing = api_router::<TestCtxWrapper>().with_state(ctx_missing);

    let (s, b) = send_json(
        app_missing.clone(),
        Method::GET,
        "/containers/runtime",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let rt: RuntimeStatusResp = serde_json::from_value(b).unwrap();
    assert_eq!(rt.runtime, "kiri");
    assert!(!rt.installed);
    assert!(!rt.running);
    assert!(!rt.can_start);
    assert!(rt.install_hint.is_some());
    assert!(rt
        .entitlement_note
        .contains("com.apple.security.virtualization"));

    let (s_start, _) =
        send_json(app_missing, Method::POST, "/containers/runtime/start", None).await;
    assert_eq!(s_start, StatusCode::BAD_REQUEST);

    // 2. External Kiri daemon on mock socket -> maps Kiri* /info fields, supports stop & wake.
    let mock = MockDockerServer::start(tmp.path()).await;
    mock.state.lock().unwrap().kiri_mode = true;
    mock.state.lock().unwrap().kiri_stopped = true;

    let fake_bin = data.join("bin/kiri");
    tokio::fs::create_dir_all(fake_bin.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&fake_bin, b"#!/bin/sh\nexit 0\n")
        .await
        .unwrap();

    let ctx_ext = TestCtxWrapper::new(data.clone(), Some(home), Some(mock.socket_path.clone()))
        .with_kiri(Some(fake_bin), Some(mock.socket_path.clone()));
    let app_ext = api_router::<TestCtxWrapper>().with_state(ctx_ext);

    let (s, b) = send_json(app_ext.clone(), Method::GET, "/containers/runtime", None).await;
    assert_eq!(s, StatusCode::OK);
    let rt_stopped: RuntimeStatusResp = serde_json::from_value(b).unwrap();
    assert!(rt_stopped.installed);
    assert!(rt_stopped.daemon_reachable);
    assert!(!rt_stopped.running);
    assert_eq!(rt_stopped.vm_state, "stopped");
    assert_eq!(rt_stopped.version.as_deref(), Some("0.2.1"));
    assert!(rt_stopped.can_start);

    // Start wakes external Kiri via POST /kiri/wake and selects kiri context.
    let (s, b) = send_json(
        app_ext.clone(),
        Method::POST,
        "/containers/runtime/start",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let rt_awake: RuntimeStatusResp = serde_json::from_value(b).unwrap();
    assert!(rt_awake.running);
    assert_eq!(rt_awake.vm_state, "running");

    // Status maps Kiri /info into EngineKind::Kiri + VmResources.
    let (s, b) = send_json(app_ext.clone(), Method::GET, "/containers/status", None).await;
    assert_eq!(s, StatusCode::OK);
    let status: ContainersStatusResp = serde_json::from_value(b).unwrap();
    assert_eq!(status.engine_kind, EngineKind::Kiri);
    assert_eq!(status.vm.as_ref().unwrap().state, "running");
    assert_eq!(
        status.vm.as_ref().unwrap().memory_used_bytes,
        Some(268_435_456)
    );

    // Stop sends POST /kiri/stop to external Kiri daemon.
    let (s, b) = send_json(app_ext, Method::POST, "/containers/runtime/stop", None).await;
    assert_eq!(s, StatusCode::OK);
    let rt_after_stop: RuntimeStatusResp = serde_json::from_value(b).unwrap();
    assert_eq!(rt_after_stop.vm_state, "stopped");
}

#[tokio::test]
async fn runtime_managed_spawn_start_restart_stop_and_early_exit() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let data = tmp.path().join("data");
    let guest_dir = data.join("runtime/guest");
    tokio::fs::create_dir_all(&guest_dir).await.unwrap();
    tokio::fs::write(guest_dir.join("Image"), b"kernel")
        .await
        .unwrap();
    tokio::fs::write(guest_dir.join("initramfs.cpio"), b"initrd")
        .await
        .unwrap();

    // 1. Early-exit binary reports log tail cleanly.
    let fail_bin = tmp.path().join("kiri-fail");
    tokio::fs::write(
        &fail_bin,
        b"#!/bin/sh\necho 'missing virtualization entitlement' >&2\nexit 1\n",
    )
    .await
    .unwrap();
    tokio::fs::set_permissions(&fail_bin, std::fs::Permissions::from_mode(0o755))
        .await
        .unwrap();

    let ctx_fail = TestCtxWrapper::new(data.clone(), Some(home.clone()), None)
        .with_kiri(Some(fail_bin), Some(tmp.path().join("fail.sock")));
    let app_fail = api_router::<TestCtxWrapper>().with_state(ctx_fail);
    let (s, b) = send_json(app_fail, Method::POST, "/containers/runtime/start", None).await;
    assert_eq!(s, StatusCode::BAD_GATEWAY);
    let msg = b.get("message").and_then(|v| v.as_str()).unwrap_or("");
    assert!(msg.contains("exited early"), "unexpected msg: {msg}");
    assert!(
        msg.contains("missing virtualization entitlement"),
        "expected log tail in error: {msg}"
    );

    // 2. Managed headless fake `kiri daemon` (python3 unix HTTP/1.1 server) tests start/stop/restart.
    let managed_data = tmp.path().join("data-managed");
    tokio::fs::create_dir_all(managed_data.join("runtime/guest"))
        .await
        .unwrap();
    tokio::fs::write(managed_data.join("runtime/guest/Image"), b"k")
        .await
        .unwrap();
    tokio::fs::write(managed_data.join("runtime/guest/initramfs.cpio"), b"i")
        .await
        .unwrap();

    let ok_sock = tmp.path().join("managed-kiri.sock");
    let ok_bin = tmp.path().join("kiri-ok");
    let script = r#"#!/usr/bin/env python3
import os, signal, socket, sys

sock_path = os.environ["KIRI_SOCKET"]
if os.path.exists(sock_path):
    os.unlink(sock_path)
srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
srv.bind(sock_path)
srv.listen(8)
print("fake kiri daemon ready", flush=True)

def handle_term(signum, frame):
    try:
        srv.close()
        if os.path.exists(sock_path):
            os.unlink(sock_path)
    finally:
        sys.exit(0)

signal.signal(signal.SIGTERM, handle_term)
while True:
    try:
        conn, _ = srv.accept()
    except OSError:
        break
    data = conn.recv(4096)
    first = data.splitlines()[0].decode("utf-8", "ignore") if data else ""
    if "/info" in first:
        body = b'{"OperatingSystem":"Kiri","KiriVersion":"0.2.1","KiriVmAwake":true,"KiriStopped":false,"NCPU":2}'
        resp = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: " + str(len(body)).encode() + b"\r\nConnection: close\r\n\r\n" + body
    else:
        resp = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK"
    conn.sendall(resp)
    conn.close()
"#;
    tokio::fs::write(&ok_bin, script.as_bytes()).await.unwrap();
    tokio::fs::set_permissions(&ok_bin, std::fs::Permissions::from_mode(0o755))
        .await
        .unwrap();

    let ctx_ok = TestCtxWrapper::new(managed_data.clone(), Some(home), None)
        .with_kiri(Some(ok_bin), Some(ok_sock.clone()));
    let app_ok = api_router::<TestCtxWrapper>().with_state(ctx_ok);

    // Start
    let (s, b) = send_json(
        app_ok.clone(),
        Method::POST,
        "/containers/runtime/start",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "start failed: {b}");
    let started: RuntimeStatusResp = serde_json::from_value(b).unwrap();
    assert!(started.running);
    assert!(started.managed_by_otto);
    assert!(started.pid.is_some());
    assert!(started.guest_assets_present);
    assert_eq!(started.version.as_deref(), Some("0.2.1"));

    // Restart
    let (s, b) = send_json(
        app_ok.clone(),
        Method::POST,
        "/containers/runtime/restart",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "restart failed: {b}");
    let restarted: RuntimeStatusResp = serde_json::from_value(b).unwrap();
    assert!(restarted.running);
    assert!(restarted.managed_by_otto);

    // Stop
    let (s, b) = send_json(app_ok, Method::POST, "/containers/runtime/stop", None).await;
    assert_eq!(s, StatusCode::OK, "stop failed: {b}");
    let stopped: RuntimeStatusResp = serde_json::from_value(b).unwrap();
    assert!(!stopped.managed_by_otto);
    assert!(!stopped.running);

    // Log captured to <data_dir>/logs/kiri.log.
    let log_text = tokio::fs::read_to_string(managed_data.join("logs/kiri.log"))
        .await
        .unwrap();
    assert!(log_text.contains("fake kiri daemon ready"));
}

#[tokio::test]
async fn pull_image_streams_beyond_8mib_and_catches_trailing_error() {
    let tmp = TempDir::new().unwrap();
    let mock = MockDockerServer::start(tmp.path()).await;
    let ctx = TestCtxWrapper::new(
        tmp.path().join("data"),
        Some(tmp.path().join("home")),
        Some(mock.socket_path.clone()),
    );
    let app = api_router::<TestCtxWrapper>().with_state(ctx);

    // >8.5 MiB NDJSON progress stream ending with a trailing error returns 502, never false 200 OK.
    mock.state.lock().unwrap().large_pull_mode = Some(false);
    let (s_err, b_err) = send_json(
        app.clone(),
        Method::POST,
        "/containers/images/pull",
        Some(json!({ "reference": "big/image:latest" })),
    )
    .await;
    assert_eq!(s_err, StatusCode::BAD_GATEWAY);
    assert!(b_err["message"]
        .as_str()
        .unwrap_or("")
        .contains("no space left on device"));

    // >8.5 MiB NDJSON progress stream ending with a digest succeeds and preserves the trailing digest.
    mock.state.lock().unwrap().large_pull_mode = Some(true);
    let (s_ok, b_ok) = send_json(
        app,
        Method::POST,
        "/containers/images/pull",
        Some(json!({ "reference": "big/image:latest" })),
    )
    .await;
    assert_eq!(s_ok, StatusCode::OK);
    let resp: PullImageResp = serde_json::from_value(b_ok).unwrap();
    assert_eq!(resp.digest.as_deref(), Some("sha256:largepullok"));
    assert_eq!(resp.events.len(), 50);
}

#[tokio::test]
async fn list_images_propagates_list_containers_failure() {
    let tmp = TempDir::new().unwrap();
    let mock = MockDockerServer::start(tmp.path()).await;
    mock.state.lock().unwrap().fail_containers_list = true;
    let ctx = TestCtxWrapper::new(
        tmp.path().join("data"),
        Some(tmp.path().join("home")),
        Some(mock.socket_path.clone()),
    );
    let app = api_router::<TestCtxWrapper>().with_state(ctx);

    let (status, _) = send_json(app, Method::GET, "/containers/images", None).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
}

#[test]
fn kiri_vm_state_and_secret_env_classifiers_cover_edge_cases() {
    assert_eq!(
        classify_kiri_vm_state(&json!({ "KiriVersion": "0.2.1" })),
        "running"
    );
    assert_eq!(
        classify_kiri_vm_state(&json!({ "KiriVmAwake": false })),
        "asleep"
    );
    assert_eq!(
        classify_kiri_vm_state(&json!({ "KiriStopped": true, "KiriVmAwake": true })),
        "stopped"
    );

    // Authoritative secret env superset (including REDIS_URL, MONGO_URI, CREDS, PASSPHRASE, and inline URL creds).
    assert!(is_secret_env("REDIS_URL", "redis://localhost:6379"));
    assert!(is_secret_env("MONGO_URI", "mongodb://localhost/app"));
    assert!(is_secret_env("SERVICE_CREDS", "abc"));
    assert!(is_secret_env("GPG_PASSPHRASE", "phrase"));
    assert!(is_secret_env(
        "BROKER_URL",
        "amqp://guest:secret@broker:5672/"
    ));
    assert!(is_secret_env(
        "CACHE_URI",
        "redis://:topsecret@cache:6379/0"
    ));
    assert!(!is_secret_env("PUBLIC_URL", "https://example.com/path"));
    assert!(!is_secret_env("GIT_URL", "ssh://git@github.com/org/repo"));
}

#[tokio::test]
async fn exec_rejects_unauthorized_workspace_role() {
    let tmp = TempDir::new().unwrap();
    let mock = MockDockerServer::start(tmp.path()).await;
    let ctx = TestCtxWrapper::new(
        tmp.path().join("data"),
        Some(tmp.path().join("home")),
        Some(mock.socket_path.clone()),
    )
    .with_role_allowed(false);
    let app = api_router::<TestCtxWrapper>()
        .layer(Extension(test_user()))
        .with_state(ctx);
    let human = RequestCredential {
        agent: false,
        root_withheld: false,
    };

    let (status, _) = with_request_credential(
        human,
        send_json(
            app,
            Method::POST,
            "/containers/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/exec",
            Some(json!({ "workspace_id": "ws_other" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn select_context_with_colliding_name_activates_custom_socket() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let data = tmp.path().join("data");
    let colima_dir = home.join(".colima/default");
    tokio::fs::create_dir_all(&colima_dir).await.unwrap();
    let colima_sock = colima_dir.join("docker.sock");
    let _colima_listener = UnixListener::bind(&colima_sock).unwrap();

    let custom_sock = tmp.path().join("custom-colima.sock");
    let _custom_listener = UnixListener::bind(&custom_sock).unwrap();

    let ctx = TestCtxWrapper::new(data, Some(home), None);
    let app = api_router::<TestCtxWrapper>().with_state(ctx);

    let (s, b) = send_json(
        app,
        Method::POST,
        "/containers/contexts/select",
        Some(json!({
            "name": "colima",
            "socket_path": custom_sock.to_string_lossy()
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let updated: ContainersContextsResp = serde_json::from_value(b).unwrap();
    assert_eq!(updated.active_context, "colima");
    assert_eq!(updated.active_socket, custom_sock.to_string_lossy());
}

#[tokio::test]
async fn concurrent_save_selection_never_corrupts_or_fails() {
    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    let mut handles = Vec::new();
    for i in 0..16 {
        let dir = data.clone();
        handles.push(tokio::spawn(async move {
            save_selection(
                &dir,
                &SavedContextSelection {
                    name: format!("ctx-{i}"),
                    socket_path: format!("/tmp/sock-{i}.sock"),
                },
            )
            .await
        }));
    }
    for h in handles {
        h.await
            .unwrap()
            .expect("save_selection must succeed concurrently");
    }
    let saved = load_saved_selection(&data)
        .await
        .expect("valid JSON after concurrent saves");
    assert!(saved.name.starts_with("ctx-"));
}

#[tokio::test]
async fn concurrent_start_runtime_spawns_only_one_daemon_and_shutdown_stops_it() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let data = tmp.path().join("data-conc");
    let spawn_log = tmp.path().join("spawns.log");
    let sock = tmp.path().join("conc-kiri.sock");
    let bin = tmp.path().join("kiri-conc");
    let script = format!(
        r#"#!/usr/bin/env python3
import os, signal, socket, sys
with open({spawn_log:?}, "a") as f:
    f.write(str(os.getpid()) + "\n")
sock_path = os.environ["KIRI_SOCKET"]
if os.path.exists(sock_path):
    os.unlink(sock_path)
srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
srv.bind(sock_path)
srv.listen(8)
def handle_term(signum, frame):
    try:
        srv.close()
        if os.path.exists(sock_path):
            os.unlink(sock_path)
    finally:
        sys.exit(0)
signal.signal(signal.SIGTERM, handle_term)
while True:
    try:
        conn, _ = srv.accept()
    except OSError:
        break
    conn.recv(4096)
    conn.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK")
    conn.close()
"#,
        spawn_log = spawn_log.to_string_lossy()
    );
    tokio::fs::write(&bin, script.as_bytes()).await.unwrap();
    tokio::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
        .await
        .unwrap();

    let mut cfg = ContainersConfig::new(data);
    cfg.home_dir_override = Some(home);
    cfg.kiri_bin_override = Some(bin);
    cfg.kiri_socket_override = Some(sock);
    let svc = Arc::new(ContainersService::with_config(cfg));

    let (r1, r2) = tokio::join!(svc.start_runtime(), svc.start_runtime());
    let s1 = r1.expect("first start_runtime");
    let s2 = r2.expect("second start_runtime");
    assert!(s1.running && s2.running);
    assert_eq!(s1.pid, s2.pid);

    let spawns = tokio::fs::read_to_string(&spawn_log).await.unwrap();
    assert_eq!(
        spawns.lines().count(),
        1,
        "concurrent start_runtime must spawn exactly one kiri daemon, got: {spawns}"
    );

    // Graceful shutdown stops the managed daemon cleanly.
    svc.shutdown().await;
    let after = svc.runtime_status(None).await;
    assert!(!after.managed_by_otto);
    assert!(!after.running);
}

#[tokio::test]
async fn input_validation_rejects_bad_ports_host_ip_workdir_nul_and_negative_since() {
    let tmp = TempDir::new().unwrap();
    let mock = MockDockerServer::start(tmp.path()).await;
    let ctx = TestCtxWrapper::new(
        tmp.path().join("data"),
        Some(tmp.path().join("home")),
        Some(mock.socket_path.clone()),
    );
    let app = api_router::<TestCtxWrapper>().with_state(ctx);
    let human = RequestCredential {
        agent: false,
        root_withheld: false,
    };

    for bad_run in [
        json!({ "image": "alpine:3.20", "ports": [{ "container_port": 0 }] }),
        json!({ "image": "alpine:3.20", "ports": [{ "container_port": 80, "host_port": 0 }] }),
        json!({ "image": "alpine:3.20", "ports": [{ "container_port": 80, "host_ip": "not-an-ip" }] }),
        json!({ "image": "alpine:3.20", "working_dir": "relative/path" }),
        json!({ "image": "alpine:3.20", "working_dir": "/app\nrm -rf" }),
        json!({ "image": "alpine:3.20", "env": ["KEY=bad\0val"] }),
        json!({ "image": "alpine:3.20", "cmd": ["echo", "bad\0arg"] }),
        json!({ "image": "alpine:3.20", "volumes": ["/tmp/a:/a\0"] }),
    ] {
        let (s, _) = with_request_credential(
            human,
            send_json(app.clone(), Method::POST, "/containers/run", Some(bad_run)),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }

    // Negative `since` on logs returns 400 Invalid.
    let (s_logs, _) = send_json(
        app,
        Method::GET,
        "/containers/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/logs?since=-1",
        None,
    )
    .await;
    assert_eq!(s_logs, StatusCode::BAD_REQUEST);

    // exec_spec rejects control chars in working_dir and NUL bytes in command args.
    let bad_wd = ContainerExecReq {
        workspace_id: "ws_1".into(),
        command: None,
        user: None,
        working_dir: Some("/app\nwhoami".into()),
    };
    assert!(exec_spec(
        Path::new("/usr/local/bin/docker"),
        Path::new("/var/run/docker.sock"),
        "abc123",
        &bad_wd
    )
    .is_err());

    let bad_cmd = ContainerExecReq {
        workspace_id: "ws_1".into(),
        command: Some(vec!["sh".into(), "bad\0arg".into()]),
        user: None,
        working_dir: None,
    };
    assert!(exec_spec(
        Path::new("/usr/local/bin/docker"),
        Path::new("/var/run/docker.sock"),
        "abc123",
        &bad_cmd
    )
    .is_err());
}

fn build_tar_gz(entries: &[(&str, u8, &[u8])]) -> Vec<u8> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    let mut tar_buf = Vec::new();
    for (name, typeflag, data) in entries {
        let mut hdr = [0u8; 512];
        let name_bytes = name.as_bytes();
        assert!(name_bytes.len() <= 100, "test tar name <= 100 bytes");
        hdr[..name_bytes.len()].copy_from_slice(name_bytes);
        hdr[100..107].copy_from_slice(b"0000755");
        let size_oct = format!("{:011o}", data.len());
        hdr[124..135].copy_from_slice(size_oct.as_bytes());
        hdr[156] = *typeflag;
        hdr[257..263].copy_from_slice(b"ustar\0");
        hdr[263..265].copy_from_slice(b"00");
        // Checksum field filled with spaces before summing.
        for b in &mut hdr[148..156] {
            *b = b' ';
        }
        let sum: u32 = hdr.iter().map(|&b| b as u32).sum();
        let sum_oct = format!("{sum:06o}\0 ");
        hdr[148..156].copy_from_slice(sum_oct.as_bytes());

        tar_buf.extend_from_slice(&hdr);
        tar_buf.extend_from_slice(data);
        let pad = (512 - (data.len() % 512)) % 512;
        tar_buf.extend(std::iter::repeat_n(0u8, pad));
    }
    tar_buf.extend(std::iter::repeat_n(0u8, 1024));

    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&tar_buf).unwrap();
    enc.finish().unwrap()
}

#[test]
fn tar_extractor_extracts_headless_assets_and_rejects_traversal_and_links() {
    use crate::install::{extract_kiri_tar_gz, validate_tar_entry_path};

    // Valid upstream `.app` bundle tarball: extracts ONLY headless binary + guest assets.
    let good = build_tar_gz(&[
        ("Kiri.app/Contents/Info.plist", b'0', b"<plist/>"),
        (
            "Kiri.app/Contents/MacOS/kiri",
            b'0',
            b"#!/bin/sh\necho ok\n",
        ),
        ("Kiri.app/Contents/Resources/guest/Image", b'0', b"kern"),
        (
            "Kiri.app/Contents/Resources/guest/initramfs.cpio",
            b'0',
            b"initrd",
        ),
        (
            "Kiri.app/Contents/Resources/guest/data.img.tgz",
            b'0',
            b"disk",
        ),
    ]);
    let extracted = extract_kiri_tar_gz(&good).unwrap();
    assert_eq!(extracted.binary, b"#!/bin/sh\necho ok\n");
    assert_eq!(extracted.guest_files.len(), 3);
    assert!(extracted.guest_files.iter().any(|(n, _)| n == "Image"));
    assert!(extracted
        .guest_files
        .iter()
        .any(|(n, _)| n == "initramfs.cpio"));

    // Reject path traversal (`..`), absolute paths, backslashes.
    assert!(validate_tar_entry_path("../etc/passwd").is_err());
    assert!(validate_tar_entry_path("/etc/passwd").is_err());
    assert!(validate_tar_entry_path("a\\b").is_err());
    let evil_tar = build_tar_gz(&[("Kiri.app/../../evil", b'0', b"nope")]);
    assert!(extract_kiri_tar_gz(&evil_tar).is_err());

    // Reject symlink targeting `kiri`.
    let link_tar = build_tar_gz(&[("Kiri.app/Contents/MacOS/kiri", b'2', b"")]);
    assert!(extract_kiri_tar_gz(&link_tar).is_err());

    // Reject archive without `kiri` binary.
    let empty_tar = build_tar_gz(&[("README.md", b'0', b"hi")]);
    assert!(extract_kiri_tar_gz(&empty_tar).is_err());
}

#[tokio::test]
async fn runtime_install_from_http_tarball_verifies_sha_sets_0755_and_updates_status() {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::PermissionsExt;

    let archive = build_tar_gz(&[
        ("Kiri.app/Contents/MacOS/kiri", b'0', b"#!/bin/sh\nexit 0\n"),
        ("Kiri.app/Contents/Resources/guest/Image", b'0', b"kernel"),
        (
            "Kiri.app/Contents/Resources/guest/initramfs.cpio",
            b'0',
            b"initrd",
        ),
    ]);
    let sha = hex::encode(Sha256::digest(&archive));
    let archive_bytes = Bytes::from(archive);

    // Serve the tarball from a local loopback HTTP/1.1 server.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let srv_handle = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let body = archive_bytes.clone();
            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let svc = service_fn(move |_req: hyper::Request<hyper::body::Incoming>| {
                    let b = body.clone();
                    async move {
                        Ok::<_, std::convert::Infallible>(
                            hyper::Response::builder()
                                .status(StatusCode::OK)
                                .header("content-type", "application/gzip")
                                .body(Full::new(b))
                                .unwrap(),
                        )
                    }
                });
                let _ = http1::Builder::new().serve_connection(io, svc).await;
            });
        }
    });

    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let data = tmp.path().join("data");
    let sock = tmp.path().join("kiri-inst.sock");

    let ctx = TestCtxWrapper::new(data.clone(), Some(home), None).with_kiri(None, Some(sock));
    let app = api_router::<TestCtxWrapper>().with_state(ctx);

    // Initially not installed, can_install = true.
    let (s, b) = send_json(app.clone(), Method::GET, "/containers/runtime", None).await;
    assert_eq!(s, StatusCode::OK);
    let before: RuntimeStatusResp = serde_json::from_value(b).unwrap();
    assert!(!before.installed);
    assert!(before.can_install);
    assert!(!before.installing);

    // Reject non-https non-loopback URL.
    let (s_bad_url, _) = send_json(
        app.clone(),
        Method::POST,
        "/containers/runtime/install",
        Some(json!({ "download_url": "http://example.com/kiri.tar.gz" })),
    )
    .await;
    assert_eq!(s_bad_url, StatusCode::BAD_REQUEST);

    // Checksum mismatch returns 502 and records install_error on GET /containers/runtime.
    let (s_bad_sha, _) = send_json(
        app.clone(),
        Method::POST,
        "/containers/runtime/install",
        Some(json!({
            "download_url": format!("http://{addr}/kiri.tar.gz"),
            "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
        })),
    )
    .await;
    assert_eq!(s_bad_sha, StatusCode::BAD_GATEWAY);
    let (_, b_err) = send_json(app.clone(), Method::GET, "/containers/runtime", None).await;
    let err_status: RuntimeStatusResp = serde_json::from_value(b_err).unwrap();
    assert!(err_status
        .install_error
        .as_deref()
        .unwrap_or("")
        .contains("SHA-256 mismatch"));

    // Valid SHA-256 download installs binary (0755) + guest assets and transitions status.
    let (s_ok, b_ok) = send_json(
        app,
        Method::POST,
        "/containers/runtime/install",
        Some(json!({
            "download_url": format!("http://{addr}/kiri.tar.gz"),
            "sha256": sha
        })),
    )
    .await;
    assert_eq!(s_ok, StatusCode::OK, "install failed: {b_ok}");
    let after: RuntimeStatusResp = serde_json::from_value(b_ok).unwrap();
    assert!(after.installed);
    assert_eq!(after.binary_source.as_deref(), Some("data_dir"));
    assert!(after.guest_assets_present);
    assert!(after.install_error.is_none());
    assert!(after.can_start);

    let bin_path = data.join("bin/kiri");
    let mode = tokio::fs::metadata(&bin_path)
        .await
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o755);
    assert!(data.join("runtime/guest/Image").is_file());
    assert!(data.join("runtime/guest/initramfs.cpio").is_file());

    srv_handle.abort();
}

#[tokio::test]
async fn runtime_install_from_local_source_dir_and_start_after_install() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let data = tmp.path().join("data");
    let stage = tmp.path().join("kiri-stage");
    let sock = tmp.path().join("staged-kiri.sock");

    tokio::fs::create_dir_all(stage.join("bin")).await.unwrap();
    tokio::fs::create_dir_all(stage.join("guest"))
        .await
        .unwrap();
    tokio::fs::write(stage.join("guest/Image"), b"kern")
        .await
        .unwrap();
    tokio::fs::write(stage.join("guest/initramfs.cpio"), b"initrd")
        .await
        .unwrap();

    let script = r#"#!/usr/bin/env python3
import os, signal, socket, sys

sock_path = os.environ["KIRI_SOCKET"]
if os.path.exists(sock_path):
    os.unlink(sock_path)
srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
srv.bind(sock_path)
srv.listen(4)

def handle_term(signum, frame):
    try:
        srv.close()
        if os.path.exists(sock_path):
            os.unlink(sock_path)
    finally:
        sys.exit(0)

signal.signal(signal.SIGTERM, handle_term)
while True:
    try:
        conn, _ = srv.accept()
    except OSError:
        break
    data = conn.recv(4096)
    first = data.splitlines()[0].decode("utf-8", "ignore") if data else ""
    if "/info" in first:
        body = b'{"OperatingSystem":"Kiri","KiriVersion":"0.2.2","KiriVmAwake":true,"KiriStopped":false,"NCPU":2}'
        resp = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: " + str(len(body)).encode() + b"\r\nConnection: close\r\n\r\n" + body
    else:
        resp = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK"
    conn.sendall(resp)
    conn.close()
"#;
    tokio::fs::write(stage.join("bin/kiri"), script.as_bytes())
        .await
        .unwrap();

    let ctx = TestCtxWrapper::new(data.clone(), Some(home), None).with_kiri(None, Some(sock));
    let app = api_router::<TestCtxWrapper>().with_state(ctx);

    let (s, b) = send_json(
        app.clone(),
        Method::POST,
        "/containers/runtime/install",
        Some(json!({
            "source_dir": stage.to_string_lossy(),
            "start_after_install": true
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "source_dir install+start failed: {b}");
    let rt: RuntimeStatusResp = serde_json::from_value(b).unwrap();
    assert!(rt.installed);
    assert!(rt.running);
    assert!(rt.managed_by_otto);
    assert_eq!(rt.version.as_deref(), Some("0.2.2"));

    // Cleanly stop managed process so no stray child lingers.
    let (s_stop, _) = send_json(app, Method::POST, "/containers/runtime/stop", None).await;
    assert_eq!(s_stop, StatusCode::OK);
}
