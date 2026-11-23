//! Otto Containers — Docker-daemon-backed container console engine & REST API.
//!
//! Speaks Docker Engine API v1.47 directly over a local Unix domain socket
//! (Docker Desktop, Colima, OrbStack, Rancher Desktop, `/var/run/docker.sock`,
//! `~/.otto/run/docker.sock`, or `DOCKER_HOST`). Every read/poll carries
//! `X-Otto-Background: 1` so idle-sleeping engines stay asleep.
//!
//! Server-side wiring mirrors `otto-k8s` / `otto-simulators`:
//! - `pub containers: Arc<otto_containers::ContainersService>` on `ServerCtx`
//! - `impl otto_containers::ContainersCtx for ServerCtx` in
//!   `crates/otto-server/src/modules/ctx_impls.rs`
//! - `otto_containers::api_router::<ServerCtx>()` in
//!   `crates/otto-server/src/modules.rs`

use std::sync::Arc;

use otto_connections::Spawner;
use otto_core::auth::RoleChecker;

pub mod client;
pub mod contexts;
pub mod install;
pub mod routes;
pub mod runtime;
pub mod service;
pub mod types;

#[cfg(test)]
mod tests;

pub use client::{
    decode_docker_log_stream, is_secret_env, parse_container_stats, parse_env_entries,
    DockerClient, API_PREFIX, BACKGROUND_HEADER, KIRI_BACKGROUND_HEADER,
};
pub use contexts::{
    classify_engine, discover_contexts, locate_docker_cli, normalize_socket_path,
    resolve_active_endpoint, ResolvedEndpoint, SavedContextSelection,
};
pub use install::{
    current_install_state, extract_kiri_tar_gz, provision_kiri, validate_tar_entry_path,
    ExtractedKiri, InstallState,
};
pub use routes::{api_router, exec_spec, DEFAULT_SHELL};
pub use runtime::{locate_guest_dir, locate_kiri_binary, resolve_kiri_socket, LocatedKiri};
pub use service::{ContainersConfig, ContainersService};
pub use types::*;

/// Server-side context required by the `/containers/*` routes.
pub trait ContainersCtx: Clone + Send + Sync + 'static {
    /// Instance-scoped containers service owning runtime processes, locks, and cache.
    fn containers(&self) -> &Arc<ContainersService>;

    /// PTY session spawner for `POST /containers/{id}/exec`.
    fn spawner(&self) -> &Arc<dyn Spawner>;

    /// Workspace role checker enforcing Editor access before spawning `exec` PTYs.
    fn roles(&self) -> &Arc<dyn RoleChecker>;
}
