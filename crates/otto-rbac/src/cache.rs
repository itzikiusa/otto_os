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
//! `DashMap<user_id, HashSet<token_hash>>` reverse-index so per-user eviction (used
//! by `revoke_all_for_user` and `set_grants`) runs in O(n_tokens_for_user)
//! without a full-map scan. Entries expire on read and are capped at 4096;
//! no background sweeper is needed. Publication is serialized with revocation,
//! and a bounded invalidation history prevents an older DB lookup from restoring
//! a revoked entry. Unrelated revocations do not discard a valid pending fill.
//!
//! # Feature gate
//!
//! Set `AUTH_CACHE_ENABLED = false` to disable the cache entirely (e.g. for
//! diagnosing correctness issues in production). All call sites compile-check the
//! same; the hot path becomes a direct DB call.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock, Weak};
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

type Entries = DashMap<String, (AuthContext, Instant)>;
type ByUser = DashMap<String, HashSet<String>>;
const MAX_ENTRIES: usize = 4096;
const MAX_INVALIDATIONS: usize = 128;

enum Invalidated {
    Token(String),
    User(String),
}

#[derive(Default)]
struct InvalidationHistory {
    generation: u64,
    changes: VecDeque<(u64, Invalidated)>,
}

impl InvalidationHistory {
    fn record(&mut self, change: Invalidated) {
        self.generation = self.generation.wrapping_add(1);
        self.changes.push_back((self.generation, change));
        if self.changes.len() > MAX_INVALIDATIONS {
            self.changes.pop_front();
        }
    }

    fn allows(&self, started: u64, token_hash: &str, user_id: &str) -> bool {
        if started == self.generation {
            return true;
        }
        // A lookup older than the retained history cannot prove it survived all
        // relevant revocations. The request can finish, but must not cache it.
        if started > self.generation
            || self
                .changes
                .front()
                .is_none_or(|(first, _)| *first > started.saturating_add(1))
        {
            return false;
        }
        !self.changes.iter().any(|(generation, change)| {
            *generation > started
                && match change {
                    Invalidated::Token(hash) => hash == token_hash,
                    Invalidated::User(user) => user == user_id,
                }
        })
    }
}

/// Every live [`AuthCache`]'s maps (S8-302). Revocation paths evict through
/// [`evict_everywhere`] / [`evict_user_everywhere`], so a revoke issued from a
/// cache-less `AuthRepo::new(pool)` (logout, PAT revoke, password change,
/// disable…) still drops the entry the shared authenticator cached — no
/// caller can forget to thread the cache through. Weak refs: a dropped cache
/// (a test's) is pruned on the next registration.
type Registered = (
    Weak<Entries>,
    Weak<ByUser>,
    Weak<Mutex<InvalidationHistory>>,
);
type LiveCache = (Arc<Entries>, Arc<ByUser>, Arc<Mutex<InvalidationHistory>>);

fn registry() -> &'static Mutex<Vec<Registered>> {
    static REG: OnceLock<Mutex<Vec<Registered>>> = OnceLock::new();
    REG.get_or_init(Default::default)
}

fn register(
    entries: &Arc<Entries>,
    by_user: &Arc<ByUser>,
    generation: &Arc<Mutex<InvalidationHistory>>,
) {
    let mut reg = registry().lock().unwrap_or_else(|p| p.into_inner());
    reg.retain(|(e, _, _)| e.strong_count() > 0);
    reg.push((
        Arc::downgrade(entries),
        Arc::downgrade(by_user),
        Arc::downgrade(generation),
    ));
}

fn live_caches() -> Vec<LiveCache> {
    registry()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .filter_map(|(e, u, g)| Some((e.upgrade()?, u.upgrade()?, g.upgrade()?)))
        .collect()
}

fn evict_in(
    entries: &Entries,
    by_user: &ByUser,
    generation: &Mutex<InvalidationHistory>,
    token_hash: &str,
) {
    let mut generation = generation.lock().unwrap_or_else(|e| e.into_inner());
    generation.record(Invalidated::Token(token_hash.into()));
    entries.remove(token_hash);
    // Scrub from all reverse-index entries (the hash appears in at most one
    // user's set, but a linear scan is fine for the small cardinality).
    by_user.retain(|_, hashes| {
        hashes.retain(|h| h != token_hash);
        !hashes.is_empty()
    });
}

