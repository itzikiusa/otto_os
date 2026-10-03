//! "All enabled regions" fan-out for the regional list views (EC2 / EKS /
//! RDS). `?region=all` resolves the account's enabled regions once
//! (`ec2 describe-regions`, cached per account for an hour — the default
//! answer already omits opt-in regions the account has not enabled), then runs
//! the per-region call with at most [`FANOUT_CONCURRENCY`] CLI children at a
//! time. A region that fails never fails the whole list: it comes back in
//! `region_errors` so the UI can show it inline next to the rows that worked.
//!
//! The fan-out runs on the request's own task (`buffered`, no `tokio::spawn`),
//! so a client that navigates away drops the future and `kill_on_drop` reaps
//! every in-flight child. Whole all-regions answers are cached for
//! [`ALL_REGIONS_TTL`] with in-flight dedupe ([`cached_all`]): auto-refresh
//! from two tabs, or a refresh racing the previous one, costs one fan-out.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use otto_core::{Error, Id, Result};
use otto_state::AwsAccountRow;
use serde::Serialize;

use crate::accounts::AwsService;

/// The `?region=` value that asks for every enabled region.
pub const ALL: &str = "all";
/// Parallel CLI children per fan-out (each is a Python process).
pub const FANOUT_CONCURRENCY: usize = 6;
const REGIONS_TTL: Duration = Duration::from_secs(3600);
/// How long a whole `?region=all` answer is reused (≈17 CLI children each).
pub const ALL_REGIONS_TTL: Duration = Duration::from_secs(20);

/// One region that failed during a fan-out.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RegionError {
    pub region: String,
    pub message: String,
}

pub fn is_all(region: Option<&str>) -> bool {
    region.map(str::trim) == Some(ALL)
}

/// account id → (fetched at, enabled regions).
type RegionCache = Mutex<HashMap<Id, (Instant, Vec<String>)>>;

fn cache() -> &'static RegionCache {
    static C: OnceLock<RegionCache> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// `ec2 describe-regions` → region codes, sorted.
pub fn parse_regions(v: &serde_json::Value) -> Vec<String> {
    let mut out: Vec<String> = v
        .get("Regions")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|r| r.get("RegionName").and_then(|n| n.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out.dedup();
    out
}

/// The account's enabled regions (cached 1 h). Falls back to the account's own
/// region when `describe-regions` is denied, so "All regions" degrades to the
/// default view instead of an error page.
pub async fn enabled_regions(svc: &AwsService, a: &AwsAccountRow) -> Result<Vec<String>> {
    {
        let map = cache().lock().unwrap_or_else(|p| p.into_inner());
        if let Some((at, list)) = map.get(&a.id) {
            if at.elapsed() < REGIONS_TTL {
                return Ok(list.clone());
            }
        }
    }
    let list = match svc.run_json(a, None, &["ec2", "describe-regions"]).await {
        Ok(v) => parse_regions(&v),
        // Credentials are a hard stop (the UI shows "Sign in"); anything else
        // (e.g. no ec2:DescribeRegions) degrades to the home region.
        Err(Error::Invalid(m)) if m.starts_with("login required") => return Err(Error::Invalid(m)),
        Err(e) => {
            tracing::debug!("describe-regions failed, using the account region: {e}");
            Vec::new()
        }
    };
    let list = if list.is_empty() {
        vec![a.region.clone()]
    } else {
        list
    };
    cache()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(a.id.clone(), (Instant::now(), list.clone()));
    Ok(list)
}

/// Human message for a per-region failure (`Error`'s Display carries a code
/// prefix the inline row does not need).
fn message_of(e: &Error) -> String {
    match e {
        Error::Invalid(m) | Error::Forbidden(m) | Error::Upstream(m) | Error::Internal(m) => {
            m.clone()
        }
        other => other.to_string(),
    }
}

/// Run `call(region)` for every region with bounded concurrency. Results keep
/// the input region order. If EVERY region failed with `login required`, that
/// error is returned as-is so the UI shows its Sign-in path.
pub async fn fan_out<T, F, Fut>(
    regions: Vec<String>,
    call: F,
) -> Result<(Vec<(String, T)>, Vec<RegionError>)>
where
    T: Send + 'static,
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<T>> + Send + 'static,
{
    let total = regions.len();
    // `buffered` (not `tokio::spawn`): the calls live inside this future, so
    // dropping the request cancels them — and kills their children.
    let results: Vec<(String, Result<T>)> = futures_util::stream::iter(regions)
        .map(|region| {
            let fut = call(region.clone());
            // Fan-out children draw from the background share of the CLI cap,
            // so a click elsewhere never queues behind 6 regions (N1).
            async move { (region, crate::cli::background(fut).await) }
        })
        .buffered(FANOUT_CONCURRENCY)
        .collect()
        .await;
    let mut ok = Vec::new();
    let mut errs = Vec::new();
    let mut login: Option<Error> = None;
    for (region, r) in results {
        match r {
            Ok(v) => ok.push((region, v)),
            Err(e) => {
                let message = message_of(&e);
                if message.starts_with("login required") && login.is_none() {
                    login = Some(e);
                }
                errs.push(RegionError { region, message });
            }
        }
    }
    if total > 0 && ok.is_empty() {
        if let Some(e) = login {
            return Err(e);
        }
    }
    Ok((ok, errs))
}

/// cache key → (computed at, answer).
type AllCache = Mutex<HashMap<String, (Instant, serde_json::Value)>>;
/// cache key → in-flight lock (single-flight per key).
type AllLocks = Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>;

fn all_cache() -> &'static AllCache {
    static C: OnceLock<AllCache> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

fn all_locks() -> &'static AllLocks {
    static L: OnceLock<AllLocks> = OnceLock::new();
    L.get_or_init(|| Mutex::new(HashMap::new()))
}

