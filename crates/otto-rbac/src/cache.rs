//! Short-TTL auth-lookup cache for `login` and `api` token kinds.
//!
//! # Safety contract
//!
//! **`kind='share'` and `kind='impersonation'` tokens are NEVER cached.** They
//! are explicitly revocable mid-session and must always hit the DB. The cache is
//! only applied to long-lived `login`/`api` tokens where a short stale window is
//! acceptable and revocation paths (`revoke`, `revoke_api_token`,
//! `revoke_all_for_user`) actively evict the relevant entries.
//!
//! # Architecture
//!
//! `AuthCache` wraps a `DashMap<token_hash, (AuthContext, Instant)>` and a
//! `DashMap<user_id, Vec<token_hash>>` reverse-index so per-user eviction (used
//! by `revoke_all_for_user` and `set_grants`) runs in O(n_tokens_for_user)
//! without a full-map scan. Entries are lazily expired on read; no background
//! sweeper is needed given the short TTL.
//!
//! # Feature gate
//!
//! Set `AUTH_CACHE_ENABLED = false` to disable the cache entirely (e.g. for
//! diagnosing correctness issues in production). All call sites compile-check the
//! same; the hot path becomes a direct DB call.

use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use otto_core::auth::{AuthContext, GrantsInvalidator};

/// Hard switch: set to `false` to bypass the cache on every path without
/// recompiling the guards out of existence. Individual tests can also override
/// by constructing an `AuthCache` and ignoring its presence (the authenticator
/// falls through to the DB when `enabled = false`).
pub const AUTH_CACHE_ENABLED: bool = true;

/// Cache TTL for `login` and `api` tokens. Short enough that a revoked-then-
/// reused token is accepted for at most this window *only if* the eviction path
/// somehow missed it, long enough to have measurable impact on a busy daemon.
/// In practice `revoke`/`revoke_api_token`/`revoke_all_for_user` all actively
/// evict, so this is a belt-and-suspenders backstop.
pub const AUTH_CACHE_TTL: Duration = Duration::from_secs(10);

/// The full cache state, cheaply `Arc`-cloned so `AuthRepo` and the
/// `GrantsInvalidator` impl share the same backing map.
#[derive(Clone, Default)]
pub struct AuthCache {
    /// token_hash → (AuthContext, inserted_at)
    entries: Arc<DashMap<String, (AuthContext, Instant)>>,
    /// user_id → set of token_hashes owned by that user (for per-user eviction)
    by_user: Arc<DashMap<String, Vec<String>>>,
    /// When false all get/insert/evict calls are no-ops: the authenticator
    /// falls through to the DB on every request, just as before the cache
    /// existed. Revocation paths are unaffected (they still call DB + evict,
    /// but the evict is a no-op, which is safe).
    enabled: bool,
}

impl AuthCache {
    /// Create an enabled cache with the standard TTL.
    pub fn new() -> Self {
        Self {
            entries: Arc::default(),
            by_user: Arc::default(),
            enabled: AUTH_CACHE_ENABLED,
        }
    }

    /// Create a cache with the enabled flag forced to `value`. Used in tests.
    #[cfg(test)]
    pub fn with_enabled(enabled: bool) -> Self {
        Self {
            entries: Arc::default(),
            by_user: Arc::default(),
            enabled,
        }
    }

    /// Look up a cached entry by `token_hash`. Returns `None` when the cache is
    /// disabled, the entry is absent, or it has lived past `AUTH_CACHE_TTL`.
    pub fn get(&self, token_hash: &str) -> Option<AuthContext> {
        if !self.enabled {
            return None;
        }
        let entry = self.entries.get(token_hash)?;
        let (ctx, inserted_at) = entry.value();
        if inserted_at.elapsed() > AUTH_CACHE_TTL {
            // Lazy expiry: treat as absent (will be re-fetched from DB).
            drop(entry);
            self.entries.remove(token_hash);
            return None;
        }
        Some(ctx.clone())
    }

    /// Insert a validated `AuthContext` keyed by `token_hash`, associated with
    /// `user_id` for per-user eviction. The enabled flag is respected.
    pub fn insert(&self, token_hash: String, user_id: String, ctx: AuthContext) {
        if !self.enabled {
            return;
        }
        self.entries
            .insert(token_hash.clone(), (ctx, Instant::now()));
        self.by_user
            .entry(user_id)
            .or_default()
            .push(token_hash);
    }

