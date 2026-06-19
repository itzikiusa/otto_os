//! Contract-drift guard: every REST route the daemon registers must be
//! documented in `docs/contracts/api.md`.
//!
//! Axum's `Router` doesn't expose its path table for introspection, so instead
//! of asking the live router we parse the source of truth directly: every
//! `.route("PATH", …)` literal across the crates is the canonical registered
//! set. The test extracts that set and asserts each path appears verbatim in
//! `api.md`. If you add a route and forget to document it, this test fails with
//! the exact list of undocumented paths.
//!
//! Implementation note: this uses only `std` (no `regex`/`walkdir` dev-dep) — a
//! small hand-rolled scanner finds each `.route(` and reads the first quoted
//! string literal as the path. Paths carry axum-style params (`{id}`), which is
//! exactly how they are written in `api.md`, so the comparison is a plain
//! substring check.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Resolve the repository root from `CARGO_MANIFEST_DIR` (= crates/otto-server)
/// by walking up until we find a dir that contains both `crates/` and
/// `docs/contracts/api.md`.
fn repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if dir.join("crates").is_dir() && dir.join("docs/contracts/api.md").is_file() {
            return dir;
        }
        if !dir.pop() {
            panic!("could not locate repo root (crates/ + docs/contracts/api.md) from CARGO_MANIFEST_DIR");
        }
    }
}

/// Recursively collect every `*.rs` file under `dir`.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip build artifacts.
            if path.file_name().map(|n| n == "target").unwrap_or(false) {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
            out.push(path);
        }
    }
}

/// Extract every path literal that is the first string argument to a `.route(`
/// call in `src`. Tolerant of whitespace/newlines between `.route(` and the
/// path, and of params like `{id}` / `:id`.
fn extract_route_paths(src: &str) -> Vec<String> {
    let bytes = src.as_bytes();
    let needle = b".route(";
    let mut paths = Vec::new();
    let mut i = 0usize;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            // Scan forward to the first '"' (the start of the path literal),
            // skipping intervening whitespace. Stop early if we hit something
            // that clearly isn't a string-first .route( (e.g. a method call).
            let mut j = i + needle.len();
            // Allow whitespace before the opening quote.
            while j < bytes.len() && (bytes[j] as char).is_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'"' {
                // Read until the closing unescaped quote.
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
                    // Only record things that look like routes (start with '/').
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

/// Collect the canonical registered route set from the crates source tree.
fn registered_routes(root: &Path) -> BTreeSet<String> {
    let mut files = Vec::new();
    rust_files(&root.join("crates"), &mut files);
    let mut set = BTreeSet::new();
    for f in &files {
        let src = std::fs::read_to_string(f).unwrap_or_default();
        if !src.contains(".route(") {
            continue;
        }
        for p in extract_route_paths(&src) {
            set.insert(p);
        }
    }
    set
}

#[test]
fn every_registered_route_is_documented() {
    let root = repo_root();
    let routes = registered_routes(&root);

    // Sanity floor: if extraction silently broke, fail loudly rather than pass
    // a near-empty set. The real count is ~261; 100 is a safe lower bound.
    assert!(
        routes.len() >= 100,
        "extracted only {} routes — extraction likely broke",
        routes.len()
    );

    let api_md = std::fs::read_to_string(root.join("docs/contracts/api.md"))
        .expect("read docs/contracts/api.md");

    let undocumented: Vec<&String> = routes.iter().filter(|p| !api_md.contains(p.as_str())).collect();

    assert!(
        undocumented.is_empty(),
        "{} registered route(s) are NOT documented in docs/contracts/api.md:\n{}",
        undocumented.len(),
        undocumented
            .iter()
            .map(|p| format!("  {p}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

#[test]
fn documented_paths_are_well_formed() {
    // Guard the other direction loosely: every path we extracted is a non-empty,
    // absolute, brace-balanced path. (Catches a future regex/scanner regression
    // that would otherwise let garbage through the substring check.)
    let root = repo_root();
    for p in registered_routes(&root) {
        assert!(!p.is_empty(), "empty route path");
        assert!(p.starts_with('/'), "route path not absolute: {p}");
        let opens = p.matches('{').count();
        let closes = p.matches('}').count();
        assert_eq!(opens, closes, "unbalanced path params in route: {p}");
    }
}
