//! Dashboard read cache + single-flight.
//!
//! Every dashboard read is a function of its query parameters and of what
//! the collectors have written since. Two flavours:
//!
//! - **cycle-keyed** ([`get`] / [`put`]): per-cluster reads stay valid exactly
//!   until that cluster's next cycle (`last_cycle_at` changes) — no TTL
//!   guessing;
//! - **generation-keyed** ([`get_fleet`] / [`put_fleet`]): cross-cluster
//!   (fleet) reads are valid for [`FLEET_FLOOR`] no matter what, then until
//!   any collector writes again ([`bump_generation`]).
//!
//! - **closed-span** ([`get_closed`] / [`put_closed`]): the answer to a query
//!   over a time span that ended before the current tier bucket can no
//!   longer change; it is keyed by its SQL (which carries the snapped
//!   bounds) and kept until the next bucket boundary — so a 24 h baseline is
//!   computed once an hour, not on every 60 s cycle.
//!
//! [`flight`] serialises identical computations: N viewers (Home box, a
//! second window, a reconnecting socket) asking the same question at once
//! cost ONE set of ClickHouse queries — the rest wait and read the cache.
//! Both maps are bounded by the number of distinct views actually opened.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;

struct Entry {
    stamp: String,
    at: Instant,
    value: Value,
}

static CACHE: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
static FLIGHTS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Safety net when a collector stops writing (disabled, unreachable).
const MAX_AGE: Duration = Duration::from_secs(10 * 60);
/// A fleet answer is reused this long even while collectors keep writing —
/// with N clusters some cycle lands every few seconds, and a cross-cluster
/// window of ≥ 1 h does not move in 15 s.
pub const FLEET_FLOOR: Duration = Duration::from_secs(15);
const CAP: usize = 512;

fn map() -> &'static Mutex<HashMap<String, Entry>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A collector wrote samples / events.
pub fn bump_generation() {
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

pub fn get(key: &str, cycle: &str) -> Option<Value> {
    let m = map().lock().ok()?;
    let e = m.get(key)?;
    (e.stamp == cycle && e.at.elapsed() < MAX_AGE).then(|| e.value.clone())
}

pub fn put(key: String, cycle: &str, value: &Value) {
    if let Ok(mut m) = map().lock() {
        if m.len() > CAP {
            m.clear();
        }
        m.insert(
            key,
            Entry {
                stamp: cycle.to_string(),
                at: Instant::now(),
                value: value.clone(),
            },
        );
    }
}

/// Fleet entry: fresh within [`FLEET_FLOOR`], else while no collector wrote.
pub fn get_fleet(key: &str) -> Option<Value> {
    let m = map().lock().ok()?;
    let e = m.get(key)?;
    let age = e.at.elapsed();
    let same_gen = e.stamp == generation().to_string();
    (age < FLEET_FLOOR || (same_gen && age < MAX_AGE)).then(|| e.value.clone())
}

/// Store a fleet answer stamped with the generation read BEFORE computing it
/// (`gen`), so a write that lands mid-computation invalidates it.
pub fn put_fleet(key: String, gen: u64, value: &Value) {
    put(key, &gen.to_string(), value);
}

struct Closed {
    until: Instant,
    value: Value,
}

static CLOSED: OnceLock<Mutex<HashMap<String, Closed>>> = OnceLock::new();

fn closed_map() -> &'static Mutex<HashMap<String, Closed>> {
    CLOSED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A closed span's cached answer, while still inside its validity.
pub fn get_closed(sql: &str) -> Option<Value> {
    let m = closed_map().lock().ok()?;
    let e = m.get(sql)?;
    (Instant::now() < e.until).then(|| e.value.clone())
}

/// Cache a closed span's answer for `ttl` (until the next bucket boundary).
pub fn put_closed(sql: String, value: &Value, ttl: Duration) {
    if let Ok(mut m) = closed_map().lock() {
        if m.len() > CAP {
            let now = Instant::now();
            m.retain(|_, e| e.until > now);
            if m.len() > CAP {
                m.clear();
            }
        }
        m.insert(
            sql,
            Closed {
                until: Instant::now() + ttl.min(MAX_AGE * 6),
                value: value.clone(),
            },
        );
    }
}

/// Wait for (then hold) the single flight for `key`. Re-check the cache
/// after this returns: the flight you waited on has usually filled it.
pub async fn flight(key: &str) -> tokio::sync::OwnedMutexGuard<()> {
    let lock = {
        let mut m = FLIGHTS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if m.len() > CAP {
            // Drop idle flights (nobody holds or waits on them).
            m.retain(|_, l| Arc::strong_count(l) > 1);
        }
        m.entry(key.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };
    lock.lock_owned().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_entries_invalidate_on_a_new_cycle() {
        put("t:cycle".into(), "c1", &Value::from(1));
        assert_eq!(get("t:cycle", "c1"), Some(Value::from(1)));
        assert_eq!(get("t:cycle", "c2"), None);
    }

    #[test]
    fn fleet_entries_survive_the_floor_then_follow_the_generation() {
        let gen = generation();
        put_fleet("t:fleet".into(), gen, &Value::from(2));
        bump_generation();
        // Within the floor a write does not invalidate.
        assert_eq!(get_fleet("t:fleet"), Some(Value::from(2)));
        // An entry older than the floor with a stale generation is gone.
        if let Ok(mut m) = map().lock() {
            if let Some(e) = m.get_mut("t:fleet") {
                e.at = Instant::now() - FLEET_FLOOR - Duration::from_secs(1);
            }
        }
        assert_eq!(get_fleet("t:fleet"), None);
        put_fleet("t:fleet".into(), generation(), &Value::from(3));
        if let Ok(mut m) = map().lock() {
            if let Some(e) = m.get_mut("t:fleet") {
                e.at = Instant::now() - FLEET_FLOOR - Duration::from_secs(1);
            }
        }
        assert_eq!(get_fleet("t:fleet"), Some(Value::from(3)));
    }

    #[test]
    fn closed_entries_expire_at_their_boundary() {
        put_closed("SELECT 1".into(), &Value::from(4), Duration::from_secs(60));
        assert_eq!(get_closed("SELECT 1"), Some(Value::from(4)));
        assert_eq!(get_closed("SELECT 2"), None);
        put_closed("SELECT 3".into(), &Value::from(5), Duration::ZERO);
        assert_eq!(get_closed("SELECT 3"), None, "past its boundary");
    }

    #[tokio::test]
    async fn flight_serialises_identical_keys() {
        let g = flight("t:flight").await;
        let waiter = tokio::spawn(async {
            let _g = flight("t:flight").await;
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiter.is_finished(), "second caller waits for the first");
        drop(g);
        waiter.await.unwrap();
        // Different keys never wait on each other.
        let _a = flight("t:a").await;
        let _b = flight("t:b").await;
    }
}
