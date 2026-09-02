//! Kubernetes console — see `docs/features/kubernetes-console.md`.
//!
//! Skeleton: the router is empty until the module is implemented. Server-side
//! wiring (ctx trait impl + router merge) already lives in
//! `crates/otto-server/src/modules.rs`.
use std::sync::Arc;

use axum::Router;
use otto_connections::Spawner;
use otto_core::secrets::SecretStore;
use otto_core::event::Event;
use otto_state::SqlitePool;
use tokio::sync::broadcast;

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
    Router::new()
}
