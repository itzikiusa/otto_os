//! Policy-coverage regression test (Task 1.5).
//!
//! Ensures that **every route template registered by the daemon has an
//! intentional RBAC policy entry** — i.e. `policy_for(method, template)`
//! returns `Exempt` or `Require(..)`, never the fail-closed `Deny` default.
//!
//! A newly-added route that someone forgot to classify will make this test
//! fail with the exact list of uncovered `(method, template)` pairs, so the
//! gap is caught in CI rather than silently 403-ing in production.
//!
//! ## Route enumeration
//! Reuses the source scanner from `route_inventory.rs`
//! (`daemon_sources`): every `*.rs` file under `crates/` minus `target/` and
//! `tests/` dirs and minus `#[cfg(test)]` code (unit tests mount mock upstream
//! servers — Telegram, Jira, Confluence — whose routes are not daemon routes),
//! then extracts the first string argument from each `.route(` call.  The scanner correctly handles both single-line and multi-line
//! `.route(` calls.
//!
//! ## Methods tested
//! For each path template we probe with GET (read path) **and** POST (write
//! path).  Both must be non-`Deny`.  For the small number of routes where the
//! method matters for the capability tier (e.g. GET=View vs PUT=Admin), both
//! must still be non-`Deny` (either `Exempt` or `Require(...)`).  We add PUT
//! and DELETE probes as well because a few routes allow only those methods and
//! the policy must cover them.
//!
//! ## Exclusions
//! - `/ws/*` and `/browser/proxy` — WebSocket / proxy routes that
//!   self-authenticate via `?token=` and never reach the central feature guard.
//!   Documented in `policy.rs` and route_inventory's exclusion comment.
//! - `/auth/tokens` (bare path) — handled under the `/auth/tokens` Exempt rule
//!   whether the method is GET or POST; covered correctly.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

// `repo_root` mirrors route_inventory.rs; the source walk is shared with it
// (both suites are modules of the single `it` test binary).

fn repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if dir.join("crates").is_dir() && dir.join("docs/contracts/api.md").is_file() {
            return dir;
        }
        if !dir.pop() {
            panic!("could not locate repo root from CARGO_MANIFEST_DIR");
        }
    }
}