fn evict_user_in(
    entries: &Entries,
    by_user: &ByUser,
    generation: &Mutex<InvalidationHistory>,
    user_id: &str,
) {
    let mut generation = generation.lock().unwrap_or_else(|e| e.into_inner());
    generation.record(Invalidated::User(user_id.into()));
    if let Some((_, hashes)) = by_user.remove(user_id) {
        for h in hashes {
            entries.remove(&h);
        }
    }
}

/// Evict `token_hash` from EVERY live [`AuthCache`] in the process.
pub fn evict_everywhere(token_hash: &str) {
    for (entries, by_user, generation) in live_caches() {
        evict_in(&entries, &by_user, &generation, token_hash);
    }
}

/// Evict every cached token of `user_id` from EVERY live [`AuthCache`].
pub fn evict_user_everywhere(user_id: &str) {
    for (entries, by_user, generation) in live_caches() {
        evict_user_in(&entries, &by_user, &generation, user_id);
    }
}

/// The full cache state, cheaply `Arc`-cloned so `AuthRepo` and the
/// `GrantsInvalidator` impl share the same backing map.
#[derive(Clone)]
pub struct AuthCache {
    /// token_hash → (AuthContext, inserted_at)
    entries: Arc<Entries>,
    /// user_id → set of token_hashes owned by that user (for per-user eviction)
    by_user: Arc<ByUser>,
    /// Serializes fill publication with revocation; captured before the DB read.
    generation: Arc<Mutex<InvalidationHistory>>,
    /// When false, reads miss and fills are ignored: the authenticator always
    /// reads the DB. Revocation still records invalidations, but has no cached
    /// contexts to remove.
    enabled: bool,
    /// Per-user feature/plugin grant rows for the feature guard (SG-11). Rides
    /// on this cache so the one `GrantsInvalidator` (`set_grants`,
    /// `set_plugin_grants`, `revoke_all_for_user`) flushes both at once.
    grants: otto_state::GrantCache,
}

impl AuthCache {
    /// Create an enabled cache with the standard TTL.
    pub fn new() -> Self {
        Self::build(AUTH_CACHE_ENABLED)
    }

    /// Create a cache with the enabled flag forced to `value`. Used in tests.
    #[cfg(test)]
    pub fn with_enabled(enabled: bool) -> Self {
        Self::build(enabled)
    }

    /// Every cache registers itself so process-wide revocations reach it.
    fn build(enabled: bool) -> Self {
        let entries = Arc::default();
        let by_user = Arc::default();
        let generation = Arc::new(Mutex::new(InvalidationHistory::default()));
        register(&entries, &by_user, &generation);
        Self {
            entries,
            by_user,
            generation,
            enabled,
            grants: otto_state::GrantCache::new(),
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
            let expired_at = *inserted_at;
            drop(entry);
            let _write = self.generation.lock().unwrap_or_else(|e| e.into_inner());
            // A concurrent refresh may have replaced the observed entry while
            // we acquired the write gate; never delete its fresh value/index.
            if let Some((_, (ctx, _))) = self
                .entries
                .remove_if(token_hash, |_, (_, at)| *at == expired_at)
            {
                self.by_user.remove_if_mut(&ctx.real_user.id, |_, hashes| {
                    hashes.remove(token_hash);
                    hashes.is_empty()
                });
            }
            return None;
        }
        Some(ctx.clone())
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .generation
    }

    /// Publish a DB lookup that began at `started`.
    pub(crate) fn insert_if_current(
        &self,
        started: u64,
        token_hash: String,
        user_id: String,
        ctx: AuthContext,
    ) {
        if !self.enabled {
            return;
        }
        let history = self.generation.lock().unwrap_or_else(|e| e.into_inner());
        if history.allows(started, &token_hash, &user_id) {
            self.insert_locked(token_hash, user_id, ctx);
        }
    }

    /// Tests can publish a context synchronously without a pending DB lookup.
    #[cfg(test)]
    fn insert(&self, token_hash: String, user_id: String, ctx: AuthContext) {
        if !self.enabled {
            return;
        }
        let _write = self.generation.lock().unwrap_or_else(|e| e.into_inner());
        self.insert_locked(token_hash, user_id, ctx);
    }

