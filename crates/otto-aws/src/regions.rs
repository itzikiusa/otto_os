//! "All enabled regions" fan-out for the regional list views (EC2 / EKS /
//! RDS). `?region=all` resolves the account's enabled regions once
//! (`ec2 describe-regions`, cached per account for an hour — the default
//! answer already omits opt-in regions the account has not enabled), then runs
//! the per-region call with at most [`FANOUT_CONCURRENCY`] CLI children at a
//! time. A region that fails never fails the whole list: it comes back in
//! `region_errors` so the UI can show it inline next to the rows that worked.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use otto_core::{Error, Id, Result};
use otto_state::AwsAccountRow;
use serde::Serialize;
use tokio::sync::Semaphore;

use crate::accounts::AwsService;

/// The `?region=` value that asks for every enabled region.
pub const ALL: &str = "all";
/// Parallel CLI children per fan-out (each is a Python process).
pub const FANOUT_CONCURRENCY: usize = 6;
const REGIONS_TTL: Duration = Duration::from_secs(3600);

/// One region that failed during a fan-out.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RegionError {
    pub region: String,
    pub message: String,
}

pub fn is_all(region: Option<&str>) -> bool {
    region.map(str::trim) == Some(ALL)
}

fn cache() -> &'static Mutex<HashMap<Id, (Instant, Vec<String>)>> {
    static C: OnceLock<Mutex<HashMap<Id, (Instant, Vec<String>)>>> = OnceLock::new();
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
    let sem = Arc::new(Semaphore::new(FANOUT_CONCURRENCY));
    let mut handles = Vec::with_capacity(regions.len());
    for region in regions {
        let sem = sem.clone();
        let fut = call(region.clone());
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await;
            (region, fut.await)
        }));
    }
    let mut ok = Vec::new();
    let mut errs = Vec::new();
    let mut login: Option<Error> = None;
    let total = handles.len();
    for h in handles {
        match h.await {
            Ok((region, Ok(v))) => ok.push((region, v)),
            Ok((region, Err(e))) => {
                let message = message_of(&e);
                if message.starts_with("login required") && login.is_none() {
                    login = Some(e);
                }
                errs.push(RegionError { region, message });
            }
            Err(join) => errs.push(RegionError {
                region: "?".into(),
                message: format!("region task failed: {join}"),
            }),
        }
    }
    if total > 0 && ok.is_empty() {
        if let Some(e) = login {
            return Err(e);
        }
    }
    Ok((ok, errs))
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
    async fn all_regions_login_required_surfaces_the_sign_in_error() {
        let r = fan_out(vec!["a".into(), "b".into()], |_| async {
            Err::<(), _>(Error::Invalid("login required: token expired".into()))
        })
        .await;
        assert!(matches!(r, Err(Error::Invalid(m)) if m.starts_with("login required")));
    }
}