    /// Evict a single token entry by `token_hash`. Also cleans the reverse index
    /// to keep `by_user` from accumulating stale pointers over time.
    pub fn evict(&self, token_hash: &str) {
        self.entries.remove(token_hash);
        // Scrub from all reverse-index entries (the hash appears in at most one
        // user's vec, but a linear scan is fine for the small cardinality).
        self.by_user.retain(|_, hashes| {
            hashes.retain(|h| h != token_hash);
            !hashes.is_empty()
        });
    }

    /// Evict ALL cached entries belonging to `user_id`. Called on
    /// `revoke_all_for_user` and `set_grants` to flush stale auth/grant state.
    pub fn evict_user(&self, user_id: &str) {
        if let Some((_, hashes)) = self.by_user.remove(user_id) {
            for h in hashes {
                self.entries.remove(&h);
            }
        }
    }
}

/// Implements `GrantsInvalidator` so `GrantsRepo::set_grants` can flush a
/// user's cached auth entries without depending on `otto-rbac`.
impl GrantsInvalidator for AuthCache {
    fn invalidate_user(&self, user_id: &str) {
        self.evict_user(user_id);
    }
}

/// A no-op `GrantsInvalidator` for use where no cache is wired in (unit tests
/// of `GrantsRepo`, or any call site that does not opt in to the cache).
pub struct NoopGrantsInvalidator;

impl GrantsInvalidator for NoopGrantsInvalidator {
    fn invalidate_user(&self, _user_id: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use otto_core::auth::AuthContext;
    use otto_core::domain::{User, WorkspaceRole};

    fn fake_ctx(user_id: &str) -> AuthContext {
        let u = User {
            id: user_id.into(),
            username: user_id.into(),
            display_name: user_id.into(),
            is_root: false,
            disabled: false,
            created_at: Utc::now(),
        };
        AuthContext {
            real_user: u.clone(),
            effective_user: u,
            scope: None,
        }
    }

    #[test]
    fn hit_and_miss() {
        let cache = AuthCache::new();
        assert!(cache.get("h1").is_none(), "empty cache is a miss");
        cache.insert("h1".into(), "u1".into(), fake_ctx("u1"));
        assert!(cache.get("h1").is_some(), "freshly-inserted entry is a hit");
    }

    #[test]
    fn evict_single_removes_entry() {
        let cache = AuthCache::new();
        cache.insert("h1".into(), "u1".into(), fake_ctx("u1"));
        cache.evict("h1");
        assert!(cache.get("h1").is_none(), "evicted entry must be a miss");
    }

    #[test]
    fn evict_user_removes_all_tokens_for_that_user() {
        let cache = AuthCache::new();
        cache.insert("h1".into(), "u1".into(), fake_ctx("u1"));
        cache.insert("h2".into(), "u1".into(), fake_ctx("u1"));
        cache.insert("h3".into(), "u2".into(), fake_ctx("u2"));

        cache.evict_user("u1");

        assert!(cache.get("h1").is_none(), "u1's first token must be evicted");
        assert!(cache.get("h2").is_none(), "u1's second token must be evicted");
        assert!(
            cache.get("h3").is_some(),
            "u2's token must not be affected by u1's eviction"
        );
    }

    #[test]
    fn grants_invalidator_impl_delegates_to_evict_user() {
        let cache = AuthCache::new();
        cache.insert("hx".into(), "ux".into(), fake_ctx("ux"));
        cache.invalidate_user("ux");
        assert!(
            cache.get("hx").is_none(),
            "GrantsInvalidator::invalidate_user must evict the user's tokens"
        );
    }

    #[test]
    fn disabled_cache_is_always_miss() {
        let cache = AuthCache::with_enabled(false);
        cache.insert("h1".into(), "u1".into(), fake_ctx("u1"));
        assert!(
            cache.get("h1").is_none(),
            "a disabled cache must always return None"
        );
    }

    #[test]
    fn expired_entry_is_a_miss() {
        let cache = AuthCache::new();
        // Insert with an artificially-backdated Instant by exploiting that
        // DashMap lets us overwrite the entry.
        cache.entries.insert(
            "hx".into(),
            (fake_ctx("ux"), Instant::now() - AUTH_CACHE_TTL - Duration::from_secs(1)),
        );
        assert!(
            cache.get("hx").is_none(),
            "a past-TTL entry must be treated as a miss"
        );
    }

    /// The `WorkspaceRole` import is needed for a `scope`-bearing context test.
    #[allow(dead_code)]
    fn _uses_workspace_role(_: WorkspaceRole) {}
}
