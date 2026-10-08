//! otto-git — local git operations (shelling out to system `git`) and hosted
//! provider clients (GitHub / Bitbucket Cloud / GitLab) plus the axum router
//! implementing contract endpoints #31–#56.

#[cfg(test)]
mod config_hardening_tests;
mod diff_cache;
#[cfg(test)]
mod diff_tests;
pub mod history;
pub mod http;
pub mod local;
pub mod ops;
pub mod parse;
pub mod patch;
pub mod pr_checks;
pub mod providers;
pub mod push;
pub mod recovery;
#[cfg(test)]
mod spawn_budget_tests;
mod status_cache;
pub mod types;
pub mod watch;
mod working_diff;
mod worktree_probe;

pub use http::{router, GitCtx};
pub use local::{
    clone_repo, hardened_command, hardened_std_command, DiffOpts, DiffTarget, LocalGit,
    ResolvedBase,
};
pub use providers::{detect, make_provider, GitProvider, RemoteRef};
pub use types::CiStatus;
