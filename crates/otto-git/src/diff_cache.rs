//! Process-wide memo for diff responses: a bytes-bounded LRU of SERIALIZED
//! immutable diffs (a hit costs no git, no parse, no serde) and a single-flight
//! map so N identical concurrent requests share one computation.
//!
//! Only content-addressed work is cached: keys carry full commit ids (a
//! `commit:` target or both ends of a range, resolved first), never a branch
//! name or the worktree/index. PR diffs from a provider are not
//! content-addressed, so they live in [`PR_DIFFS`] with a short TTL instead.
//!
//! Cancellation: the in-flight map holds only a WEAK handle. When every
//! request awaiting a computation is dropped (client aborted), the shared
//! future is dropped with them — and the `git` child it was waiting on is
//! killed (`kill_on_drop`, `SpawnClass::LocalRead`).

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use futures_util::future::{BoxFuture, FutureExt, Shared, WeakShared};
use otto_core::api::DiffResp;
use otto_core::{Error, Result};

/// A bytes-bounded least-recently-used map. Eviction scans for the oldest
/// tick — O(entries), and entries stay in the low hundreds.
pub(crate) struct ByteLru<V> {
    max_bytes: usize,
    max_entry: usize,
    max_entries: usize,
    ttl: Option<Duration>,
    total: usize,
    tick: u64,
    entries: HashMap<String, Slot<V>>,
}

struct Slot<V> {
    value: V,
    size: usize,
    used: u64,
    at: Instant,
}

impl<V: Clone> ByteLru<V> {
    pub(crate) fn new(
        max_bytes: usize,
        max_entry: usize,
        max_entries: usize,
        ttl: Option<Duration>,
    ) -> Self {
        Self {
            max_bytes,
            max_entry,
            max_entries,
            ttl,
            total: 0,
            tick: 0,
            entries: HashMap::new(),
        }
    }

    pub(crate) fn get(&mut self, key: &str) -> Option<V> {
        self.tick += 1;
        let tick = self.tick;
        let expired = match (self.entries.get(key), self.ttl) {
            (Some(s), Some(ttl)) => s.at.elapsed() > ttl,
            (Some(_), None) => false,
            (None, _) => return None,
        };
        if expired {
            self.remove(key);
            return None;
        }
        let slot = self.entries.get_mut(key)?;
        slot.used = tick;
        Some(slot.value.clone())
    }

    /// Insert `value` weighing `size` bytes; a value above the per-entry
    /// ceiling is not cached at all.
    pub(crate) fn put(&mut self, key: String, value: V, size: usize) {
        if size > self.max_entry {
            return;
        }
        self.remove(&key);
        while !self.entries.is_empty()
            && (self.total + size > self.max_bytes || self.entries.len() >= self.max_entries)
        {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, s)| s.used)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(k) => self.remove(&k),
                None => break,
            }
        }
        self.tick += 1;
        self.total += size;
        self.entries.insert(
            key,
            Slot {
                value,
                size,
                used: self.tick,
                at: Instant::now(),
            },
        );
    }

    fn remove(&mut self, key: &str) {
        if let Some(s) = self.entries.remove(key) {
            self.total -= s.size;
        }
    }

    #[cfg(test)]
    pub(crate) fn total_bytes(&self) -> usize {
        self.total
    }
}

/// Serialized immutable `/diff` responses: 64 MB total, ≤ 16 MB each.
fn diff_bodies() -> &'static Mutex<ByteLru<Bytes>> {
    static C: OnceLock<Mutex<ByteLru<Bytes>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(ByteLru::new(64 << 20, 16 << 20, 512, None)))
}

pub(crate) fn get_body(key: &str) -> Option<Bytes> {
    diff_bodies()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(key)
}

pub(crate) fn put_body(key: String, body: Bytes) {
    let size = body.len() + key.len();
    diff_bodies()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .put(key, body, size);
}

/// Parsed provider PR diffs, keyed by (repo, PR): the Files tab, the Review
/// tab and every lazy per-file request of one visit share ONE download.
/// Short TTL — a PR's head moves and the provider is the source of truth.
pub(crate) const PR_DIFF_TTL: Duration = Duration::from_secs(60);

pub(crate) fn pr_diffs() -> &'static Mutex<ByteLru<Arc<DiffResp>>> {
    static C: OnceLock<Mutex<ByteLru<Arc<DiffResp>>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(ByteLru::new(96 << 20, 64 << 20, 32, Some(PR_DIFF_TTL))))
}

/// Rough resident size of a parsed diff (content + per-line overhead).
pub(crate) fn approx_size(resp: &DiffResp) -> usize {
    resp.files
        .iter()
        .map(|f| {
            f.path.len()
                + 128
                + f.hunks
                    .iter()
                    .flat_map(|h| h.lines.iter())
                    .map(|l| l.content.len() + 48)
                    .sum::<usize>()
        })
        .sum()
}

type SharedResult<T> = std::result::Result<T, Arc<Error>>;
type Flight<T> = Shared<BoxFuture<'static, SharedResult<T>>>;

/// Coalesces identical concurrent computations. See the module docs for how
/// cancellation flows through the weak handles.
pub(crate) struct SingleFlight<T: Clone + Send + Sync + 'static> {
    inflight: Mutex<HashMap<String, WeakShared<BoxFuture<'static, SharedResult<T>>>>>,
}

