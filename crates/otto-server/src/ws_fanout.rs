//! Serialize-once fan-out for the `/ws/events` sockets (perf SG-09b / D-4).
//!
//! Every events socket used to hold its own receiver on the daemon bus, so
//! tokio cloned the whole [`Event`] (up to a 64 KB `TranscriptAppended` frame)
//! once per socket, and each socket then ran `serde_json::to_string` on its
//! copy. The main window and the side-by-side pane each hold a socket (the
//! tray a filtered third), so the big frames were cloned and serialized 2–3×.
//!
//! Now ONE pump per bus receives each event once and re-broadcasts an
//! `Arc<EventFrame>`: a socket's `recv` clones a pointer, and the JSON text is
//! produced lazily, at most once, by the first socket that is allowed to
//! receive it ([`EventFrame::text`]) — later sockets share the same bytes. An
//! event nobody may receive (or nobody subscribed to) is never serialized.
//!
//! Producers are untouched: the bus stays `broadcast::Sender<Event>`. The pump
//! is found per bus by channel identity (a weak handle, so a test's bus that is
//! dropped also retires its pump), which keeps every `ServerCtx` constructor
//! as it is.

use std::sync::{Arc, Mutex, OnceLock};

use axum::extract::ws::Utf8Bytes;
use otto_core::event::Event;
use tokio::sync::broadcast;

/// Same depth as the daemon bus (ottod `broadcast::channel::<Event>(1024)`).
const FANOUT_CAPACITY: usize = 1024;

/// One bus event, shared by every socket.
pub struct EventFrame {
    pub event: Event,
    text: OnceLock<Option<Utf8Bytes>>,
}

impl EventFrame {
    pub fn new(event: Event) -> Self {
        Self {
            event,
            text: OnceLock::new(),
        }
    }

    /// The wire text, serialized on first use and shared afterwards (the
    /// clone is a refcount bump). `None` if the event does not serialize.
    pub fn text(&self) -> Option<Utf8Bytes> {
        self.text
            .get_or_init(|| serde_json::to_string(&self.event).ok().map(Utf8Bytes::from))
            .clone()
    }

    /// Whether some socket already serialized it (tests).
    pub fn is_serialized(&self) -> bool {
        self.text.get().is_some()
    }
}

/// Bytes of frame text a socket may write back to back before handing its
/// worker back (see [`Pacer`]): about one big frame. Serializing 64 KB of
/// event JSON is ~5 ms in a debug build, well under a millisecond in release.
pub const PACE_BYTES: usize = 64 * 1024;

/// Cooperative pacing for a socket draining a backlog (r3-10-02). A burst of
/// queued events (a flood of trail/status events) is otherwise serialized and
/// written in ONE uninterrupted stretch — `recv` on a ready channel and a
/// writable sink never yield — measured at ~200 ms of blocked worker for
/// 200 × 64 KB in a debug build. Yield once per [`PACE_BYTES`] written.
#[derive(Debug, Default)]
pub struct Pacer {
    since_yield: usize,
}

impl Pacer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `bytes` just written; yields when the stretch reached the pace.
    pub async fn sent(&mut self, bytes: usize) {
        self.since_yield = self.since_yield.saturating_add(bytes);
        if self.since_yield >= PACE_BYTES {
            self.since_yield = 0;
            tokio::task::yield_now().await;
        }
    }
}

/// What a socket receives from the fan-out.
#[derive(Clone)]
pub enum FanItem {
    Event(Arc<EventFrame>),
    /// The pump itself fell behind the bus: `n` events were dropped for
    /// EVERY socket — each should send its client a `resync`.
    Lagged(u64),
}

struct Hub {
    bus: broadcast::WeakSender<Event>,
    tx: broadcast::Sender<FanItem>,
}

fn hubs() -> &'static Mutex<Vec<Hub>> {
    static HUBS: OnceLock<Mutex<Vec<Hub>>> = OnceLock::new();
    HUBS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Subscribe to `bus` through its shared fan-out (started on first use).
