//! Exported-credential cache for `profile` accounts (A-5).
//!
//! Every `aws` child used to resolve its profile from scratch — for SSO
//! profiles that means reading the SSO token cache and calling
//! `sso:GetRoleCredentials` on every spawn. Instead the daemon runs
//! `aws configure export-credentials --profile P --format process` once,
//! keeps the result in memory until five minutes before it expires, and hands
//! the keys to each child as `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` /
//! `AWS_SESSION_TOKEN` **alongside** `AWS_PROFILE` (env credentials win over
//! the profile's credential source, while the profile still supplies its other
//! settings — `ca_bundle`, s3 addressing, retries…).
//!
//! The SSO token cache (`~/.aws/sso/cache/*.json`) is also read (never
//! written) so `GET /aws/accounts` can say when the sign-in ends — the UI warns
//! ahead of time instead of failing mid-action.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use otto_core::Id;
use serde::Serialize;

use crate::accounts::StaticCreds;

/// Refresh this long before the reported expiry.
pub const REFRESH_SKEW: Duration = Duration::from_secs(5 * 60);
/// Exported creds without an `Expiration` (long-lived profile keys) are
/// re-exported after this long, so a rotated key is picked up.
pub const NO_EXPIRY_TTL: Duration = Duration::from_secs(15 * 60);
/// After a non-credential export failure (e.g. a CLI older than 2.9 without
/// `configure export-credentials`) skip the export for this long and let each
/// child resolve the profile itself, as before.
pub const FAILURE_BACKOFF: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone)]
struct Entry {
    profile: String,
    creds: StaticCreds,
    expires_at: Option<DateTime<Utc>>,
    fetched: Instant,
}

#[derive(Default)]
struct State {
    ok: HashMap<Id, Entry>,
    failed: HashMap<Id, Instant>,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State::default()))
}

fn lock() -> std::sync::MutexGuard<'static, State> {
    state().lock().unwrap_or_else(|p| p.into_inner())
}

/// Is this cached entry still usable at `now`?
fn fresh(e: &Entry, now: DateTime<Utc>) -> bool {
    match e.expires_at {
        Some(exp) => exp - chrono::Duration::from_std(REFRESH_SKEW).unwrap_or_default() > now,
        None => e.fetched.elapsed() < NO_EXPIRY_TTL,
    }
}

/// Cached creds for `id` (exported for `profile`), if still fresh.
pub fn get(id: &Id, profile: &str) -> Option<StaticCreds> {
    let st = lock();
    st.ok
        .get(id)
        .filter(|e| e.profile == profile && fresh(e, Utc::now()))
        .map(|e| e.creds.clone())
}

pub fn put(id: &Id, profile: &str, creds: StaticCreds, expires_at: Option<DateTime<Utc>>) {
    let mut st = lock();
    st.failed.remove(id);
    st.ok.insert(
        id.clone(),
        Entry {
            profile: profile.to_string(),
            creds,
            expires_at,
            fetched: Instant::now(),
        },
    );
}

/// Forget the account's creds (login required, account edited / deleted).
pub fn evict(id: &Id) {
    let mut st = lock();
    st.ok.remove(id);
    st.failed.remove(id);
}

pub fn mark_failed(id: &Id) {
    lock().failed.insert(id.clone(), Instant::now());
}

pub fn recently_failed(id: &Id) -> bool {
    lock()
        .failed
        .get(id)
        .is_some_and(|at| at.elapsed() < FAILURE_BACKOFF)
}

/// Expiry of the account's cached temporary credentials, if any.
pub fn expires_at(id: &Id) -> Option<DateTime<Utc>> {
    lock().ok.get(id).and_then(|e| e.expires_at)
}