impl<T: Clone + Send + Sync + 'static> SingleFlight<T> {
    pub(crate) fn new() -> Self {
        Self {
            inflight: Mutex::new(HashMap::new()),
        }
    }

    /// Await the computation for `key`, starting it with `make` unless an
    /// identical one is already running.
    pub(crate) async fn run<F, Fut>(&self, key: &str, make: F) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T>> + Send + 'static,
    {
        let flight: Flight<T> = {
            let mut m = self.inflight.lock().unwrap_or_else(|p| p.into_inner());
            match m.get(key).and_then(WeakShared::upgrade) {
                Some(f) => f,
                None => {
                    if m.len() > 64 {
                        m.retain(|_, w| w.upgrade().is_some());
                    }
                    let f = make().map(|r| r.map_err(Arc::new)).boxed().shared();
                    if let Some(w) = f.downgrade() {
                        m.insert(key.to_string(), w);
                    }
                    f
                }
            }
        };
        let out = flight.clone().await;
        {
            // Done: forget the flight (a later request hits the cache or
            // starts fresh) — unless a newer one already took the key.
            let mut m = self.inflight.lock().unwrap_or_else(|p| p.into_inner());
            let same = m
                .get(key)
                .and_then(WeakShared::upgrade)
                .is_none_or(|cur| Shared::ptr_eq(&cur, &flight));
            if same {
                m.remove(key);
            }
        }
        out.map_err(|e| clone_error(&e))
    }

    #[cfg(test)]
    pub(crate) fn in_flight(&self) -> usize {
        self.inflight
            .lock()
            .unwrap()
            .values()
            .filter(|w| w.upgrade().is_some())
            .count()
    }
}

pub(crate) fn diff_flights() -> &'static SingleFlight<Bytes> {
    static F: OnceLock<SingleFlight<Bytes>> = OnceLock::new();
    F.get_or_init(SingleFlight::new)
}

pub(crate) fn pr_flights() -> &'static SingleFlight<Arc<DiffResp>> {
    static F: OnceLock<SingleFlight<Arc<DiffResp>>> = OnceLock::new();
    F.get_or_init(SingleFlight::new)
}

/// `otto_core::Error` isn't `Clone`; every waiter of a shared failure gets
/// its own copy with the same variant (→ the same HTTP status).
fn clone_error(e: &Error) -> Error {
    match e {
        Error::NotFound(m) => Error::NotFound(m.clone()),
        Error::Unauthorized => Error::Unauthorized,
        Error::Forbidden(m) => Error::Forbidden(m.clone()),
        Error::Conflict(m) => Error::Conflict(m.clone()),
        Error::Invalid(m) => Error::Invalid(m.clone()),
        Error::PayloadTooLarge(m) => Error::PayloadTooLarge(m.clone()),
        Error::UnsupportedMedia(m) => Error::UnsupportedMedia(m.clone()),
        Error::Upstream(m) => Error::Upstream(m.clone()),
        Error::Internal(m) => Error::Internal(m.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn lru_is_bounded_by_bytes_and_evicts_least_recent() {
        let mut c: ByteLru<u32> = ByteLru::new(100, 60, 10, None);
        c.put("a".into(), 1, 40);
        c.put("b".into(), 2, 40);
        assert_eq!(c.get("a"), Some(1)); // a is now more recent than b
        c.put("c".into(), 3, 40); // 120 > 100 → evict b
        assert_eq!(c.get("b"), None);
        assert_eq!(c.get("a"), Some(1));
        assert_eq!(c.get("c"), Some(3));
        assert_eq!(c.total_bytes(), 80);
        c.put("huge".into(), 4, 61); // over the per-entry ceiling: not cached
        assert_eq!(c.get("huge"), None);
        c.put("a".into(), 5, 10); // replace re-weighs
        assert_eq!(c.total_bytes(), 50);
    }

    #[test]
    fn lru_ttl_expires() {
        let mut c: ByteLru<u32> = ByteLru::new(100, 100, 10, Some(Duration::from_millis(1)));
        c.put("a".into(), 1, 1);
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(c.get("a"), None);
        assert_eq!(c.total_bytes(), 0);
    }

    #[tokio::test]
    async fn single_flight_coalesces_concurrent_identical_work() {
        let sf: Arc<SingleFlight<u32>> = Arc::new(SingleFlight::new());
        let runs = Arc::new(AtomicUsize::new(0));
        let mut hs = Vec::new();
        for _ in 0..8 {
            let (sf, runs) = (sf.clone(), runs.clone());
            hs.push(tokio::spawn(async move {
                sf.run("k", || {
                    let runs = runs.clone();
                    async move {
                        runs.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        Ok(7)
                    }
                })
                .await
            }));
        }
        for h in hs {
            assert_eq!(h.await.unwrap().unwrap(), 7);
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert_eq!(sf.in_flight(), 0);
    }

    #[tokio::test]
    async fn single_flight_drops_work_when_every_waiter_is_gone() {
        let sf: Arc<SingleFlight<u32>> = Arc::new(SingleFlight::new());
        let finished = Arc::new(AtomicUsize::new(0));
        let f2 = finished.clone();
        let h = tokio::spawn({
            let sf = sf.clone();
            async move {
                sf.run("k", || async move {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    f2.fetch_add(1, Ordering::SeqCst);
                    Ok(1)
                })
                .await
            }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        h.abort(); // the only waiter (a client abort)
        let _ = h.await;
        assert_eq!(sf.in_flight(), 0, "the dropped flight must not stay alive");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(finished.load(Ordering::SeqCst), 0, "work was cancelled");
        // A fresh request starts over and completes.
        assert_eq!(sf.run("k", || async { Ok(2) }).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn single_flight_shares_errors_with_their_variant() {
        let sf: SingleFlight<u32> = SingleFlight::new();
        let e = sf
            .run("k", || async { Err(Error::NotFound("gone".into())) })
            .await
            .unwrap_err();
        assert!(matches!(e, Error::NotFound(m) if m == "gone"));
    }
}
