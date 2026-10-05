//! otto-server's integration tests, linked as ONE test binary.
//!
//! Every integration test links the whole workspace (~2/5 of it is this
//! crate), so each `tests/*.rs` file used to cost its own multi-hundred-MB
//! link and its own copy on disk. Here each file is a module of a single
//! binary instead: one link per change. Files stay where they are, so
//! `cargo nextest run -p otto-server -E 'test(/^runtime_lag::/)'` or
//! `cargo test -p otto-server --test it runtime_lag::` selects one suite.
//!
//! `snips.rs` stays a binary of its own (`[[test]]` in Cargo.toml): it sets
//! process environment variables, which would leak into the other suites
//! under `cargo test`'s threads-in-one-process runner.
//!
//! Adding a suite: create `tests/<name>.rs` and add a `#[path]` line below —
//! `every_integration_test_file_is_linked` fails until you do (Cargo.toml has
//! `autotests = false`, so an unlisted file would otherwise never run).
#![allow(clippy::disallowed_methods)] // integration tests: plain sync fs / secret store is fine

#[path = "activity_isolation.rs"]
mod activity_isolation;
#[path = "admin_sessions.rs"]
mod admin_sessions;
#[path = "auth_security.rs"]
mod auth_security;
#[path = "canvas_refs_api.rs"]
mod canvas_refs_api;
#[path = "email_sender_storage.rs"]
mod email_sender_storage;
#[path = "grants_api.rs"]
mod grants_api;
#[path = "impersonation.rs"]
mod impersonation;
#[path = "k8s_backfill_budget.rs"]
mod k8s_backfill_budget;
#[path = "k8s_monitor_clickhouse.rs"]
mod k8s_monitor_clickhouse;
#[path = "mcp_auto_approve.rs"]
mod mcp_auto_approve;
#[path = "personal_agent_policy.rs"]
mod personal_agent_policy;
#[path = "policy_coverage.rs"]
mod policy_coverage;
#[path = "provider_resolve.rs"]
mod provider_resolve;
#[path = "rbac_matrix.rs"]
mod rbac_matrix;
#[path = "review_agent_retry.rs"]
mod review_agent_retry;
#[path = "review_comment_states.rs"]
mod review_comment_states;
#[path = "rooms_api.rs"]
mod rooms_api;
#[path = "route_inventory.rs"]
mod route_inventory;
#[path = "run_repo_scope.rs"]
mod run_repo_scope;
#[path = "runtime_lag.rs"]
mod runtime_lag;
#[path = "share_api.rs"]
mod share_api;
#[path = "share_otp.rs"]
mod share_otp;
#[path = "share_scope_guard.rs"]
mod share_scope_guard;
#[path = "ui_control.rs"]
mod ui_control;
#[path = "workbench_api.rs"]
mod workbench_api;

/// Guard: every `tests/*.rs` file is either a module above or a standalone
/// `[[test]]` target (`snips.rs`), so no suite silently stops running.
#[test]
fn every_integration_test_file_is_linked() {
    const STANDALONE: &[&str] = &["it.rs", "snips.rs"];
    let this = include_str!("it.rs");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut missing = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("read tests/") {
        let name = entry.expect("dir entry").file_name();
        let name = name.to_string_lossy();
        if !name.ends_with(".rs") || STANDALONE.contains(&name.as_ref()) {
            continue;
        }
        if !this.contains(&format!("#[path = \"{name}\"]")) {
            missing.push(name.into_owned());
        }
    }
    assert!(
        missing.is_empty(),
        "tests/{missing:?} not linked into tests/it.rs — add `#[path = \"<file>\"] mod <name>;`"
    );
}
