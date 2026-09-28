//! Invalidation events derived from successful HTTP writes (TRANSPORT_PLAN
//! stage 2). Some UI caches have no natural single emit point: effective
//! resource access depends on resource policies, access groups/roles, user
//! grants and workspace roles, all written through different route families.
//! Rather than threading the event bus through each handler, one outer
//! middleware watches for a SUCCESSFUL write on those paths and broadcasts
//! `resource_access_changed`, so the UI's access cache refreshes on change
//! instead of polling every resource every 15 s.

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

/// Outer middleware: after a 2xx write that [`access_change_for`] flags,
/// broadcast `resource_access_changed`.
pub async fn notify_access_changes(
    State(tx): State<broadcast::Sender<Event>>,
    req: Request,
    next: Next,
) -> Response {
    let hit = access_change_for(req.method(), req.uri().path());
    let resp = next.run(req).await;
    if let Some((kind, resource_id)) = hit {
        if resp.status().is_success() {
            let _ = tx.send(Event::ResourceAccessChanged { kind, resource_id });
        }
    }
    resp
}

// Keep fixture routers in the test tree so production route inventories exclude them.
#[cfg(test)]
#[path = "live_events/tests/mod.rs"]
mod tests;