/// Must be called inside a tokio runtime (it may spawn the pump).
pub fn subscribe(bus: &broadcast::Sender<Event>) -> broadcast::Receiver<FanItem> {
    let mut hubs = hubs().lock().unwrap_or_else(|e| e.into_inner());
    // Retire hubs whose bus is gone (a test context that was dropped).
    hubs.retain(|h| h.bus.upgrade().is_some());
    if let Some(h) = hubs
        .iter()
        .find(|h| h.bus.upgrade().is_some_and(|b| b.same_channel(bus)))
    {
        return h.tx.subscribe();
    }
    let (tx, rx) = broadcast::channel(FANOUT_CAPACITY);
    tokio::spawn(pump(bus.subscribe(), tx.clone()));
    hubs.push(Hub {
        bus: bus.downgrade(),
        tx,
    });
    rx
}

async fn pump(mut rx: broadcast::Receiver<Event>, tx: broadcast::Sender<FanItem>) {
    loop {
        match rx.recv().await {
            Ok(event) => {
                // No socket listening: nothing to wrap (a send would fail).
                if tx.receiver_count() == 0 {
                    continue;
                }
                let _ = tx.send(FanItem::Event(Arc::new(EventFrame::new(event))));
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!(
                    "events fan-out lagged, skipped {n} events — resyncing every socket"
                );
                let _ = tx.send(FanItem::Lagged(n));
            }
            Err(broadcast::error::RecvError::Closed) => {
                // The bus is gone: drop our hub so the sockets see `Closed`.
                let mut hubs = hubs().lock().unwrap_or_else(|e| e.into_inner());
                hubs.retain(|h| !h.tx.same_channel(&tx));
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(title: &str) -> Event {
        Event::Notice {
            level: "info".into(),
            title: title.into(),
            body: "b".into(),
        }
    }

    async fn next_frame(rx: &mut broadcast::Receiver<FanItem>) -> Arc<EventFrame> {
        match tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("delivered")
            .expect("open")
        {
            FanItem::Event(f) => f,
            FanItem::Lagged(n) => panic!("unexpected lag {n}"),
        }
    }

    #[tokio::test]
    async fn every_socket_shares_one_frame_and_one_serialization() {
        let (bus, _keep) = broadcast::channel::<Event>(16);
        let mut a = subscribe(&bus);
        let mut b = subscribe(&bus);
        let ev = notice("hello");
        let want = serde_json::to_string(&ev).unwrap();
        bus.send(ev).unwrap();
        let fa = next_frame(&mut a).await;
        let fb = next_frame(&mut b).await;
        assert!(
            Arc::ptr_eq(&fa, &fb),
            "one Arc'd frame, not a clone per socket"
        );
        assert!(
            !fa.is_serialized(),
            "nothing serialized until a socket needs it"
        );
        let ta = fa.text().unwrap();
        let tb = fb.text().unwrap();
        assert_eq!(ta.as_str(), want, "same bytes as a direct serialization");
        assert_eq!(
            ta.as_str().as_ptr(),
            tb.as_str().as_ptr(),
            "the second socket reuses the bytes"
        );
    }

    #[tokio::test]
    async fn separate_buses_get_separate_pumps_and_a_dropped_bus_closes() {
        let (bus1, _k1) = broadcast::channel::<Event>(16);
        let (bus2, k2) = broadcast::channel::<Event>(16);
        let mut r1 = subscribe(&bus1);
        let mut r2 = subscribe(&bus2);
        bus2.send(notice("two")).unwrap();
        let f = next_frame(&mut r2).await;
        assert!(f.text().unwrap().as_str().contains("\"two\""));
        assert!(
            r1.try_recv().is_err(),
            "bus1's sockets never see bus2's events"
        );
        drop(bus2);
        drop(k2);
        let closed = tokio::time::timeout(std::time::Duration::from_secs(5), r2.recv())
            .await
            .expect("settles");
        assert!(matches!(closed, Err(broadcast::error::RecvError::Closed)));
    }

    #[tokio::test]
    async fn events_with_no_socket_are_not_wrapped() {
        let (bus, _keep) = broadcast::channel::<Event>(16);
        let rx = subscribe(&bus);
        drop(rx);
        bus.send(notice("nobody")).unwrap();
        // Let the pump drain it (no socket → dropped, never wrapped).
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        // A socket that joins later sees only later events.
        let mut late = subscribe(&bus);
        bus.send(notice("later")).unwrap();
        let f = next_frame(&mut late).await;
        assert!(f.text().unwrap().as_str().contains("later"));
    }
}
