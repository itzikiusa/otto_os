//! Contract-drift guard: the REST routes the daemon registers and the routes
//! `docs/contracts/*.md` document must be the SAME set of exact
//! `(METHOD, PATH)` tuples — in both directions.
//!
//! Axum's `Router` doesn't expose its path table for introspection, so instead
//! of asking the live router we parse the source of truth directly: every
//! `.route("PATH", <method router>)` across the crates yields the path plus
//! the methods named in its method router (`get(…).post(…)`, `delete(…)`).
//!
//! The documented side is parsed from the contract's route ENTRIES only — not
//! prose — so a removed route that a paragraph still mentions is not "stale":
//!   * table rows whose first route cell is `METHOD /path` (optionally numbered,
//!     several `METHOD path` pairs joined by `·`), or a methods cell
//!     (`GET`, `GET / POST`, `POST, DELETE`, `GET\|PUT`, `WS`) followed by a
//!     `` `/path` `` cell;
//!   * headings and bullet items that name `METHOD /path`.
//!
//! Paths are compared after dropping the `/api/v1` prefix, query strings and
//! `[?…]` optional-query notes, and renaming every `{param}` to `{}` (the
//! contract and the code are free to name params differently). `[/{id}]`
//! documents both the bare and the suffixed path. `WS` is a GET upgrade.
//! (A plain substring check used to pass any route whose path happened to be a
//! prefix of a documented one, and never caught a documented route that no
//! longer exists.)

use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
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
            // Skip build artifacts and integration-test trees. Test files may
            // register throwaway stub routes (e.g. the RBAC guard matrix mounts
            // a minimal router at the real templates) that are NOT daemon routes
            // and must not be held to the api.md contract.
            let name = path.file_name();
            if name.map(|n| n == "target" || n == "tests").unwrap_or(false) {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
            out.push(path);
        }
    }
}

const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "WS", "ANY"];

/// Normalise a route path for comparison (see the module docs). Returns every
/// concrete path a documented spelling stands for (`[/{id}]` → two).
fn normalize(raw: &str) -> Vec<String> {
    let mut p = raw.trim().trim_matches('`').to_string();
    if let Some(i) = p.find('?') {
        p.truncate(i);
    }
    p = p
        .trim_end_matches(['`', '.', ',', ';', ':', '\\'])
        .to_string();
    let mut out = Vec::new();
    if let Some(open) = p.find('[') {
        let base = p[..open].to_string();
        let inner = p[open + 1..].trim_end_matches(']').to_string();
        out.push(base.clone());
        if inner.starts_with('/') {
            out.push(format!("{base}{}", inner.trim_end_matches(']')));
        }
    } else {
        out.push(p);
    }
    let param = Regex::new(r"\{[^}]*\}").unwrap();
    out.into_iter()
        .map(|p| {
            let p = p.strip_prefix("/api/v1").unwrap_or(&p).to_string();
            let p = if p.is_empty() { "/".to_string() } else { p };
            param.replace_all(&p, "{}").into_owned()
        })
        .collect()
}

