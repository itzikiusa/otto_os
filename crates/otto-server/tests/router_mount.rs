//! Mount guard: every route the source inventory finds is actually SERVED by
//! the assembled daemon router.
//!
//! `route_inventory` (code ↔ contract) and `policy_coverage` (code ↔ RBAC
//! table) parse `.route(…)` calls out of the source, so they cannot see a
//! router that is built but never merged into `module_routers` (easy to do
//! when a crate split moves a router out of otto-server), nor an axum
//! overlap/conflict panic at merge time. This suite builds the real
//! `build_router(ServerCtx::for_tests, module_routers)` — the panic would fire
//! right here — serves it on loopback, and sends every inventoried
//! `(METHOD, path)` with no credentials.
//!
//! A mounted route answers with ITS behavior: 401 from the auth chokepoint for
//! the protected API, a handler status for the public routes, 400/426 for a WS
//! upgrade without the upgrade headers. An unmounted one gets the router's
//! answer instead: 405 (path known, method not), the SPA fallback's
//! `{"code":"not_found","message":"no such route: …"}` for `/api/*` + `/ws/*`,
//! or the placeholder page for any other root path. Each route is tried under
//! `/api/v1` first, then at the root (WS sockets, `/browser/proxy`, plugin
//! assets). No credentials are sent, so no protected handler ever runs.

use std::collections::BTreeMap;
use std::time::Duration;

use otto_server::ServerCtx;
use otto_state::DbPool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

async fn mem_pool() -> DbPool {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect in-memory sqlite");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool.into()
}

/// Fill every `{param}` / `{*rest}` with a harmless concrete segment.
fn concrete(path: &str) -> String {
    let re = regex::Regex::new(r"\{(\*?)[^}]*\}").unwrap();
    re.replace_all(path, |c: &regex::Captures| {
        if &c[1] == "*" {
            "a/b".to_string()
        } else {
            "x1".to_string()
        }
    })
    .into_owned()
}

/// What the router answered for one request, reduced to "did a route match".
#[derive(Debug)]
enum Outcome {
    Matched,
    Unmatched(String),
}

async fn probe(
    http: &reqwest::Client,
    base: &str,
    method: &str,
    url_path: &str,
    placeholder: &str,
) -> Outcome {
    // `any(…)` (the plugin proxy) answers every method; GET exercises it.
    let m = if method == "ANY" { "GET" } else { method };
    let m = reqwest::Method::from_bytes(m.as_bytes()).unwrap();
    let resp = match http
        .request(m, format!("{base}{url_path}"))
        .timeout(Duration::from_secs(15))
        .send()
        .await
    {
        Ok(r) => r,
        // A handler that holds the response open (a stream) still matched;
        // only the router's own answers are immediate. Treat a timeout on
        // headers as a failure to inspect, not a mount.
        Err(e) => return Outcome::Unmatched(format!("request error: {e}")),
    };
    let status = resp.status().as_u16();
    if status == 405 {
        return Outcome::Unmatched("405 method not allowed".into());
    }
    let ctype = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    // Only buffer small, non-streaming bodies (JSON problem / HTML page).
    if status == 404 && ctype.starts_with("application/json") {
        let body = resp.text().await.unwrap_or_default();
        if body.contains("no such route") {
            return Outcome::Unmatched(format!("404 fallback: {body}"));
        }
        return Outcome::Matched;
    }
    if status == 200 && ctype.starts_with("text/html") {
        let body = resp.text().await.unwrap_or_default();
        if body == placeholder {
            return Outcome::Unmatched("SPA placeholder fallback".into());
        }
    }
    Outcome::Matched
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_inventoried_route_is_mounted_on_the_daemon_router() {
    let routes: BTreeMap<(String, String), String> =
        super::route_inventory::registered_routes(&super::route_inventory::repo_root());
    assert!(
        routes.len() >= 1000,
        "inventory broke: {} routes",
        routes.len()
    );

    let pool = mem_pool().await;
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ServerCtx::for_tests(&pool, tmp.path()).await;
    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    // Building + merging is where an axum route overlap panics.
    let app = otto_server::build_router(ctx, api_extras, root_extras);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let http = reqwest::Client::new();

    // The root fallback page (a non-API path no route claims).
    let placeholder = http
        .get(format!("{base}/__otto_router_mount_probe__"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    // Raw literal paths (the inventory keys are normalized; the value is the
    // path as written). Dedupe (method, raw) — `[/{id}]` docs fan out keys.
    let mut cases: Vec<(String, String)> = routes
        .iter()
        .map(|((m, _), raw)| (m.clone(), raw.clone()))
        .collect();
    cases.sort();
    cases.dedup();

    let mut unmounted = Vec::new();
    for (method, raw) in &cases {
        let rel = raw.strip_prefix("/api/v1").unwrap_or(raw);
        let rel = if rel.is_empty() { "/" } else { rel };
        let path = concrete(rel);
        let api = probe(
            &http,
            &base,
            method,
            &format!("/api/v1{path}"),
            &placeholder,
        )
        .await;
        if matches!(api, Outcome::Matched) {
            continue;
        }
        let root = probe(&http, &base, method, &path, &placeholder).await;
        if let Outcome::Unmatched(why) = root {
            unmounted.push(format!("  {method} {raw}  (api: {api:?}; root: {why})"));
        }
    }
    assert!(
        unmounted.is_empty(),
        "{} of {} inventoried route(s) are NOT served by build_router(module_routers) — \
         a router built but never merged (modules.rs `module_routers`), or a path \
         typo:\n{}",
        unmounted.len(),
        cases.len(),
        unmounted.join("\n")
    );
}

#[test]
fn concrete_fills_params_and_wildcards() {
    assert_eq!(concrete("/a/{id}/b/{name}"), "/a/x1/b/x1");
    assert_eq!(concrete("/plugins/{slug}/ui/{*path}"), "/plugins/x1/ui/a/b");
    assert_eq!(concrete("/health"), "/health");
}
