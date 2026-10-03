//! Process-wide dedupe for `GET /repos/{id}/status`: one `git status` walk per
//! repo no matter how many windows ask, plus a 1 s memo of the SERIALIZED
//! response.
//!
//! Every Otto window re-reads status on each `repo_status_changed`; with N
//! windows open that was N full working-tree walks per saved file. Now
//! concurrent requests share one spawn ([`SingleFlight`], abort-safe: when
//! every waiter is gone the git child is killed), and a request arriving just
//! after reuses the bytes.
//!
//! Staleness is bounded by a per-repo GENERATION, not just the clock: the
//! watcher bumps it right before it emits `RepoStatusChanged`, and every
//! mutating `/repos/{id}/…` route bumps it once it has run (a router layer
//! in `http.rs`). A memo
//! entry is served only while its generation is current and it is younger
//! than [`MEMO_TTL`]; a walk that started before a bump stores under the old
//! generation and is never served after it.
//!
//! The same table records how long each repo's last status took, which the
//! watcher reads to space its events ([`crate::watch`]: a 2 s status must not
//! be re-run every 400 ms).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use otto_core::{Error, Result};

use crate::diff_cache::SingleFlight;
use crate::local::LocalGit;

/// How long a status body may be reused while no change was signalled.
pub(crate) const MEMO_TTL: Duration = Duration::from_secs(1);
/// Repos tracked at once; past this, idle slots are dropped.
const MAX_SLOTS: usize = 256;

#[derive(Default)]
struct Slot {
    gen: u64,
    memo: Option<(u64, Instant, Bytes)>,
    last_status: Option<Duration>,
    touched: Option<Instant>,
}

fn slots() -> &'static Mutex<HashMap<String, Slot>> {
    static S: OnceLock<Mutex<HashMap<String, Slot>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

fn with_slot<R>(key: &str, f: impl FnOnce(&mut Slot) -> R) -> R {
    let mut m = slots().lock().unwrap_or_else(|p| p.into_inner());
    if m.len() > MAX_SLOTS && !m.contains_key(key) {
        // Dropping a slot drops its memo with it, so a reset generation can
        // never serve a stale body.
        m.retain(|_, s| {
            s.touched
                .is_some_and(|t| t.elapsed() < Duration::from_secs(600))
        });
    }
    let slot = m.entry(key.to_string()).or_default();
    slot.touched = Some(Instant::now());
    f(slot)
}

fn flights() -> &'static SingleFlight<Bytes> {
    static F: OnceLock<SingleFlight<Bytes>> = OnceLock::new();
    F.get_or_init(SingleFlight::new)
}

/// Invalidate `key`'s memo: something changed (a watcher event, a mutation).
pub(crate) fn bump(key: &str) {
    with_slot(key, |s| {
        s.gen = s.gen.wrapping_add(1);
        s.memo = None;
    });
}

/// How long the repo's last full status took (`None` before the first).
pub(crate) fn last_status(key: &str) -> Option<Duration> {
    slots()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(key)
        .and_then(|s| s.last_status)
}

pub(crate) fn record_status_time(key: &str, took: Duration) {
    with_slot(key, |s| s.last_status = Some(took));
}

/// The serialized status of `git`'s repo, shared/memoized as described in the
/// module docs. Returns the body and whether it was a memo `hit`, a joined or
/// fresh walk (`miss`).
/// Keyed by repo id (what the watcher and the write layer know).
pub(crate) async fn status_body(repo_id: &str, git: &LocalGit) -> Result<(Bytes, &'static str)> {
    let key = repo_id.to_string();
    let gen = match with_slot(&key, |s| match &s.memo {
        Some((g, at, body)) if *g == s.gen && at.elapsed() < MEMO_TTL => Err(body.clone()),
        _ => Ok(s.gen),
    }) {
        Err(hit) => return Ok((hit, "hit")),
        Ok(g) => g,
    };
    let git = git.clone();
    let flight_key = format!("{key}\0{gen}");
    let body = flights()
        .run(&flight_key, move || async move {
            let t0 = Instant::now();
            let st = git.status().await?;
            record_status_time(&key, t0.elapsed());
            let est = st.changes.len() * 96;
            let body = crate::local::off_runtime(est, move || serde_json::to_vec(&st))
                .await?
                .map(Bytes::from)
                .map_err(|e| Error::Internal(format!("status json: {e}")))?;
            with_slot(&key, |s| {
                if s.gen == gen {
                    s.memo = Some((gen, Instant::now(), body.clone()));
                }
            });
            Ok(body)
        })
        .await?;
    Ok((body, "miss"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> (tempfile::TempDir, LocalGit) {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .unwrap()
                .status
                .success());
        };
        run(&["init", "-q", "-b", "main"]);
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        let git = LocalGit::new(dir.path());
        (dir, git)
    }

    const ID: &str = "status-cache-test-a";

    #[tokio::test]
    async fn memo_serves_until_bumped_and_concurrent_reads_share_one_walk() {
        let (dir, git) = repo();
        let (a, ka) = status_body(ID, &git).await.unwrap();
        let (b, kb) = status_body(ID, &git).await.unwrap();
        assert_eq!((ka, kb), ("miss", "hit"));
        assert_eq!(a, b);
        assert!(last_status(ID).is_some());

        // A change the memo can't see until something signals it…
        std::fs::write(dir.path().join("b.txt"), "b").unwrap();
        bump(ID);
        let (c, kc) = status_body(ID, &git).await.unwrap();
        assert_eq!(kc, "miss");
        assert!(String::from_utf8_lossy(&c).contains("b.txt"));

        // N concurrent requests after a bump: one shared walk, same bytes.
        bump(ID);
        let reads = (0..6).map(|_| status_body(ID, &git));
        let out = futures_util::future::join_all(reads).await;
        let bodies: Vec<Bytes> = out.into_iter().map(|r| r.unwrap().0).collect();
        assert!(bodies.windows(2).all(|w| w[0] == w[1]));
    }

    #[tokio::test]
    async fn memo_expires_after_ttl() {
        const ID: &str = "status-cache-test-b";
        let (_dir, git) = repo();
        status_body(ID, &git).await.unwrap();
        with_slot(ID, |s| {
            if let Some(m) = s.memo.as_mut() {
                m.1 = Instant::now() - MEMO_TTL - Duration::from_millis(1);
            }
        });
        assert_eq!(status_body(ID, &git).await.unwrap().1, "miss");
    }
}
