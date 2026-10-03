//! AWS console — see `docs/features/aws-console.md` and the build contract
//! `docs/design/aws-k8s-consoles.md` (§1, §2).
//!
//! Everything goes through the `aws` CLI v2 (`cli.rs`) except the few per-tick
//! calls `native.rs` signs in-process (F2d); accounts are DB rows
//! (`otto_state::AwsAccountsRepo`) + Keychain secrets (`accounts.rs`); the CLI
//! is located / installed on demand (`install.rs`); each AWS service has its
//! own module with pure normalizers over the CLI's JSON. Server-side wiring
//! (ctx trait impl + router merge) lives in `crates/otto-server/src/modules.rs`.
pub mod access;
pub mod accounts;
pub mod athena;
pub mod cli;
pub mod creds;
pub mod discover;
pub mod ec2;
pub mod eks;
pub mod http;
pub mod install;
pub mod logs;
pub mod metrics;
pub mod native;
pub mod paths;
pub mod rds;
pub mod regions;
pub mod s3;
pub mod s3_download;
pub mod sqs;

use std::sync::Arc;

use axum::Router;
use otto_connections::Spawner;
use otto_core::event::Event;
use otto_core::secrets::SecretStore;
use otto_state::DbPool;
use tokio::sync::broadcast;

/// Server-side context required by the AWS routes.
pub trait AwsCtx: Clone + Send + Sync + 'static {
    fn pool(&self) -> DbPool;
    fn secrets(&self) -> &Arc<dyn SecretStore>;
    fn events(&self) -> &broadcast::Sender<Event>;
    /// Daemon data dir (`~/Library/Application Support/Otto`); auto-installed
    /// binaries go under `<data_dir>/bin`, Otto-owned kubeconfigs under
    /// `<data_dir>/kube`.
    fn data_dir(&self) -> &std::path::Path;
    fn spawner(&self) -> &Arc<dyn Spawner>;
    /// Whether the high-frequency calls (logs tail, query status polls, EC2
    /// lists) go through the in-process SigV4 client instead of the CLI.
    fn aws_native(&self) -> native::Mode {
        native::Mode::from_env()
    }
}

/// All `/aws/*` routes (handler-relative templates; nested under `/api/v1`).
pub fn api_router<S: AwsCtx>() -> Router<S> {
    http::api_router::<S>()
}

pub use accounts::{
    AuthMode, AwsAccount, AwsIdentity, AwsPermissions, AwsService, AwsTestResp, PermState,
    UpsertAwsAccountReq,
};
pub use discover::DiscoveredProfile;
pub use install::{AwsStatus, InstallJob, InstallState};
