//! Kubernetes console — see `docs/features/kubernetes-console.md` and the build
//! contract `docs/design/aws-k8s-consoles.md` (§3, §4).
//!
//! Everything goes through the `kubectl` binary (`-o json`, normalised in
//! Rust); no kube-rs, no `argocd` CLI, no rollouts plugin. Module map:
//!
//! | module | responsibility |
//! |---|---|
//! | [`cli`] | kubectl runner: base flags (§4.1), timeouts, `forbidden` → 403, `not installed` |
//! | [`install`] | locate ladder + background installers for `kubectl` / `k9s` |
//! | [`clusters`] | registry CRUD, kubeconfig discovery / import, test, capability probe |
//! | [`resources`] | `get -o json` → `K8sRow` per kind (pods health rules, Argo extras, secret redaction, metrics merge) |
//! | [`logs`] | one-shot + `follow` streaming pod logs |
//! | [`actions`] | every §4.6 verb as a planned kubectl argv list |
//! | [`sessions`] | `exec` + k9s PTY sessions via `Spawner::spawn_command` |
//! | [`http`] | the `/k8s/*` axum router |
//!
//! Server-side wiring (ctx trait impl + router merge) lives in
//! `crates/otto-server/src/modules.rs`.
use std::sync::Arc;

use axum::Router;
use otto_connections::Spawner;
use otto_core::event::Event;
use otto_core::secrets::SecretStore;
use otto_state::SqlitePool;
use tokio::sync::broadcast;

pub mod access;
pub mod actions;
pub mod cli;
pub mod clusters;
pub mod http;
pub mod install;
pub mod logs;
pub mod resources;
pub mod sessions;

pub use actions::{K8sActionReq, K8sActionResp};
pub use clusters::{DiscoveredContext, K8sCapabilities, K8sTestResp};
pub use install::{InstallJob, InstallState, Tool, ToolStatus};
pub use resources::{Health, K8sRow, Kind, NodeRow};

/// Server-side context required by the Kubernetes routes.
pub trait K8sCtx: Clone + Send + Sync + 'static {
    fn pool(&self) -> SqlitePool;
    fn secrets(&self) -> &Arc<dyn SecretStore>;
    fn events(&self) -> &broadcast::Sender<Event>;
    /// Daemon data dir (`~/Library/Application Support/Otto`); auto-installed
    /// binaries go under `<data_dir>/bin`, Otto-owned kubeconfigs under
    /// `<data_dir>/kube`.
    fn data_dir(&self) -> &std::path::Path;
    fn spawner(&self) -> &Arc<dyn Spawner>;
}

/// All `/k8s/*` routes (handler-relative templates; nested under `/api/v1`).
pub fn api_router<S: K8sCtx>() -> Router<S> {
    http::api_router::<S>()
}