fn all_cache_get(key: &str) -> Option<serde_json::Value> {
    let map = all_cache().lock().unwrap_or_else(|p| p.into_inner());
    map.get(key)
        .filter(|(at, _)| at.elapsed() < ALL_REGIONS_TTL)
        .map(|(_, v)| v.clone())
}

/// The cache key for one all-regions view: account + service + the
/// normalized query (filters change the answer).
pub fn all_key(account: &Id, service: &str, query: &impl std::fmt::Debug) -> String {
    format!("{account}|{service}|{query:?}")
}

/// Serve a `?region=all` answer from the [`ALL_REGIONS_TTL`] cache, or compute
/// it once — concurrent callers for the same key wait for the first one
/// instead of starting their own fan-out. Errors are never cached.
pub async fn cached_all<F, Fut>(key: String, compute: F) -> Result<serde_json::Value>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<serde_json::Value>>,
{
    if let Some(v) = all_cache_get(&key) {
        return Ok(v);
    }
    let lock = {
        let mut locks = all_locks().lock().unwrap_or_else(|p| p.into_inner());
        // Keep the lock map bounded: drop entries nobody holds.
        locks.retain(|_, l| Arc::strong_count(l) > 1);
        locks.entry(key.clone()).or_default().clone()
    };
    let _g = lock.lock().await;
    if let Some(v) = all_cache_get(&key) {
        return Ok(v);
    }
    let v = compute().await?;
    let mut map = all_cache().lock().unwrap_or_else(|p| p.into_inner());
    map.retain(|_, (at, _)| at.elapsed() < ALL_REGIONS_TTL);
    map.insert(key, (Instant::now(), v.clone()));
    Ok(v)
}