/// `configure export-credentials --format process` JSON → creds + expiry.
pub fn parse_exported(v: &serde_json::Value) -> Option<(StaticCreds, Option<DateTime<Utc>>)> {
    let s = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let creds = StaticCreds {
        access_key_id: s("AccessKeyId")?,
        secret_access_key: s("SecretAccessKey")?,
        session_token: s("SessionToken"),
    };
    let exp = s("Expiration").and_then(|e| {
        DateTime::parse_from_rfc3339(&e)
            .ok()
            .map(|d| d.with_timezone(&Utc))
    });
    Some((creds, exp))
}

// ---------------------------------------------------------------------------
// SSO sign-in expiry (read-only peek at the CLI's token cache)
// ---------------------------------------------------------------------------

/// When the account's sign-in ends, for the UI's "expires soon" warning.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionInfo {
    pub expires_at: DateTime<Utc>,
    /// `sso` — the IAM Identity Center token; `credentials` — the temporary
    /// role credentials Otto exported.
    pub source: &'static str,
    /// The CLI renews it on its own (an `sso-session` refresh token, or
    /// re-exportable creds behind a live SSO token): no warning is needed until
    /// a renewal actually fails.
    pub refreshable: bool,
}

/// One SSO token cache file → `(startUrl, expiresAt, has refresh token)`.
pub fn parse_sso_token(v: &serde_json::Value) -> Option<(String, DateTime<Utc>, bool)> {
    let start = v.get("startUrl")?.as_str()?.to_string();
    v.get("accessToken")?.as_str()?;
    let exp = DateTime::parse_from_rfc3339(v.get("expiresAt")?.as_str()?)
        .ok()?
        .with_timezone(&Utc);
    let refreshable = v
        .get("refreshToken")
        .and_then(|r| r.as_str())
        .is_some_and(|r| !r.is_empty());
    Some((start, exp, refreshable))
}

/// Latest token for `start_url` in `dir` (the CLI writes one file per
/// session / start URL; client-registration files carry no `accessToken`).
pub fn sso_token_in(dir: &Path, start_url: &str) -> Option<(DateTime<Utc>, bool)> {
    let want = start_url.trim_end_matches('/');
    let mut best: Option<(DateTime<Utc>, bool)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if let Some((url, exp, refreshable)) = parse_sso_token(&v) {
            if url.trim_end_matches('/') == want && best.is_none_or(|(b, _)| exp > b) {
                best = Some((exp, refreshable));
            }
        }
    }
    best
}

/// Session info for a profile account: the SSO token when the profile is an
/// SSO one, else the exported temporary credentials' expiry (if any).
pub fn session_info(id: &Id, profile: Option<&str>) -> Option<SessionInfo> {
    let sso = profile.and_then(|p| {
        let start = crate::discover::discover()
            .into_iter()
            .find(|d| d.name == p)?
            .sso_start_url?;
        let dir = dirs::home_dir()?.join(".aws").join("sso").join("cache");
        sso_token_in(&dir, &start)
    });
    combine(sso, expires_at(id))
}

