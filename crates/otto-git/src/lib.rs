//! otto-git — local git operations (shelling out to system `git`) and hosted
//! provider clients (GitHub / Bitbucket Cloud / GitLab) plus the axum router
//! implementing contract endpoints #31–#56.

pub mod history;
pub mod http;
pub mod local;
pub mod ops;
pub mod parse;
pub mod patch;
pub mod pr_checks;
pub mod providers;
pub mod types;

pub use http::{router, GitCtx};
pub use local::{clone_repo, DiffTarget, LocalGit, ResolvedBase};
pub use providers::{detect, make_provider, GitProvider, RemoteRef};
pub use types::CiStatus;