/// Forget every cached all-regions answer for `account` (after a state change
/// such as an EC2 start/stop, so the next refresh shows it).
pub fn invalidate(account: &Id) {
    let prefix = format!("{account}|");
    all_cache()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .retain(|k, _| !k.starts_with(&prefix));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn parses_and_sorts_regions() {
        let v = serde_json::json!({"Regions":[{"RegionName":"us-east-1"},{"RegionName":"eu-west-1"},{"Endpoint":"x"}]});
        assert_eq!(parse_regions(&v), vec!["eu-west-1", "us-east-1"]);
        assert!(is_all(Some("all")));
        assert!(!is_all(Some("eu-west-1")));
        assert!(!is_all(None));
    }

    #[tokio::test]
    async fn fan_out_is_bounded_and_keeps_partial_failures() {
        let live = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let regions: Vec<String> = (0..20).map(|i| format!("r-{i}")).collect();
        let (ok, errs) = fan_out(regions, |r| {
            let live = live.clone();
            let peak = peak.clone();
            async move {
                let now = live.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(5)).await;
                live.fetch_sub(1, Ordering::SeqCst);
                if r == "r-3" {
                    Err(Error::Forbidden("not authorized in r-3".into()))
                } else {
                    Ok(r.len())
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(ok.len(), 19);
        assert_eq!(ok[0].0, "r-0", "input order is kept");
        assert_eq!(
            errs,
            vec![RegionError {
                region: "r-3".into(),
                message: "not authorized in r-3".into()
            }]
        );
        assert!(peak.load(Ordering::SeqCst) <= FANOUT_CONCURRENCY);
    }

    #[tokio::test]
    async fn dropping_a_fan_out_cancels_in_flight_calls() {
        // Counts calls that started but never finished: with spawned tasks
        // they would keep running after the request is gone.
        struct Guard(Arc<AtomicUsize>);
        impl Drop for Guard {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let started = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicUsize::new(0));
        let finished = Arc::new(AtomicUsize::new(0));
        let regions: Vec<String> = (0..17).map(|i| format!("r-{i}")).collect();
        let fut = fan_out(regions, |_| {
            let (started, dropped, finished) = (started.clone(), dropped.clone(), finished.clone());
            async move {
                let _g = Guard(dropped);
                started.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_secs(30)).await;
                finished.fetch_add(1, Ordering::SeqCst);
                Ok::<_, Error>(())
            }
        });
        let _ = tokio::time::timeout(Duration::from_millis(50), fut).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(started.load(Ordering::SeqCst), FANOUT_CONCURRENCY);
        assert_eq!(
            dropped.load(Ordering::SeqCst),
            FANOUT_CONCURRENCY,
            "in-flight calls dropped"
        );
        assert_eq!(finished.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn cached_all_dedupes_concurrent_and_repeat_calls() {
        let computes = Arc::new(AtomicUsize::new(0));
        let key = all_key(&"acct-cache-test".to_string(), "ec2", &("q", 1));
        let run = || {
            let computes = computes.clone();
            cached_all(key.clone(), move || async move {
                computes.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(30)).await;
                Ok(serde_json::json!({"instances": []}))
            })
        };
        let (a, b, c) = tokio::join!(run(), run(), run());
        assert!(a.is_ok() && b.is_ok() && c.is_ok());
        assert_eq!(computes.load(Ordering::SeqCst), 1, "in-flight dedupe");
        run().await.unwrap();
        assert_eq!(
            computes.load(Ordering::SeqCst),
            1,
            "repeat within TTL is cached"
        );
        invalidate(&"acct-cache-test".to_string());
        run().await.unwrap();
        assert_eq!(
            computes.load(Ordering::SeqCst),
            2,
            "invalidate forces a refetch"
        );
        // Errors are not cached.
        let k2 = all_key(&"acct-cache-err".to_string(), "ec2", &());
        let e = cached_all(k2.clone(), || async { Err(Error::Upstream("boom".into())) }).await;
        assert!(e.is_err());
        let ok = cached_all(k2, || async { Ok(serde_json::json!(1)) })
            .await
            .unwrap();
        assert_eq!(ok, 1);
    }

    #[tokio::test]
    async fn all_regions_login_required_surfaces_the_sign_in_error() {
        let r = fan_out(vec!["a".into(), "b".into()], |_| async {
            Err::<(), _>(Error::Invalid("login required: token expired".into()))
        })
        .await;
        assert!(matches!(r, Err(Error::Invalid(m)) if m.starts_with("login required")));
    }
}