/// Methods named in a contract methods token list (`GET / POST`, `GET\|PUT`,
/// `POST, DELETE`). `WS` is the GET upgrade the router registers. `None` when
/// the text is not purely a method list.
fn parse_methods(text: &str) -> Option<Vec<String>> {
    let toks: Vec<&str> = text
        .split(|c: char| c == '/' || c == ',' || c == '|' || c == '\\' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .collect();
    if toks.is_empty() || !toks.iter().all(|t| METHODS.contains(t)) {
        return None;
    }
    Some(
        toks.into_iter()
            .map(|t| if t == "WS" { "GET" } else { t }.to_string())
            .collect(),
    )
}

/// Split a markdown table row on unescaped `|`.
fn cells(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev = '\0';
    for c in line.trim().trim_start_matches('|').chars() {
        if c == '|' && prev != '\\' {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
        prev = c;
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out.into_iter().map(|c| c.trim().to_string()).collect()
}

/// The documented `(METHOD, PATH)` set from one contract file's route entries.
fn documented_routes_in(md: &str, out: &mut BTreeSet<(String, String)>) {
    // `METHOD[\|METHOD…] /path` — the inline spelling.
    let pair = Regex::new(
        r"(?:^|[^A-Za-z])((?:GET|POST|PUT|PATCH|DELETE|WS|ANY)(?:\s*(?:\\\||/|,)\s*(?:GET|POST|PUT|PATCH|DELETE|WS|ANY))*)\s+`?(/[^\s|`]*)",
    )
    .unwrap();
    // `` `METHOD /path` `` — one backticked route inside prose.
    let tick = Regex::new(r"`((?:GET|POST|PUT|PATCH|DELETE|WS|ANY)(?:\s*(?:/|,)\s*(?:GET|POST|PUT|PATCH|DELETE|WS|ANY))*)\s+(/[^\s`]*)`").unwrap();
    let add = |methods: &[String], path: &str, out: &mut BTreeSet<(String, String)>| {
        for p in normalize(path) {
            for m in methods {
                out.insert((m.clone(), p.clone()));
            }
        }
    };
    let mut in_code = false;
    for line in md.lines() {
        let t = line.trim_start();
        if t.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        if t.starts_with('|') {
            let cs = cells(t);
            for (i, c) in cs.iter().enumerate() {
                // `| GET / POST | `/path` |`
                if let Some(ms) = parse_methods(c) {
                    if let Some(next) = cs.get(i + 1) {
                        if next.trim_matches('`').starts_with('/') {
                            add(&ms, next, out);
                        }
                    }
                    break;
                }
                // `| 102 | GET /api/v1/… |` / `| GET `/a` · GET `/b` |`
                let hits: Vec<_> = pair.captures_iter(c).collect();
                if !hits.is_empty()
                    && c.trim_start_matches('`')
                        .starts_with(|ch: char| ch.is_ascii_uppercase())
                {
                    for h in hits {
                        if let Some(ms) = parse_methods(&h[1]) {
                            add(&ms, &h[2], out);
                        }
                    }
                    break;
                }
                // Only a leading row-number cell (`16a`, `—`) may precede the route cell.
                if !(c == "—" || c == "-" || c.chars().all(|ch| ch.is_ascii_alphanumeric())) {
                    break;
                }
            }
        } else if t.starts_with('#') {
            for h in pair.captures_iter(t) {
                if let Some(ms) = parse_methods(&h[1]) {
                    add(&ms, &h[2], out);
                }
            }
        } else {
            // A bullet (`- GET /x`, `` - `GET /x` ``) or a paragraph that OPENS
            // with a backticked route documents it — and every backticked
            // route on that opening line (`` `GET /a` and `POST /b` ``).
            let body = t
                .strip_prefix("- ")
                .or_else(|| t.strip_prefix("* "))
                .unwrap_or(t);
            let is_bullet = body.len() != t.len();
            let opens = pair
                .captures(body.trim_start_matches('`'))
                .is_some_and(|h| h.get(0).unwrap().start() == 0);
            if !opens || !(is_bullet || body.starts_with('`')) {
                continue;
            }
            if body.starts_with('`') {
                for h in tick.captures_iter(body) {
                    if let Some(ms) = parse_methods(&h[1]) {
                        add(&ms, &h[2], out);
                    }
                }
            } else if let Some(h) = pair.captures(body) {
                if let Some(ms) = parse_methods(&h[1]) {
                    add(&ms, &h[2], out);
                }
            }
        }
    }
}

fn documented_routes(root: &Path) -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    let dir = root.join("docs/contracts");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("read docs/contracts")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    files.sort();
    for f in files {
        documented_routes_in(&std::fs::read_to_string(&f).unwrap(), &mut out);
    }
    out
}

/// Every `.route("PATH", <method router>)` in `src`: the path literal and the
/// methods its router names (`ANY` for `any(…)`; empty when not inferable).
fn extract_routes(src: &str) -> Vec<(String, BTreeSet<String>)> {
    let head = Regex::new(r#"\.route\(\s*"(/[^"]*)"\s*,"#).unwrap();
    let method =
        Regex::new(r"(?:^|[^A-Za-z0-9_])(get|post|put|patch|delete|any)(?:_service)?\s*\(")
            .unwrap();
    let mut out = Vec::new();
    for cap in head.captures_iter(src) {
        let path = cap[1].to_string();
        // The method-router expression runs to the `)` closing `.route(`.
        let body = &src[cap.get(0).unwrap().end()..];
        let mut depth = 1i32;
        let mut end = body.len();
        for (i, c) in body.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let methods = method
            .captures_iter(&body[..end])
            .map(|m| m[1].to_ascii_uppercase())
            .collect();
        out.push((path, methods));
    }
    out
}

/// Collect the canonical registered `(METHOD, PATH)` set from the crates
/// source tree, keyed back to the literal path for readable failures.
fn registered_routes(root: &Path) -> BTreeMap<(String, String), String> {
    let mut files = Vec::new();
    rust_files(&root.join("crates"), &mut files);
    let mut set = BTreeMap::new();
    for f in &files {
        let src = std::fs::read_to_string(f).unwrap_or_default();
        if !src.contains(".route(") {
            continue;
        }
        for (p, methods) in extract_routes(&src) {
            // Test-only seed routes (gated by OTTO_E2E, policy-exempt) are not part
            // of the public API contract and are intentionally absent from api.md.
            if p.contains("__e2e") {
                continue;
            }
            assert!(
                !methods.is_empty(),
                "{}: cannot infer the methods of route {p}",
                f.display()
            );
            for np in normalize(&p) {
                for m in &methods {
                    set.insert((m.clone(), np.clone()), p.clone());
                }
            }
        }
    }
    set
}

#[test]
fn registered_and_documented_routes_match_exactly() {
    let root = repo_root();
    let routes = registered_routes(&root);
    let documented = documented_routes(&root);

    // Sanity floors: if extraction silently broke, fail loudly rather than
    // compare near-empty sets. The real counts are ~1,270 each.
    assert!(
        routes.len() >= 1000,
        "extracted only {} routes — extraction likely broke",
        routes.len()
    );
    assert!(
        documented.len() >= 1000,
        "parsed only {} documented routes — parsing likely broke",
        documented.len()
    );

    // `any(…)` (the plugin proxy) is documented as `ANY`.
    let undocumented: Vec<String> = routes
        .iter()
        .filter(|(k, _)| !documented.contains(k))
        .map(|((m, _), raw)| format!("  {m} {raw}"))
        .collect();
    let stale: Vec<String> = documented
        .iter()
        .filter(|k| !routes.contains_key(*k))
        .map(|(m, p)| format!("  {m} {p}"))
        .collect();

    assert!(
        undocumented.is_empty() && stale.is_empty(),
        "route contract drift.\n{} registered route(s) NOT documented in docs/contracts/*.md:\n{}\n\
         {} documented route(s) the daemon does NOT register (params shown as {{}}):\n{}",
        undocumented.len(),
        undocumented.join("\n"),
        stale.len(),
        stale.join("\n"),
    );
}

#[test]
fn contract_parser_reads_every_route_spelling() {
    let md = "\
| 33 | DELETE /api/v1/git/accounts/{id} | member | — | 204 |
| GET / POST | `/access/groups` | List / create |
| POST, DELETE | `/room-recaps/{id}/summary` | x |
| 137 | GET\\|PUT /api/v1/repos/{id}/proof-config | x |
| GET `/plugins/{slug}/ui` · GET `/plugins/{slug}/ui/{*path}` | public | x |
| WS | `/ws/rooms/{id}` | x |
| GET /api/v1/canvas/scenes/{id}[?files=ref] | x |
| GET /aws/accounts[/{id}] | x |
### Actions (`POST /k8s/clusters/{id}/actions`)
- `GET /database-changes?connection_id=<id>` lists
- GET /usage/by-kind?days=N → rollup
| — | POST /api/v1/sessions/{id}/resume | x |
| ANY `/plugins/{slug}` · ANY `/plugins/{slug}/{*rest}` | x |
`GET /api/v1/state/formats` and `POST /api/v1/state/export` open the paragraph.
- prose mentioning `POST /repos/{id}/checkout-update` is not an entry
A paragraph citing `POST /mid/sentence` is not one either.
Removed: `POST /gone` never counts.
```
GET /in/a/code/block
```
";
    let mut got = BTreeSet::new();
    documented_routes_in(md, &mut got);
    let want: BTreeSet<(String, String)> = [
        ("DELETE", "/git/accounts/{}"),
        ("GET", "/access/groups"),
        ("POST", "/access/groups"),
        ("POST", "/room-recaps/{}/summary"),
        ("DELETE", "/room-recaps/{}/summary"),
        ("GET", "/repos/{}/proof-config"),
        ("PUT", "/repos/{}/proof-config"),
        ("GET", "/plugins/{}/ui"),
        ("GET", "/plugins/{}/ui/{}"),
        ("GET", "/ws/rooms/{}"),
        ("GET", "/canvas/scenes/{}"),
        ("GET", "/aws/accounts"),
        ("GET", "/aws/accounts/{}"),
        ("POST", "/k8s/clusters/{}/actions"),
        ("GET", "/database-changes"),
        ("GET", "/usage/by-kind"),
        ("POST", "/sessions/{}/resume"),
        ("ANY", "/plugins/{}"),
        ("ANY", "/plugins/{}/{}"),
        ("GET", "/state/formats"),
        ("POST", "/state/export"),
    ]
    .into_iter()
    .map(|(m, p)| (m.to_string(), p.to_string()))
    .collect();
    assert_eq!(got, want);
}

#[test]
fn route_extractor_reads_method_routers() {
    let src = r#"
        Router::new()
            .route("/a/{id}", get(h::read).put(h::write).delete(h::rm))
            .route(
                "/b",
                post(with_limit(h::create)),
            )
            .route("/p/{slug}/{*rest}", any(proxy))
    "#;
    let got = extract_routes(src);
    let m = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
    assert_eq!(
        got,
        vec![
            ("/a/{id}".to_string(), m(&["GET", "PUT", "DELETE"])),
            ("/b".to_string(), m(&["POST"])),
            ("/p/{slug}/{*rest}".to_string(), m(&["ANY"])),
        ]
    );
}

#[test]
fn documented_paths_are_well_formed() {
    // Every path we extracted is a non-empty, absolute, brace-balanced path.
    // (Catches a future scanner regression that would let garbage through.)
    let root = repo_root();
    for p in registered_routes(&root).into_values() {
        assert!(!p.is_empty(), "empty route path");
        assert!(p.starts_with('/'), "route path not absolute: {p}");
        let opens = p.matches('{').count();
        let closes = p.matches('}').count();
        assert_eq!(opens, closes, "unbalanced path params in route: {p}");
    }
}