/// Pure merge of the two sources (unit-tested).
pub fn combine(
    sso: Option<(DateTime<Utc>, bool)>,
    creds: Option<DateTime<Utc>>,
) -> Option<SessionInfo> {
    match (sso, creds) {
        // A non-refreshable SSO token is the hard end of the sign-in.
        (Some((exp, false)), _) => Some(SessionInfo {
            expires_at: exp,
            source: "sso",
            refreshable: false,
        }),
        // Refreshable SSO: the creds re-export silently; report them as such.
        (Some((exp, true)), c) => Some(SessionInfo {
            expires_at: c.unwrap_or(exp),
            source: if c.is_some() { "credentials" } else { "sso" },
            refreshable: true,
        }),
        // Non-SSO temporary creds (credential_process, …) cannot be renewed
        // by Otto without the profile's own source succeeding again — still
        // re-exported automatically, so refreshable.
        (None, Some(c)) => Some(SessionInfo {
            expires_at: c,
            source: "credentials",
            refreshable: true,
        }),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds(k: &str) -> StaticCreds {
        StaticCreds {
            access_key_id: k.into(),
            secret_access_key: "s".into(),
            session_token: Some("t".into()),
        }
    }

    #[test]
    fn parses_export_credentials_process_shape() {
        let v = serde_json::json!({"Version":1,"AccessKeyId":"ASIA1","SecretAccessKey":"sec","SessionToken":"tok","Expiration":"2026-10-03T10:00:00+00:00"});
        let (c, exp) = parse_exported(&v).unwrap();
        assert_eq!(c.access_key_id, "ASIA1");
        assert_eq!(c.session_token.as_deref(), Some("tok"));
        assert_eq!(exp.unwrap().to_rfc3339(), "2026-10-03T10:00:00+00:00");
        // Long-lived keys: no token, no expiry.
        let (c, exp) =
            parse_exported(&serde_json::json!({"AccessKeyId":"AKIA","SecretAccessKey":"x"}))
                .unwrap();
        assert!(c.session_token.is_none() && exp.is_none());
        assert!(parse_exported(&serde_json::json!({"AccessKeyId":"A"})).is_none());
    }

    #[test]
    fn cache_honours_expiry_skew_profile_and_eviction() {
        let id: Id = otto_core::new_id();
        put(
            &id,
            "dev",
            creds("A"),
            Some(Utc::now() + chrono::Duration::hours(1)),
        );
        assert_eq!(get(&id, "dev").unwrap().access_key_id, "A");
        assert!(get(&id, "other").is_none(), "a renamed profile re-exports");
        // Inside the 5-minute skew ⇒ stale.
        put(
            &id,
            "dev",
            creds("B"),
            Some(Utc::now() + chrono::Duration::minutes(3)),
        );
        assert!(get(&id, "dev").is_none());
        assert!(expires_at(&id).is_some());
        evict(&id);
        assert!(expires_at(&id).is_none());
        mark_failed(&id);
        assert!(recently_failed(&id));
        put(&id, "dev", creds("C"), None);
        assert!(!recently_failed(&id), "a success clears the back-off");
        assert!(get(&id, "dev").is_some(), "no expiry ⇒ fresh for the TTL");
        evict(&id);
    }

    #[test]
    fn reads_the_sso_token_cache() {
        let dir = std::env::temp_dir().join(format!("otto-sso-{}", otto_core::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.json"), r#"{"startUrl":"https://corp.awsapps.com/start","region":"us-east-1","accessToken":"x","expiresAt":"2026-10-03T12:00:00Z"}"#).unwrap();
        std::fs::write(dir.join("b.json"), r#"{"startUrl":"https://corp.awsapps.com/start/","accessToken":"y","expiresAt":"2026-10-03T14:00:00Z","refreshToken":"r"}"#).unwrap();
        std::fs::write(
            dir.join("reg.json"),
            r#"{"clientId":"c","expiresAt":"2027-01-01T00:00:00Z"}"#,
        )
        .unwrap();
        std::fs::write(dir.join("junk.json"), "nope").unwrap();
        let (exp, refreshable) = sso_token_in(&dir, "https://corp.awsapps.com/start").unwrap();
        assert_eq!(exp.to_rfc3339(), "2026-10-03T14:00:00+00:00");
        assert!(refreshable);
        assert!(sso_token_in(&dir, "https://other.awsapps.com/start").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn combine_prefers_the_hard_sso_deadline() {
        let t1 = Utc::now();
        let t2 = t1 + chrono::Duration::hours(1);
        let hard = combine(Some((t1, false)), Some(t2)).unwrap();
        assert_eq!(
            (hard.source, hard.refreshable, hard.expires_at),
            ("sso", false, t1)
        );
        let soft = combine(Some((t1, true)), Some(t2)).unwrap();
        assert_eq!(
            (soft.source, soft.refreshable, soft.expires_at),
            ("credentials", true, t2)
        );
        assert_eq!(combine(None, Some(t2)).unwrap().source, "credentials");
        assert!(combine(None, None).is_none());
    }
}
