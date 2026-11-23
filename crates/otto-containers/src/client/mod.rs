//! Async Docker Engine API v1.47 client over Unix domain sockets.
//!
//! Uses `hyper` 1.x + `tokio::net::UnixStream` (mirroring `otto-k8s`'s proxy
//! gateway), enforces strict timeouts and bounded body caps, and attaches
//! `X-Otto-Background: 1` on background/read requests so idle-sleeping engines
//! are never kept awake by UI polling.

mod containers;
mod logs;
mod parse;
mod resources;
mod secrets;
mod transport;

use std::path::{Path, PathBuf};

use hyper::Method;
use otto_core::Result;
use serde_json::Value;

use crate::contexts::classify_engine;
use crate::types::{ContainersStatusResp, EngineKind};

pub use containers::{validate_container_name, validate_id, validate_image_ref};
pub use logs::decode_docker_log_stream;
pub(crate) use parse::classify_kiri_vm_state;
pub use parse::parse_container_stats;
pub use secrets::{is_secret_env, parse_env_entries};
pub use transport::encode_query_component;

/// Docker Engine API version prefix used for every request.
pub const API_PREFIX: &str = "/v1.47";

/// Header attached to background/poll read requests so idle-sleeping engines
/// do not reset their idle timer.
pub const BACKGROUND_HEADER: &str = "x-otto-background";

/// Upstream Kiri background header (prevents UI polls from keeping Kiri's VM awake).
pub const KIRI_BACKGROUND_HEADER: &str = "x-kiri-background";

/// Async client bound to one Docker Unix socket.
#[derive(Debug, Clone)]
pub struct DockerClient {
    socket_path: PathBuf,
}

impl DockerClient {
    pub fn new(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Fast background `/_ping` probe (`true` when the Unix socket answers HTTP 2xx).
    pub async fn ping_ok(&self) -> bool {
        matches!(
            self.request(
                Method::GET,
                &format!("{API_PREFIX}/_ping"),
                None,
                true,
                transport::PROBE_TIMEOUT,
                4096,
            )
            .await,
            Ok(resp) if (200..300).contains(&resp.status)
        )
    }

    /// Background `GET /info` JSON snapshot for embedded runtime probing.
    pub async fn raw_info(&self) -> Option<Value> {
        self.get_json(
            &format!("{API_PREFIX}/info"),
            true,
            transport::PROBE_TIMEOUT,
        )
        .await
        .ok()
    }

    /// Empty POST to a daemon control endpoint (e.g. `/kiri/wake`, `/kiri/stop`).
    pub async fn post_control(&self, path: &str) -> Result<u16> {
        let resp = self
            .request(
                Method::POST,
                path,
                None,
                false,
                transport::DEFAULT_TIMEOUT,
                4096,
            )
            .await?;
        transport::check_status(&resp)?;
        Ok(resp.status)
    }

    /// Probe daemon health + metadata (`GET /containers/status`).
    pub async fn status(
        &self,
        context_name: &str,
        fallback_engine: EngineKind,
        cli_path: Option<PathBuf>,
    ) -> ContainersStatusResp {
        let socket_str = self.socket_path.to_string_lossy().into_owned();
        let cli_available = cli_path.is_some();
        let cli_path_str = cli_path.map(|p| p.to_string_lossy().into_owned());

        let ping = self
            .request(
                Method::GET,
                &format!("{API_PREFIX}/_ping"),
                None,
                true,
                transport::PROBE_TIMEOUT,
                4096,
            )
            .await;
        if let Err(err) = ping {
            return parse::offline_status(
                context_name,
                socket_str,
                fallback_engine,
                cli_available,
                cli_path_str,
                err.to_string(),
            );
        }

        let info_path = format!("{API_PREFIX}/info");
        let ver_path = format!("{API_PREFIX}/version");
        let (info_res, ver_res): (Result<Value>, Result<Value>) = tokio::join!(
            self.get_json(&info_path, true, transport::PROBE_TIMEOUT),
            self.get_json(&ver_path, true, transport::PROBE_TIMEOUT),
        );

        let info = match info_res {
            Ok(v) => v,
            Err(err) => {
                return parse::offline_status(
                    context_name,
                    socket_str,
                    fallback_engine,
                    cli_available,
                    cli_path_str,
                    err.to_string(),
                );
            }
        };
        let ver = ver_res.unwrap_or(Value::Null);

        let os = parse::str_field(&info, "OperatingSystem");
        let engine_kind = {
            let detected = classify_engine(context_name, &self.socket_path, os.as_deref());
            if detected == EngineKind::Unknown {
                fallback_engine
            } else {
                detected
            }
        };

        let vm = parse::extract_vm_resources(&info);

        ContainersStatusResp {
            available: true,
            error: None,
            active_context: context_name.to_string(),
            socket_path: socket_str,
            engine_kind,
            engine_version: parse::str_field(&ver, "Version")
                .or_else(|| parse::str_field(&info, "ServerVersion")),
            api_version: parse::str_field(&ver, "ApiVersion"),
            os,
            arch: parse::str_field(&info, "Architecture")
                .or_else(|| parse::str_field(&ver, "Arch")),
            containers_running: parse::u64_field(&info, "ContainersRunning"),
            containers_paused: parse::u64_field(&info, "ContainersPaused"),
            containers_stopped: parse::u64_field(&info, "ContainersStopped"),
            images_count: parse::u64_field(&info, "Images"),
            mem_total_bytes: info.get("MemTotal").and_then(|v| v.as_u64()),
            ncpu: info
                .get("NCPU")
                .and_then(|v| v.as_u64())
                .and_then(|n| u32::try_from(n).ok()),
            docker_cli_available: cli_available,
            docker_cli_path: cli_path_str,
            vm,
        }
    }
}