fn extract_route_paths(src: &str) -> Vec<String> {
    let bytes = src.as_bytes();
    let needle = b".route(";
    let mut paths = Vec::new();
    let mut i = 0usize;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            let mut j = i + needle.len();
            while j < bytes.len() && (bytes[j] as char).is_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'"' {
                let start = j + 1;
                let mut k = start;
                let mut esc = false;
                while k < bytes.len() {
                    let c = bytes[k];
                    if esc {
                        esc = false;
                    } else if c == b'\\' {
                        esc = true;
                    } else if c == b'"' {
                        break;
                    }
                    k += 1;
                }
                if k < bytes.len() {
                    let path = &src[start..k];
                    if path.starts_with('/') {
                        paths.push(path.to_string());
                    }
                    i = k + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    paths
}

fn registered_routes(root: &Path) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for (_, src) in &super::route_inventory::daemon_sources(root) {
        if !src.contains(".route(") {
            continue;
        }
        for p in extract_route_paths(src) {
            set.insert(p);
        }
    }
    set
}

/// Routes that are legitimately outside the bearer-auth / feature-policy
/// surface and must be excluded from the coverage check.
///
/// `/ws/*` and `/browser/proxy` use per-session `?token=` authentication and
/// never reach the central feature guard (documented in `policy.rs`).  The
/// route-inventory test also skips `tests/` directories, so test-stub routes
/// are never included in the source set.
fn is_policy_exempt_by_design(path: &str) -> bool {
    // `/ws/*` and `/browser/proxy` self-authenticate via `?token=`. The runtime
    // plugin reverse-proxy + iframe-asset routes (`/plugins/{slug}/…`) are
    // feature-gated by the dedicated plugin branch in `feature_guard` BEFORE
    // `policy_for` is consulted, so they intentionally have no policy-table entry.
    path.starts_with("/ws/") || path == "/browser/proxy" || path.starts_with("/plugins/")
}

#[test]
fn every_protected_route_has_a_policy_entry() {
    use axum::http::Method;
    use otto_server::policy::{policy_for, PolicyDecision};

    let root = repo_root();
    let routes = registered_routes(&root);

    // Sanity floor: guard against a silently broken scanner.
    assert!(
        routes.len() >= 100,
        "extracted only {} routes — scanner likely broke",
        routes.len()
    );

    // Methods we probe for each path.  Covering GET + POST catches most read/
    // write splits; PUT, PATCH and DELETE catch the remaining method-specific
    // rules (PATCH e.g. saved-query rename / dashboard update).
    let probe_methods = [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
    ];

    // For each `(method, template)` pair, assert the policy is not `Deny`.
    // Collect all failures so the developer sees the full list in one run.
    let mut uncovered: Vec<(String, String)> = Vec::new();

    for path_template in &routes {
        if is_policy_exempt_by_design(path_template) {
            continue;
        }

        // The feature guard sees the path with the `/api/v1` nest prefix that
        // the daemon mounts the API router under (see `lib.rs`).  Public routes
        // like `/health` and `/meta` are mounted without the prefix and are
        // matched as-is inside `policy_for`.
        let full_path = if path_template.starts_with("/ws/")
            || path_template == "/browser/proxy"
            || path_template == "/health"
            || path_template == "/meta"
        {
            path_template.clone()
        } else {
            format!("/api/v1{path_template}")
        };

        for method in &probe_methods {
            if policy_for(method, &full_path) == PolicyDecision::Deny {
                uncovered.push((method.to_string(), full_path.clone()));
            }
        }
    }

    // Deduplicate (same path may appear from multiple source files).
    uncovered.sort();
    uncovered.dedup();

    assert!(
        uncovered.is_empty(),
        "{} (method, template) pair(s) have no RBAC policy entry (policy_for returns Deny).\n\
         Add each to `crates/otto-server/src/policy.rs`:\n{}",
        uncovered.len(),
        uncovered
            .iter()
            .map(|(m, p)| format!("  {m} {p}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

/// Snapshot file for [`policy_decisions_match_the_golden_snapshot`].
const POLICY_SNAPSHOT: &str = "tests/snapshots/policy_decisions.txt";
/// Set to `1` to rewrite [`POLICY_SNAPSHOT`] from the current `policy_for`.
const POLICY_SNAPSHOT_UPDATE_ENV: &str = "OTTO_UPDATE_POLICY_SNAPSHOT";

/// Golden snapshot of the WHOLE policy table: `(method, route template) →
/// decision` for every registered route × GET/POST/PUT/PATCH/DELETE.
///
/// `policy_for` is a long ORDERED if-chain, so an innocent-looking new rule
/// can shadow a later one and silently change another route's capability
/// tier — something the "not Deny" check above can never see. Any change to
/// any decision fails here with a line diff; the author reviews it and, if
/// intended, regenerates with
/// `OTTO_UPDATE_POLICY_SNAPSHOT=1 cargo test -p otto-server --test it policy_coverage::`
/// and commits the snapshot alongside the policy change.
#[test]
fn policy_decisions_match_the_golden_snapshot() {
    use axum::http::Method;
    use otto_server::policy::policy_for;

    let root = repo_root();
    let routes = registered_routes(&root);
    assert!(routes.len() >= 100, "scanner likely broke");
    let methods = [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
    ];
    let mut lines = Vec::new();
    for template in &routes {
        // Same mount rule as the coverage test above.
        let full = if template.starts_with("/ws/")
            || template == "/browser/proxy"
            || template == "/health"
            || template == "/meta"
        {
            template.clone()
        } else {
            format!("/api/v1{template}")
        };
        for m in &methods {
            lines.push(format!("{m:<6} {full} => {:?}", policy_for(m, &full)));
        }
    }
    let current = format!(
        "# Generated by policy_coverage::policy_decisions_match_the_golden_snapshot.\n\
         # Do not edit by hand: set {POLICY_SNAPSHOT_UPDATE_ENV}=1 and re-run the test.\n{}\n",
        lines.join("\n")
    );

    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(POLICY_SNAPSHOT);
    if std::env::var(POLICY_SNAPSHOT_UPDATE_ENV).as_deref() == Ok("1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &current).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(&path).unwrap_or_default();
    if golden == current {
        return;
    }
    let old: BTreeSet<&str> = golden.lines().filter(|l| !l.starts_with('#')).collect();
    let new: BTreeSet<&str> = current.lines().filter(|l| !l.starts_with('#')).collect();
    let removed: Vec<_> = old.difference(&new).map(|l| format!("  - {l}")).collect();
    let added: Vec<_> = new.difference(&old).map(|l| format!("  + {l}")).collect();
    panic!(
        "RBAC policy decisions changed vs {POLICY_SNAPSHOT} ({} removed, {} added):\n{}\n{}\n\n\
         Review every line: a `-`/`+` pair on the same route is a capability-tier change.\n\
         If intended, regenerate and commit the snapshot:\n  \
         {POLICY_SNAPSHOT_UPDATE_ENV}=1 cargo test -p otto-server --test it \
         policy_coverage::policy_decisions_match_the_golden_snapshot",
        removed.len(),
        added.len(),
        removed.join("\n"),
        added.join("\n"),
    );
}