    /// Called only under the publication/revocation write gate.
    fn insert_locked(&self, token_hash: String, user_id: String, ctx: AuthContext) {
        if !self.entries.contains_key(&token_hash) && self.entries.len() >= MAX_ENTRIES {
            self.entries
                .retain(|_, (_, inserted_at)| inserted_at.elapsed() <= AUTH_CACHE_TTL);
            self.by_user.retain(|_, hashes| {
                hashes.retain(|hash| self.entries.contains_key(hash));
                !hashes.is_empty()
            });
            if self.entries.len() >= MAX_ENTRIES {
                // A cache miss is safe. Clear in one bounded pass instead of
                // introducing per-hit LRU contention or unbounded retention.
                self.entries.clear();
                self.by_user.clear();
            }
        }
        self.entries
            .insert(token_hash.clone(), (ctx, Instant::now()));
        self.by_user.entry(user_id).or_default().insert(token_hash);
    }

    /// The shared grant-row cache (hand it to `GrantsRepo::with_cache`).
    /// Disabled with the rest of the cache: then every lookup reads the DB.
    pub fn grant_cache(&self) -> Option<otto_state::GrantCache> {
        self.enabled.then(|| self.grants.clone())
    }

    /// Evict a single token entry by `token_hash`. Also cleans the reverse index
    /// to keep `by_user` from accumulating stale pointers over time.
    pub fn evict(&self, token_hash: &str) {
        evict_in(&self.entries, &self.by_user, &self.generation, token_hash);
    }

    /// Evict ALL cached entries belonging to `user_id`. Called on
    /// `revoke_all_for_user` and `set_grants` to flush stale auth/grant state.
    pub fn evict_user(&self, user_id: &str) {
        self.grants.invalidate_user(user_id);
        evict_user_in(&self.entries, &self.by_user, &self.generation, user_id);
    }
}

