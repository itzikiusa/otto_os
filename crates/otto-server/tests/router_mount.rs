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
//!
//! It also pins WHO MAY CALL WITHOUT A TOKEN (test-quality-1-01): every
//! matched probe must answer 401 unless it is in [`PUBLIC_ROUTES`]. A route
//! moved out of the `protected` router (or a new router added to the root
//! vec) therefore fails here with a diff instead of going public silently —
//! making a route public is a reviewed edit to that list.

use std::collections::{BTreeMap, BTreeSet};
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
    /// A route answered, with this status.
    Matched(u16),
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
        return Outcome::Matched(status);
    }
    if status == 200 && ctype.starts_with("text/html") {
        let body = resp.text().await.unwrap_or_default();
        if body == placeholder {
            return Outcome::Unmatched("SPA placeholder fallback".into());
        }
    }
    Outcome::Matched(status)
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
    // Every matched probe that did NOT answer 401 without credentials.
    let mut open = BTreeSet::new();
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
        if let Outcome::Matched(status) = api {
            if status != 401 {
                open.insert(format!("{method} /api/v1{rel}"));
            }
            continue;
        }
        match probe(&http, &base, method, &path, &placeholder).await {
            Outcome::Matched(status) => {
                if status != 401 {
                    open.insert(format!("{method} {rel}"));
                }
            }
            Outcome::Unmatched(why) => {
                unmounted.push(format!("  {method} {raw}  (api: {api:?}; root: {why})"));
            }
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

    let pinned: BTreeSet<String> = PUBLIC_ROUTES.iter().map(|s| s.to_string()).collect();
    let newly_open: Vec<_> = open
        .difference(&pinned)
        .map(|l| format!("  + {l}"))
        .collect();
    let now_closed: Vec<_> = pinned
        .difference(&open)
        .map(|l| format!("  - {l}"))
        .collect();
    assert!(
        newly_open.is_empty() && now_closed.is_empty(),
        "the set of routes answering WITHOUT credentials (anything but 401) changed vs \
         PUBLIC_ROUTES ({} newly open, {} no longer open):\n{}\n{}\n\n\
         `+` is reachable with no token — if that is intended, add it to PUBLIC_ROUTES \
         (and its contract); `-` now answers 401 — drop it from the list.",
        newly_open.len(),
        now_closed.len(),
        newly_open.join("\n"),
        now_closed.join("\n"),
    );
}

/// Every route that answers a credential-less request with something other
/// than 401 (its handler ran, or a WS upgrade refused the missing headers):
/// `METHOD path` with the path as registered — `/api/v1…` for the API, bare
/// for the root routes. `ANY` is an `any(…)` route probed with GET. Pinned
/// from the daemon's behaviour; see the module docs.
const PUBLIC_ROUTES: &[&str] = &[
    // Liveness, daemon metadata, sign-in and first-run onboarding.
    "GET /api/v1/health",
    "GET /api/v1/meta",
    "POST /api/v1/auth/login",
    "POST /api/v1/onboarding/root",
    // OAuth2 redirect target of the API client (state-bound in the handler).
    "GET /api/v1/api-client/oauth2/callback",
    // Agent/CLI sidecar ingest — authenticated by its own key, not a bearer.
    "POST /api/v1/ingest/claude",
    "POST /api/v1/ingest/codex",
    "POST /api/v1/ingest/swarm/board",
    "POST /api/v1/ingest/swarm/discovery-report",
    "POST /api/v1/ingest/swarm/mockup",
    "POST /api/v1/ingest/swarm/product",
    "POST /api/v1/ingest/usage",
    // DB Assistant `q` tool — per-assist `x-assist-key`.
    "POST /api/v1/db-assist/{aid}/query",
    // Share-link guests (email OTP verify / extend).
    "POST /api/v1/share/extend",
    "POST /api/v1/share/verify",
    // Inbound webhooks — per-workspace webhook key.
    "POST /api/v1/webhooks/{workspace_id}",
    "POST /api/v1/webhooks/{workspace_id}/run",
    "POST /api/v1/webhooks/swarm/{workspace_id}/{swarm_id}",
    // Room / game-room guests join with an invite credential.
    "POST /api/v1/room-join",
    "POST /api/v1/game-room-join",
    // Plugin host API — the plugin's own host token.
    "GET /api/v1/plugin-host/jira/credentials",
    "POST /api/v1/plugin-host/agents/run",
    // Root routes: WS upgrades (authenticated in the upgrade / first frame;
    // a bare GET is refused for missing upgrade headers) and the browser proxy.
    "GET /browser/proxy",
    "GET /ws/api-client/stream",
    "GET /ws/browser/{tab_id}/live",
    "GET /ws/events",
    "GET /ws/game-rooms/{id}",
    "GET /ws/lsp",
    "GET /ws/rooms/{id}",
    "GET /ws/rooms/{id}/terminal",
];

#[test]
fn concrete_fills_params_and_wildcards() {
    assert_eq!(concrete("/a/{id}/b/{name}"), "/a/x1/b/x1");
    assert_eq!(concrete("/plugins/{slug}/ui/{*path}"), "/plugins/x1/ui/a/b");
    assert_eq!(concrete("/health"), "/health");
}
