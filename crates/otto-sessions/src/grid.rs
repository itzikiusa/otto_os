//! The per-session grid feed behind the WS `grid` frame (docs/contracts/ws.md
//! "Grid feed"): every terminal viewer learns the PTY's REAL grid and which
//! attached connection sizes it. Without it a viewer could not tell another
//! viewer had re-sized the PTY and kept painting that grid's output into its
//! own (stale bottom rows, hard-wrapped TUI frames) until a restart.

use axum::extract::ws::{Message, WebSocket};
use dashmap::DashMap;
use otto_core::Id;
use tokio::sync::watch;

/// A session's PTY grid as its viewers must know it: the real `cols`×`rows`
/// (0×0 while no process is live) and the ATTACHED connection that owns the
/// size, if any (a detached owner reads as `None`: authority is sticky
/// server-side, but the remaining viewers may take the grid back).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GridState {
    pub cols: u16,
    pub rows: u16,
    pub owner: Option<u64>,
}

/// One `watch` channel per watched session.
#[derive(Default)]
pub(crate) struct GridFeeds(DashMap<Id, watch::Sender<GridState>>);

impl GridFeeds {
    /// Subscribe to `id` (the channel is created at `now` on first use).
    pub(crate) fn watch(&self, id: &Id, now: GridState) -> watch::Receiver<GridState> {
        self.0
            .entry(id.clone())
            .or_insert_with(|| watch::channel(now).0)
            .subscribe()
    }

    /// Push `now` to `id`'s viewers when it changed. `now` is computed by the
    /// caller only when somebody watches (`state` is lazy).
    pub(crate) fn publish(&self, id: &Id, state: impl FnOnce() -> GridState) {
        let Some(tx) = self.0.get(id).map(|t| t.clone()) else {
            return;
        };
        let now = state();
        tx.send_if_modified(|g| {
            let changed = *g != now;
            *g = now;
            changed
        });
    }

    /// Drop the feed once nobody holds a receiver (a viewer attaching right
    /// now may already hold one, so this is not keyed on the attach count).
    pub(crate) fn forget_if_unwatched(&self, id: &Id) {
        self.0.remove_if(id, |_, tx| tx.receiver_count() == 0);
    }

    pub(crate) fn remove(&self, id: &Id) {
        self.0.remove(id);
    }

    #[cfg(test)]
    pub(crate) fn contains(&self, id: &Id) -> bool {
        self.0.contains_key(id)
    }
}

/// The `grid` frame for one viewer: owner `"self"` (this connection),
/// `"other"` (another attached one) or `"none"`.
pub(crate) fn grid_frame(g: GridState, conn_id: u64) -> String {
    let owner = match g.owner {
        Some(o) if o == conn_id => "self",
        Some(_) => "other",
        None => "none",
    };
    serde_json::json!({ "type": "grid", "cols": g.cols, "rows": g.rows, "owner": owner })
        .to_string()
}

/// Next change; `None` once the feed is gone (never ready while `rx` is None).
pub(crate) async fn next_grid(rx: &mut Option<watch::Receiver<GridState>>) -> Option<GridState> {
    match rx.as_mut() {
        Some(r) => match r.changed().await {
            Ok(()) => Some(*r.borrow_and_update()),
            Err(_) => None,
        },
        None => std::future::pending().await,
    }
}

/// Send the current grid (`force`), or only a pending change — before a
/// snapshot taken at that grid, so the client never judges it against the
/// old one.
pub(crate) async fn send_grid(
    rx: &mut Option<watch::Receiver<GridState>>,
    socket: &mut WebSocket,
    conn_id: u64,
    force: bool,
) -> Result<(), ()> {
    let Some(r) = rx.as_mut() else { return Ok(()) };
    if !force && !r.has_changed().unwrap_or(false) {
        return Ok(());
    }
    let g = *r.borrow_and_update();
    let frame = grid_frame(g, conn_id);
    socket
        .send(Message::Text(frame.into()))
        .await
        .map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(g: GridState, conn: u64) -> serde_json::Value {
        serde_json::from_str(&grid_frame(g, conn)).unwrap()
    }

    /// docs/contracts/ws.md "Grid feed": `owner` is relative to the viewer.
    #[test]
    fn the_grid_frame_names_the_owner_relative_to_each_viewer() {
        let g = GridState {
            cols: 100,
            rows: 30,
            owner: Some(7),
        };
        assert_eq!(
            frame(g, 7),
            serde_json::json!({"type": "grid", "cols": 100, "rows": 30, "owner": "self"})
        );
        assert_eq!(frame(g, 8)["owner"], "other");
        let idle = GridState::default();
        assert_eq!(
            frame(idle, 7),
            serde_json::json!({"type": "grid", "cols": 0, "rows": 0, "owner": "none"})
        );
    }

    #[test]
    fn a_feed_publishes_only_changes_and_only_while_watched() {
        let feeds = GridFeeds::default();
        let id: Id = "s1".into();
        // Nobody watching: the state closure is never even evaluated.
        feeds.publish(&id, || panic!("computed for an unwatched session"));
        assert!(!feeds.contains(&id));

        let start = GridState {
            cols: 80,
            rows: 24,
            owner: None,
        };
        let mut rx = feeds.watch(&id, start);
        assert_eq!(*rx.borrow_and_update(), start);
        feeds.publish(&id, || start);
        assert!(!rx.has_changed().unwrap(), "same grid → no frame");
        let resized = GridState {
            cols: 120,
            owner: Some(1),
            ..start
        };
        feeds.publish(&id, || resized);
        assert!(rx.has_changed().unwrap());
        assert_eq!(*rx.borrow_and_update(), resized);

        // A second viewer joins the existing feed (its initial value is ignored).
        let rx2 = feeds.watch(&id, GridState::default());
        assert_eq!(*rx2.borrow(), resized);

        // Still watched → kept; once every receiver is gone → dropped.
        drop(rx);
        feeds.forget_if_unwatched(&id);
        assert!(feeds.contains(&id));
        drop(rx2);
        feeds.forget_if_unwatched(&id);
        assert!(!feeds.contains(&id));
    }

    #[tokio::test]
    async fn next_grid_ends_when_the_feed_is_removed() {
        let feeds = GridFeeds::default();
        let id: Id = "s1".into();
        let mut rx = Some(feeds.watch(&id, GridState::default()));
        let g = GridState {
            cols: 9,
            rows: 9,
            owner: None,
        };
        feeds.publish(&id, || g);
        assert_eq!(next_grid(&mut rx).await, Some(g));
        feeds.remove(&id);
        assert_eq!(next_grid(&mut rx).await, None, "sender gone → feed over");
    }
}
