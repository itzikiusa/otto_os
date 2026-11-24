//! Invalidation events derived from successful HTTP writes (TRANSPORT_PLAN
//! stage 2). Some UI caches have no natural single emit point: effective
//! resource access depends on resource policies, access groups/roles, user
//! grants and workspace roles, all written through different route families.
//! Rather than threading the event bus through each handler, one outer
//! middleware watches for a SUCCESSFUL write on those paths and broadcasts
//! `resource_access_changed`, so the UI's access cache refreshes on change
//! instead of polling every resource every 15 s. The same middleware
//! broadcasts `connections_changed` after a write to the connection library,
//! so every page that lists connections reads one cached list.

use axum::extract::{Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response;
use otto_core::event::Event;
use tokio::sync::broadcast;

/// `(kind, resource_id)` for a write that may change effective access, or
/// `None` when the request cannot. `(None, None)` = "anything may have
/// changed" (a group/role/grant/membership write). `path` is the full request
/// path (`/api/v1/...`).
pub fn access_change_for(method: &Method, path: &str) -> Option<(Option<String>, Option<String>)> {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return None;
    }
    let rest = path.strip_prefix("/api/v1/")?;
    let segs: Vec<&str> = rest.trim_end_matches('/').split('/').collect();
    match segs.as_slice() {
        // Resource policy (`PUT /access/{kind}/{id}`); `…/preview` is a dry run.
        ["access", "groups" | "roles", ..] => Some((None, None)),
        ["access", kind, id] => Some((Some((*kind).to_string()), Some((*id).to_string()))),
        // A user's feature grants, plugin grants, or the user row (disable/delete).
        ["users", _, "grants" | "plugin-grants"] | ["users", _] => Some((None, None)),
        // Workspace membership / role changes.
        ["workspaces", _, "members", ..] => Some((None, None)),
        _ => None,
    }
}

/// Whether a request writes the connection library (profiles or the shared
/// section tree) — not the per-connection DB/SFTP/test/open routes, which
/// change no listed row. `path` is the full request path (`/api/v1/...`).
pub fn connections_change_for(method: &Method, path: &str) -> bool {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return false;
    }
    let Some(rest) = path.strip_prefix("/api/v1/") else {
        return false;
    };
    let segs: Vec<&str> = rest.trim_end_matches('/').split('/').collect();
    matches!(
        segs.as_slice(),
        ["workspaces", _, "connections"]
            | ["workspaces", _, "connections", "import", "create"]
            | ["workspaces", _, "connection-sections", ..]
            | ["connection-sections", ..]
            | ["connections", _]
            | ["connections", _, "duplicate" | "pin"]
    )
}

/// Outer middleware: after a 2xx write that [`access_change_for`] flags,
/// broadcast `resource_access_changed`; after one that
/// [`connections_change_for`] flags, `connections_changed`.
pub async fn notify_access_changes(
    State(tx): State<broadcast::Sender<Event>>,
    req: Request,
    next: Next,
) -> Response {
    let hit = access_change_for(req.method(), req.uri().path());
    let conns = connections_change_for(req.method(), req.uri().path());
    let resp = next.run(req).await;
    if resp.status().is_success() {
        if let Some((kind, resource_id)) = hit {
            let _ = tx.send(Event::ResourceAccessChanged { kind, resource_id });
        }
        if conns {
            let _ = tx.send(Event::ConnectionsChanged {});
        }
    }
    resp
}

// Keep fixture routers in the test tree so production route inventories exclude them.
#[cfg(test)]
#[path = "live_events/tests/mod.rs"]
mod tests;
