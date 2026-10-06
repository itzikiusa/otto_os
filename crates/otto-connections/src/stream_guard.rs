//! Re-authorization of long-lived streamed response bodies (followed pod
//! logs, S3 downloads): the grant is rechecked while the body streams —
//! including when no data arrives — and the stream ends the moment it is
//! revoked. Shared by `otto-k8s` and `otto-aws` (S6-310: the two crates used
//! to carry identical copies).

use axum::body::Body;
use futures_util::StreamExt;
use std::future::Future;
use std::time::{Duration, Instant};

/// Longest a streamed body runs on a stale authorization (see [`guard_body`]).
pub const GUARD_RECHECK: Duration = Duration::from_secs(1);

/// True when a streamed body's grant is due for a recheck.
pub fn recheck_due(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|t| now.saturating_duration_since(t) >= GUARD_RECHECK)
}

/// Wrap `body` so `authorized()` is re-evaluated at most once per
/// [`GUARD_RECHECK`] (S6-08: a check is several SQLite queries, and a fast
/// stream otherwise ran one per chunk) and at least once per idle second; the
/// first chunk always checks. `false` ends the stream.
pub fn guard_body<F, Fut>(body: Body, authorized: F) -> Body
where
    F: FnMut() -> Fut + Send + 'static,
    Fut: Future<Output = bool> + Send,
{
    let stream = futures_util::stream::unfold(
        (body.into_data_stream(), authorized, None::<Instant>),
        |(mut stream, mut authorized, mut last_checked)| async move {
            loop {
                if recheck_due(last_checked, Instant::now()) {
                    if !authorized().await {
                        return None;
                    }
                    last_checked = Some(Instant::now());
                }
                tokio::select! {
                    data = stream.next() => return data.map(|data| (data, (stream, authorized, last_checked))),
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {},
                }
            }
        },
    );
    Body::from_stream(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// S6-08: a streamed body re-authorizes once per `GUARD_RECHECK`, not
    /// once per chunk — the first chunk always checks.
    #[test]
    fn recheck_runs_at_most_once_per_interval() {
        let t0 = Instant::now();
        assert!(recheck_due(None, t0));
        assert!(!recheck_due(Some(t0), t0));
        assert!(!recheck_due(Some(t0), t0 + Duration::from_millis(999)));
        assert!(recheck_due(Some(t0), t0 + GUARD_RECHECK));
        // 1,000 chunks inside one second cost exactly one check.
        let mut last = None;
        let mut checks = 0;
        for i in 0..1000u64 {
            let now = t0 + Duration::from_micros(i * 900);
            if recheck_due(last, now) {
                checks += 1;
                last = Some(now);
            }
        }
        assert_eq!(checks, 1);
    }

    /// A revoked grant ends the stream before any byte is served; a granted
    /// one streams the whole body with one check.
    #[tokio::test]
    async fn guarded_body_ends_when_the_grant_is_revoked() {
        use http_body_util::BodyExt;
        let denied = guard_body(Body::from("secret"), || async { false });
        let bytes = denied.collect().await.unwrap().to_bytes();
        assert!(bytes.is_empty());

        let checks = Arc::new(AtomicUsize::new(0));
        let c = checks.clone();
        let allowed = guard_body(Body::from("payload"), move || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                true
            }
        });
        let bytes = allowed.collect().await.unwrap().to_bytes();
        assert_eq!(&bytes[..], b"payload");
        assert_eq!(checks.load(Ordering::SeqCst), 1);
    }
}
