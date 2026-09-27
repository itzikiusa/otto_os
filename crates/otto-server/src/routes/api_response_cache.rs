//! Full response bodies the API client UI may download later.
//!
//! `POST …/api-client/execute` used to inline EVERY body (up to 25 MB) as
//! base64 in the JSON response, although the viewer renders at most 512 KB of
//! text: a 20 MB export cost ~100 MB of transient daemon memory, 27 MB over
//! loopback and a 27 MB `JSON.parse` on the UI main thread per send. Now only
//! what the viewer needs is inlined (see `attach_body` in `api_client`) and the
//! full bytes of a truncated / non-UTF-8 body wait here, keyed by an opaque
//! `body_id`, for `GET …/api-client/responses/{body_id}/raw` ("Save to disk").
//!
//! Bounded: entries expire after [`TTL`], and the total is capped at
//! [`MAX_TOTAL_BYTES`] (oldest evicted first). An entry is only served to the
//! user and workspace that executed the request.
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue};
use axum::response::Response;
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id};

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// How long a body stays downloadable after the response.
pub(crate) const TTL: Duration = Duration::from_secs(10 * 60);
/// Cap on all cached bodies together (a single body is ≤ 25 MB upstream).
pub(crate) const MAX_TOTAL_BYTES: usize = 128 * 1024 * 1024;

struct Entry {
    id: String,
    workspace_id: Id,
    user_id: Id,
    content_type: Option<String>,
    bytes: Bytes,
    at: Instant,
}

#[derive(Default)]
struct Cache {
    /// Oldest first.
    entries: VecDeque<Entry>,
    total: usize,
}

impl Cache {
    fn expire(&mut self, now: Instant) {
        while self
            .entries
            .front()
            .is_some_and(|e| now.duration_since(e.at) > TTL)
        {
            let e = self.entries.pop_front().expect("front checked");
            self.total -= e.bytes.len();
        }
    }

    fn insert(&mut self, entry: Entry, cap: usize) {
        self.expire(entry.at);
        self.total += entry.bytes.len();
        self.entries.push_back(entry);
        // Evict oldest until within the cap — but never the entry just added.
        while self.total > cap && self.entries.len() > 1 {
            let e = self.entries.pop_front().expect("len > 1");
            self.total -= e.bytes.len();
        }
    }

    fn get(
        &mut self,
        id: &str,
        wid: &str,
        user: &str,
        now: Instant,
    ) -> Option<(Bytes, Option<String>)> {
        self.expire(now);
        self.entries
            .iter()
            .find(|e| e.id == id && e.workspace_id == wid && e.user_id == user)
            .map(|e| (e.bytes.clone(), e.content_type.clone()))
    }
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache::default()))
}

/// Park `bytes` for a later download by `user` in `wid`; returns the body id.
pub(crate) fn put(wid: &Id, user: &Id, content_type: Option<String>, bytes: Vec<u8>) -> String {
    let id = otto_core::new_id();
    let entry = Entry {
        id: id.clone(),
        workspace_id: wid.clone(),
        user_id: user.clone(),
        content_type,
        bytes: Bytes::from(bytes),
        at: Instant::now(),
    };
    cache()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(entry, MAX_TOTAL_BYTES);
    id
}

/// `GET /workspaces/{wid}/api-client/responses/{id}/raw` — the full bytes of a
/// recent response body (404 once expired or evicted).
pub async fn raw(
    Path((wid, id)): Path<(Id, String)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Response> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let found =
        cache()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&id, &wid, &user.id, Instant::now());
    let Some((bytes, content_type)) = found else {
        return Err(ApiError(Error::NotFound(
            "response body (it is kept for 10 minutes — send the request again)".into(),
        )));
    };
    let ct = content_type
        .as_deref()
        .and_then(|c| HeaderValue::from_str(c).ok())
        .unwrap_or_else(|| HeaderValue::from_static("application/octet-stream"));
    let mut resp = Response::new(Body::from(bytes));
    resp.headers_mut().insert(header::CONTENT_TYPE, ct);
    // Never let a text/html body render as a page on the daemon's origin.
    resp.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment"),
    );
    resp.headers_mut().insert(
        header::HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, len: usize, at: Instant) -> Entry {
        Entry {
            id: id.into(),
            workspace_id: "w".into(),
            user_id: "u".into(),
            content_type: None,
            bytes: Bytes::from(vec![0u8; len]),
            at,
        }
    }

    #[test]
    fn scoped_to_workspace_and_user() {
        let mut c = Cache::default();
        let now = Instant::now();
        c.insert(entry("a", 10, now), 1000);
        assert!(c.get("a", "w", "u", now).is_some());
        assert!(c.get("a", "other", "u", now).is_none());
        assert!(c.get("a", "w", "someone", now).is_none());
        assert!(c.get("missing", "w", "u", now).is_none());
    }

    #[test]
    fn evicts_oldest_over_cap_but_keeps_newest() {
        let mut c = Cache::default();
        let now = Instant::now();
        c.insert(entry("a", 60, now), 100);
        c.insert(entry("b", 60, now), 100);
        assert!(c.get("a", "w", "u", now).is_none());
        assert!(c.get("b", "w", "u", now).is_some());
        assert_eq!(c.total, 60);
        // A single body larger than the cap is still kept (alone).
        c.insert(entry("big", 500, now), 100);
        assert!(c.get("big", "w", "u", now).is_some());
        assert_eq!(c.entries.len(), 1);
        assert_eq!(c.total, 500);
    }

    #[test]
    fn expires_after_ttl() {
        let mut c = Cache::default();
        let t0 = Instant::now();
        c.insert(entry("a", 10, t0), 1000);
        let later = t0 + TTL + Duration::from_secs(1);
        assert!(c.get("a", "w", "u", later).is_none());
        assert_eq!(c.total, 0);
    }
}