impl Default for AuthCache {
    fn default() -> Self {
        Self::new()
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
            mcp_only: false,
            mcp_scope: None,
            mcp_internal: false,
            mcp_session_id: None,
            managed_session_id: None,
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

        assert!(
            cache.get("h1").is_none(),
            "u1's first token must be evicted"
        );
        assert!(
            cache.get("h2").is_none(),
            "u1's second token must be evicted"
        );
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
            (
                fake_ctx("ux"),
                Instant::now() - AUTH_CACHE_TTL - Duration::from_secs(1),
            ),
        );
        assert!(
            cache.get("hx").is_none(),
            "a past-TTL entry must be treated as a miss"
        );
    }

    /// A long-lived login refreshes every TTL. Its eviction bookkeeping must
    /// track the live token, not accumulate one allocation per refresh.
    #[test]
    fn repeated_expiry_refresh_keeps_reverse_index_bounded() {
        let cache = AuthCache::new();
        for _ in 0..100 {
            cache.insert(
                "refresh-h".into(),
                "refresh-u".into(),
                fake_ctx("refresh-u"),
            );
            cache.entries.get_mut("refresh-h").unwrap().1 =
                Instant::now() - AUTH_CACHE_TTL - Duration::from_secs(1);
            assert!(cache.get("refresh-h").is_none());
        }
        cache.insert(
            "refresh-h".into(),
            "refresh-u".into(),
            fake_ctx("refresh-u"),
        );
        let reverse_entries = cache.by_user.get("refresh-u").unwrap().len();
        assert_eq!(cache.entries.len(), 1);
        cache.evict_user("refresh-u");
        assert!(cache.get("refresh-h").is_none());
        assert!(!cache.by_user.contains_key("refresh-u"));
        assert_eq!(
            reverse_entries, 1,
            "one live token needs one reverse-index entry"
        );
    }

    #[test]
    fn process_wide_eviction_reaches_every_live_cache() {
        // S8-302: a revoke from a cache-less repo must still drop the entry
        // the shared authenticator cached.
        let cache = AuthCache::new();
        cache.insert("pw-h1".into(), "pw-u1".into(), fake_ctx("pw-u1"));
        cache.insert("pw-h2".into(), "pw-u2".into(), fake_ctx("pw-u2"));
        evict_everywhere("pw-h1");
        assert!(cache.get("pw-h1").is_none());
        evict_user_everywhere("pw-u2");
        assert!(cache.get("pw-h2").is_none());
    }

    #[test]
    fn distinct_expired_tokens_do_not_accumulate_for_process_lifetime() {
        let cache = AuthCache::new();
        for i in 0..4128 {
            let hash = format!("old-token-{i}");
            cache.insert(
                hash.clone(),
                "expired-owner".into(),
                fake_ctx("expired-owner"),
            );
            cache.entries.get_mut(&hash).unwrap().1 =
                Instant::now() - AUTH_CACHE_TTL - Duration::from_secs(1);
        }
        assert!(
            cache.entries.len() <= 4096,
            "{} expired contexts retained",
            cache.entries.len()
        );
        assert!(cache.by_user.iter().map(|entry| entry.len()).sum::<usize>() <= 4096);
        cache.insert(
            "latest-token".into(),
            "latest-owner".into(),
            fake_ctx("latest-owner"),
        );
        assert!(cache.get("latest-token").is_some());
    }

    #[test]
    fn unrelated_revocation_does_not_discard_a_valid_pending_fill() {
        let cache = AuthCache::new();
        let started = cache.generation();
        cache.evict("unrelated-h");
        cache.evict_user("unrelated-u");
        cache.insert_if_current(
            started,
            "unaffected-h".into(),
            "unaffected-u".into(),
            fake_ctx("unaffected-u"),
        );
        assert!(cache.get("unaffected-h").is_some());
    }

    #[test]
    fn lazy_expiry_removes_reverse_index_without_waiting_for_capacity() {
        let cache = AuthCache::new();
        for i in 0..MAX_ENTRIES + 32 {
            let hash = format!("read-expired-{i}");
            cache.insert(
                hash.clone(),
                "read-expired-u".into(),
                fake_ctx("read-expired-u"),
            );
            cache.entries.get_mut(&hash).unwrap().1 =
                Instant::now() - AUTH_CACHE_TTL - Duration::from_secs(1);
            assert!(cache.get(&hash).is_none());
        }
        assert!(cache.entries.is_empty());
        assert!(cache.by_user.is_empty());
    }

    #[test]
    fn fill_older_than_bounded_invalidation_history_is_not_cached() {
        let cache = AuthCache::new();
        let started = cache.generation();
        cache.evict("history-h");
        for i in 0..MAX_INVALIDATIONS {
            cache.evict(&format!("later-unrelated-{i}"));
        }
        assert_eq!(
            cache.generation.lock().unwrap().changes.len(),
            MAX_INVALIDATIONS
        );
        cache.insert_if_current(
            started,
            "history-h".into(),
            "history-u".into(),
            fake_ctx("history-u"),
        );
        assert!(cache.get("history-h").is_none());
        cache.insert_if_current(
            cache.generation(),
            "history-fresh-h".into(),
            "history-u".into(),
            fake_ctx("history-u"),
        );
        assert!(cache.get("history-fresh-h").is_some());
    }

    #[test]
    fn live_entries_are_bounded_and_latest_context_remains_usable() {
        let cache = AuthCache::new();
        for i in 0..MAX_ENTRIES + 32 {
            cache.insert(
                format!("live-bounded-{i}"),
                "live-bounded-u".into(),
                fake_ctx("live-bounded-u"),
            );
        }
        assert!(cache.entries.len() <= MAX_ENTRIES);
        assert_eq!(
            cache.entries.len(),
            cache.by_user.iter().map(|entry| entry.len()).sum::<usize>()
        );
        assert!(cache
            .get(&format!("live-bounded-{}", MAX_ENTRIES + 31))
            .is_some());
    }

    #[test]
    fn revocation_while_lookup_is_pending_cannot_repopulate_the_cache() {
        for revoke_user in [false, true] {
            let cache = AuthCache::new();
            // The DB read has validated the token but has not published its result.
            let started = cache.generation();
            if revoke_user {
                evict_user_everywhere("pending-u");
            } else {
                evict_everywhere("pending-h");
            }
            cache.insert_if_current(
                started,
                "pending-h".into(),
                "pending-u".into(),
                fake_ctx("pending-u"),
            );
            assert!(
                cache.get("pending-h").is_none(),
                "revocation must survive a delayed lookup result"
            );
            assert!(!cache.by_user.contains_key("pending-u"));
            // A lookup begun after the invalidation can cache a newly validated token.
            cache.insert_if_current(
                cache.generation(),
                "fresh-h".into(),
                "fresh-u".into(),
                fake_ctx("fresh-u"),
            );
            assert!(cache.get("fresh-h").is_some());
        }
    }

    /// The `WorkspaceRole` import is needed for a `scope`-bearing context test.
    #[allow(dead_code)]
    fn _uses_workspace_role(_: WorkspaceRole) {}
}
