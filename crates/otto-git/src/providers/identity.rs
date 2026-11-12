//! Who a provider client authenticates as — so a PR's reviewers and author
//! can be marked "you" (`PrReviewer::is_me`, `PrSummary::is_mine`,
//! `PrComment::can_edit`) by the forge's STABLE ids, never a display name.
//!
//! A git account's `username` is whatever the user typed to log in (an email
//! address on Bitbucket), while Bitbucket names participants by display name
//! and GitLab by full name — matching names marked nobody. Each forge returns
//! stable ids on its user objects (Bitbucket `uuid`/`account_id`, GitHub
//! `login`/`id`, GitLab `id`); [`Me`] holds the authenticated user's set of
//! them, read once from the forge's `/user` and cached in memory.
//!
//! The cache is keyed by the CREDENTIAL (provider, API root, a hash of the
//! secret), not the account row: a token rotated to another person's token
//! under the same account must not keep the old identity, and every provider
//! constructor (including the tests' `with_base`) gets a key for free. A failed
//! lookup is never cached and never fails the PR request — nothing is "you"
//! for that read and the next one asks again.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use otto_core::Result;
use sha2::{Digest, Sha256};

/// How long a resolved identity is trusted. Who owns a token does not change;
/// the bound only keeps a deleted/renamed forge account from living forever.
const TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// The authenticated forge user as its stable-id keys (`uuid:{…}`,
/// `account_id:…`, `login:…`, `id:…` — lowercased). Built by each provider
/// from its `/user` payload with the SAME key function it uses on PR user
/// objects, so membership is the whole comparison.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Me {
    keys: HashSet<String>,
}

impl Me {
    /// `None` when the payload carried no usable id (an empty `/user` body
    /// must never make every id-less participant "you").
    pub(crate) fn from_keys(keys: Vec<String>) -> Option<Me> {
        let keys: HashSet<String> = keys.into_iter().filter(|k| !k.is_empty()).collect();
        (!keys.is_empty()).then_some(Me { keys })
    }

    /// Whether a user object's keys name this account.
    pub(crate) fn owns(&self, user_keys: &[String]) -> bool {
        user_keys.iter().any(|k| self.keys.contains(k))
    }
}

/// [`Me::owns`] with no identity at hand ⇒ false.
pub(crate) fn is_me(me: Option<&Me>, user_keys: &[String]) -> bool {
    me.is_some_and(|m| m.owns(user_keys))
}

/// `"{kind}:{value}"`, lowercased; empty when `value` is empty.
pub(crate) fn key(kind: &str, value: &str) -> String {
    let v = value.trim();
    if v.is_empty() {
        String::new()
    } else {
        format!("{kind}:{}", v.to_ascii_lowercase())
    }
}

/// `id:<n>` for a numeric id (0 = absent).
pub(crate) fn num_key(n: u64) -> String {
    if n == 0 {
        String::new()
    } else {
        format!("id:{n}")
    }
}

/// The cache key of one credential: provider, API root, and a hash of the
/// secret material (never the secret itself).
pub(crate) fn cache_key(provider: &str, base: &str, secret: &str) -> String {
    format!(
        "{provider}|{base}|{}",
        hex::encode(Sha256::digest(secret.as_bytes()))
    )
}

fn cache() -> &'static Mutex<HashMap<String, (Instant, Me)>> {
    static C: OnceLock<Mutex<HashMap<String, (Instant, Me)>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// The identity for `key`, from the cache or — on a miss — `fetch`. A fetch
/// that fails (or yields no ids) is logged at debug and answers `None`
/// without being cached.
pub(crate) async fn cached<F, Fut>(key: &str, fetch: F) -> Option<Me>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Option<Me>>>,
{
    if enabled() {
        let guard = cache().lock().unwrap_or_else(|p| p.into_inner());
        if let Some((at, me)) = guard.get(key) {
            if at.elapsed() < TTL {
                return Some(me.clone());
            }
        }
    }
    match fetch().await {
        Ok(Some(me)) => {
            if enabled() {
                let mut guard = cache().lock().unwrap_or_else(|p| p.into_inner());
                // Bounded by the number of credentials in use; stale rows go.
                guard.retain(|_, (at, _)| at.elapsed() < TTL);
                guard.insert(key.to_string(), (Instant::now(), me.clone()));
            }
            Some(me)
        }
        Ok(None) => None,
        Err(e) => {
            tracing::debug!("forge identity lookup failed (nothing marked as you): {e}");
            None
        }
    }
}

/// Unit tests share one process and wiremock pools its servers (a later test
/// gets an earlier one's port and the same "tok" token, so the same key): the
/// identity cache follows the GET cache's per-test opt-in.
fn enabled() -> bool {
    super::client::cache_enabled()
}

/// Drop every cached identity for an API root (`enable_cache_for_tests`).
#[cfg(test)]
pub(crate) fn forget_for_tests(base: &str) {
    let mut guard = cache().lock().unwrap_or_else(|p| p.into_inner());
    guard.retain(|k, _| !k.split('|').nth(1).is_some_and(|b| b.starts_with(base)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_lowercased_and_empty_values_dropped() {
        assert_eq!(key("uuid", "{ABC-1}"), "uuid:{abc-1}");
        assert_eq!(key("login", " "), "");
        assert_eq!(num_key(0), "");
        assert_eq!(num_key(42), "id:42");
        assert!(Me::from_keys(vec![String::new()]).is_none());
        let me = Me::from_keys(vec![key("uuid", "{U1}"), num_key(7)]).unwrap();
        assert!(me.owns(&[key("uuid", "{u1}")]));
        assert!(me.owns(&["x".into(), num_key(7)]));
        assert!(!me.owns(&[key("uuid", "{u2}")]));
        assert!(!is_me(None, &[num_key(7)]));
    }

    #[test]
    fn cache_key_never_carries_the_secret() {
        let k = cache_key("github", "https://api.github.com", "ghp_secret");
        assert!(!k.contains("ghp_secret"));
        assert_ne!(k, cache_key("github", "https://api.github.com", "other"));
    }
}
