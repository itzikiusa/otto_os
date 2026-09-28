//! Per-user per-feature capability grants repository.
//!
//! Default-deny: no row ⇒ `Capability::None`. Root users bypass the table and
//! always receive `Capability::Admin` (matches the `WorkspacesRepo::role_of`
//! pattern for root bypass).
//!
//! # Cache invalidation
//!
//! When an [`otto_core::auth::GrantsInvalidator`] is attached (typically the
//! `AuthCache` from `otto-rbac`), [`GrantsRepo::set_grants`] calls
//! `invalidate_user` after committing new grants so cached auth contexts for
//! that user are flushed immediately. Use [`GrantsRepo::new_with_invalidator`]
//! at the call site that wires in the cache; plain [`GrantsRepo::new`] installs
//! a no-op invalidator and keeps the existing behaviour.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::DbPool;
use otto_core::auth::GrantsInvalidator;
use otto_core::domain::{Capability, Feature, User, WorkspaceRole};
use otto_core::{Error, Result};

/// The feature capability that answers for a **global** (workspace-less) row
/// when a gate would otherwise demand the workspace role `min`.
///
/// Some resources — connections above all — are deliberately created
/// workspace-independent ("a global library"), so the workspace axis has
/// nothing to check for them. Falling back to *root only* on that branch locks
/// every non-root user out of the entire library, which is not what the feature
/// model promises. The feature axis answers instead, on the identical ladder:
/// `Viewer→View`, `Editor→Edit`, `Admin→Admin`. `effective =
/// min(feature_grant, ws_role)` still holds — for a global row the workspace
/// term is simply vacuous.
pub fn capability_for_role(min: WorkspaceRole) -> Capability {
    match min {
        WorkspaceRole::Viewer => Capability::View,
        WorkspaceRole::Editor => Capability::Edit,
        WorkspaceRole::Admin => Capability::Admin,
    }
}

/// No-op [`GrantsInvalidator`] used when no cache is wired in.
struct NoopInvalidator;

impl GrantsInvalidator for NoopInvalidator {
    fn invalidate_user(&self, _user_id: &str) {}
}

/// How long a user's cached grant rows answer the per-request guard (SG-11).
/// The same window as `otto-rbac`'s `AUTH_CACHE_TTL`: every write path through
/// [`GrantsRepo`] evicts the user at once, so the TTL only bounds writes made
/// behind the repo's back (a state restore, a raw SQL edit).
pub const GRANT_CACHE_TTL: Duration = Duration::from_secs(10);

/// One user's grant rows, as stored (strings parsed on lookup, so a row naming
/// a feature this build doesn't know never poisons the rest).
struct CachedGrants {
    at: Instant,
    features: HashMap<String, String>,
    plugins: Option<HashMap<String, String>>,
}

/// Shared per-user grant cache for the non-root request path (the feature
/// guard ran 1–2 uncached `user_feature_grants` lookups per request). A user's
/// features load in ONE query and answer every feature for [`GRANT_CACHE_TTL`].
/// Cheap to clone (an `Arc`); `otto-rbac`'s `AuthCache` owns the daemon's copy
/// and clears it from `invalidate_user`, so a grant change is visible at once.
#[derive(Clone, Default)]
pub struct GrantCache {
    inner: Arc<Mutex<HashMap<String, CachedGrants>>>,
    /// Row loads that hit the DB (tests assert hits don't).
    loads: Arc<std::sync::atomic::AtomicU64>,
    /// Bumped by every invalidation: a fill that STARTED before one (it may
    /// have read the pre-change rows) is not stored.
    gen: Arc<std::sync::atomic::AtomicU64>,
}

impl GrantCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop everything cached for `user_id`.
    pub fn invalidate_user(&self, user_id: &str) {
        self.gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Ok(mut m) = self.inner.lock() {
            m.remove(user_id);
        }
    }

    /// Drop the whole cache (a state restore replaced the table).
    pub fn clear(&self) {
        self.gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Ok(mut m) = self.inner.lock() {
            m.clear();
        }
    }

    /// The invalidation generation a fill starts from (see `store`).
    fn generation(&self) -> u64 {
        self.gen.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// DB loads so far (feature + plugin fills).
    pub fn loads(&self) -> u64 {
        self.loads.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn lookup(&self, user_id: &str, key: &str, plugin: bool) -> Option<Option<String>> {
        let mut m = self.inner.lock().ok()?;
        let hit = m.get(user_id)?;
        if hit.at.elapsed() > GRANT_CACHE_TTL {
            m.remove(user_id);
            return None;
        }
        let rows = if plugin {
            hit.plugins.as_ref()?
        } else {
            &hit.features
        };
        Some(rows.get(key).cloned())
    }

    /// Replace `user_id`'s entry (a whole fill: both row sets share one age).
    fn store(
        &self,
        user_id: &str,
        started: u64,
        features: HashMap<String, String>,
        plugins: Option<HashMap<String, String>>,
    ) {
        self.loads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let Ok(mut m) = self.inner.lock() else { return };
        // An invalidation landed while this fill was reading: its rows may
        // predate the change — answer this request with them, cache nothing.
        if self.generation() != started {
            return;
        }
        // Bound: a daemon has a handful of users; never let a flood of ids grow it.
        if m.len() >= 1024 && !m.contains_key(user_id) {
            m.retain(|_, v| v.at.elapsed() <= GRANT_CACHE_TTL);
            if m.len() >= 1024 {
                m.clear();
            }
        }
        m.insert(
            user_id.to_string(),
            CachedGrants {
                at: Instant::now(),
                features,
                plugins,
            },
        );
    }
}

#[derive(Clone)]
pub struct GrantsRepo {
    pool: DbPool,
    /// Called in `set_grants` after committing. The default is a no-op.
    invalidator: Arc<dyn GrantsInvalidator>,
    /// Optional read-through cache for `capability_of[_plugin]` (SG-11).
    cache: Option<GrantCache>,
}

impl GrantsRepo {
    /// Construct with a no-op invalidator (no auth cache). All callers that do
    /// not opt in to caching should use this constructor; behaviour is identical
    /// to the previous single-constructor API.
    pub fn new(pool: impl Into<DbPool>) -> Self {
        let pool: DbPool = pool.into();
        Self {
            pool,
            invalidator: Arc::new(NoopInvalidator),
            cache: None,
        }
    }

    /// Construct with an explicit [`GrantsInvalidator`]. Pass the `AuthCache`
    /// from `otto-rbac` here so grant changes immediately flush the affected
    /// user's cached auth context.
    pub fn new_with_invalidator(pool: DbPool, inv: Arc<dyn GrantsInvalidator>) -> Self {
        Self {
            pool,
            invalidator: inv,
            cache: None,
        }
    }

    /// Answer `capability_of[_plugin]` through `cache` (read-through, TTL
    /// [`GRANT_CACHE_TTL`]); `set_grants` / `set_plugin_grants` evict it.
    pub fn with_cache(mut self, cache: GrantCache) -> Self {
        self.cache = Some(cache);
        self
    }

    /// Fill the cache with every feature grant row of `user_id` (one query).
    async fn load_features(&self, user_id: &str) -> Result<HashMap<String, String>> {
        use sqlx::Row;
        let rows =
            sqlx::query("SELECT feature, capability FROM user_feature_grants WHERE user_id = ?")
                .bind(user_id)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| Error::Internal(format!("capability_of: {e}")))?;
        Ok(rows
            .iter()
            .map(|r| {
                (
                    r.get::<String, _>("feature"),
                    r.get::<String, _>("capability"),
                )
            })
            .collect())
    }

    async fn load_plugins(&self, user_id: &str) -> Result<HashMap<String, String>> {
        use sqlx::Row;
        let rows = sqlx::query(
            "SELECT plugin_key, capability FROM plugin_feature_grants WHERE user_id = ?",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("capability_of_plugin: {e}")))?;
        Ok(rows
            .iter()
            .map(|r| {
                (
                    r.get::<String, _>("plugin_key"),
                    r.get::<String, _>("capability"),
                )
            })
            .collect())
    }

    fn parse_cap(stored: Option<String>) -> Result<Capability> {
        match stored {
            None => Ok(Capability::None),
            Some(s) => Capability::parse(&s)
                .ok_or_else(|| Error::Internal(format!("bad capability value '{s}'"))),
        }
    }

    /// Return the effective capability of `user` for `feature`.
    ///
    /// Root ⇒ `Admin` unconditionally. Otherwise, the row's capability or
    /// `Capability::None` when no row exists.
    pub async fn capability_of(&self, user: &User, feature: Feature) -> Result<Capability> {
        if user.is_root {
            return Ok(Capability::Admin);
        }
        if let Some(cache) = &self.cache {
            if let Some(hit) = cache.lookup(&user.id, feature.as_str(), false) {
                return Self::parse_cap(hit);
            }
            let started = cache.generation();
            let rows = self.load_features(&user.id).await?;
            let stored = rows.get(feature.as_str()).cloned();
            cache.store(&user.id, started, rows, None);
            return Self::parse_cap(stored);
        }
        let row = sqlx::query(
            "SELECT capability FROM user_feature_grants WHERE user_id = ? AND feature = ?",
        )
        .bind(&user.id)
        .bind(feature.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("capability_of: {e}")))?;

        match row {
            None => Ok(Capability::None),
            Some(r) => {
                use sqlx::Row;
                let s: String = r.get("capability");
                Capability::parse(&s)
                    .ok_or_else(|| Error::Internal(format!("bad capability value '{s}'")))
            }
        }
    }

    /// Gate a **global** (workspace-less) resource on the feature axis.
    ///
    /// Root always passes; otherwise the caller's grant for `feature` must be
    /// at least `need` (see [`capability_for_role`] for how a workspace-role
    /// bar maps onto that ladder). `denied` is the 403 body — say which grant
    /// is missing, since the caller can't infer it from a bare "forbidden".
    pub async fn check_global(
        &self,
        user: &User,
        feature: Feature,
        need: Capability,
        denied: &str,
    ) -> Result<()> {
        if user.is_root {
            return Ok(());
        }
        if self.capability_of(user, feature).await? >= need {
            Ok(())
        } else {
            Err(Error::Forbidden(denied.to_string()))
        }
    }

    /// Return all grants for `user_id` as `(Feature, Capability)` pairs.
    pub async fn grants_for(&self, user_id: &str) -> Result<Vec<(Feature, Capability)>> {
        use sqlx::Row;
        let rows = sqlx::query(
            "SELECT feature, capability FROM user_feature_grants WHERE user_id = ? ORDER BY feature",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("grants_for: {e}")))?;

        rows.iter()
            .map(|r| {
                let fs: String = r.get("feature");
                let cs: String = r.get("capability");
                let feature = Feature::parse(&fs)
                    .ok_or_else(|| Error::Internal(format!("bad feature value '{fs}'")))?;
                let cap = Capability::parse(&cs)
                    .ok_or_else(|| Error::Internal(format!("bad capability value '{cs}'")))?;
                Ok((feature, cap))
            })
            .collect()
    }

    /// Atomically replace all grants for `user_id`.
    ///
    /// Deletes existing rows and inserts `grants` in a single transaction.
    /// Passing an empty slice effectively revokes all grants.
    ///
    /// After a successful commit, calls [`GrantsInvalidator::invalidate_user`]
    /// so any auth-lookup cache evicts stale entries for this user. The
    /// invalidation happens after the commit to ensure DB consistency: if the
    /// commit fails, the cache is not touched (stale entries harmlessly re-read
    /// the unchanged DB). There is a small window between the commit and the
    /// evict where the cache serves old grants; it is bounded by
    /// `AUTH_CACHE_TTL` (10 s) in the worst case of a racing eviction failure,
    /// which is acceptable for a grant-change path (admin-only, infrequent).
    pub async fn set_grants(&self, user_id: &str, grants: &[(Feature, Capability)]) -> Result<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| Error::Internal(format!("begin tx: {e}")))?;

        sqlx::query("DELETE FROM user_feature_grants WHERE user_id = ?")
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::Internal(format!("delete grants: {e}")))?;

        for (feature, cap) in grants {
            // Skip Capability::None — it's the absence of a row, not a stored state.
            if *cap == Capability::None {
                continue;
            }
            sqlx::query(
                "INSERT INTO user_feature_grants (user_id, feature, capability) VALUES (?, ?, ?)",
            )
            .bind(user_id)
            .bind(feature.as_str())
            .bind(cap.as_str())
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::Internal(format!("insert grant: {e}")))?;
        }

        tx.commit()
            .await
            .map_err(|e| Error::Internal(format!("commit tx: {e}")))?;

        // Evict after commit: the new grants are now durable. Any auth context
        // cached for this user may carry stale capability information; flush it.
        self.invalidator.invalidate_user(user_id);
        if let Some(cache) = &self.cache {
            cache.invalidate_user(user_id);
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Custom-plugin grants — the string-keyed (by plugin slug) RBAC axis,
    // parallel to the closed `Feature` enum. Backed by `plugin_feature_grants`
    // (migration 0061). Same default-deny + root-bypass semantics as above.
    // -----------------------------------------------------------------------

    /// Return the effective capability of `user` for plugin `slug`.
    ///
    /// Root ⇒ `Admin`. Otherwise the row's capability or `Capability::None`.
    pub async fn capability_of_plugin(&self, user: &User, slug: &str) -> Result<Capability> {
        if user.is_root {
            return Ok(Capability::Admin);
        }
        if let Some(cache) = &self.cache {
            if let Some(hit) = cache.lookup(&user.id, slug, true) {
                return Self::parse_cap(hit);
            }
            let started = cache.generation();
            let plugins = self.load_plugins(&user.id).await?;
            let stored = plugins.get(slug).cloned();
            // Plugins ride on a feature fill: load both so the entry is whole.
            let features = self.load_features(&user.id).await?;
            cache.store(&user.id, started, features, Some(plugins));
            return Self::parse_cap(stored);
        }
        let row = sqlx::query(
            "SELECT capability FROM plugin_feature_grants WHERE user_id = ? AND plugin_key = ?",
        )
        .bind(&user.id)
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("capability_of_plugin: {e}")))?;

        match row {
            None => Ok(Capability::None),
            Some(r) => {
                use sqlx::Row;
                let s: String = r.get("capability");
                Capability::parse(&s)
                    .ok_or_else(|| Error::Internal(format!("bad capability value '{s}'")))
            }
        }
    }

    /// Return all plugin grants for `user_id` as `(slug, Capability)` pairs.
    pub async fn plugin_grants_for(&self, user_id: &str) -> Result<Vec<(String, Capability)>> {
        use sqlx::Row;
        let rows = sqlx::query(
            "SELECT plugin_key, capability FROM plugin_feature_grants WHERE user_id = ? ORDER BY plugin_key",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("plugin_grants_for: {e}")))?;

        rows.iter()
            .map(|r| {
                let slug: String = r.get("plugin_key");
                let cs: String = r.get("capability");
                let cap = Capability::parse(&cs)
                    .ok_or_else(|| Error::Internal(format!("bad capability value '{cs}'")))?;
                Ok((slug, cap))
            })
            .collect()
    }

    /// Atomically replace all plugin grants for `user_id` (skips `None`).
    /// Flushes the user's auth cache after commit, like [`Self::set_grants`].
    pub async fn set_plugin_grants(
        &self,
        user_id: &str,
        grants: &[(String, Capability)],
    ) -> Result<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| Error::Internal(format!("begin tx: {e}")))?;

        sqlx::query("DELETE FROM plugin_feature_grants WHERE user_id = ?")
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::Internal(format!("delete plugin grants: {e}")))?;

        for (slug, cap) in grants {
            if *cap == Capability::None {
                continue;
            }
            sqlx::query(
                "INSERT INTO plugin_feature_grants (user_id, plugin_key, capability) VALUES (?, ?, ?)",
            )
            .bind(user_id)
            .bind(slug)
            .bind(cap.as_str())
            .execute(&mut *tx)
            .await
            .map_err(|e| Error::Internal(format!("insert plugin grant: {e}")))?;
        }

        tx.commit()
            .await
            .map_err(|e| Error::Internal(format!("commit tx: {e}")))?;

        self.invalidator.invalidate_user(user_id);
        if let Some(cache) = &self.cache {
            cache.invalidate_user(user_id);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use otto_core::new_id;

    use crate::convert::fmt;

    async fn mem_pool() -> DbPool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool.into()
    }

    /// Insert a user row and return a `User` (mirroring the pattern in connections.rs).
    async fn seed_user(pool: &DbPool, username: &str, is_root: bool) -> User {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, disabled, created_at)
             VALUES (?, ?, ?, ?, ?, 0, ?)",
        )
        .bind(&id)
        .bind(username)
        .bind("hash")
        .bind(username)
        .bind(is_root as i64)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();

        User {
            id,
            username: username.to_string(),
            display_name: username.to_string(),
            is_root,
            disabled: false,
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn no_row_means_none() {
        let pool = mem_pool().await;
        let repo = GrantsRepo::new(pool.clone());
        let u = seed_user(&pool, "alice", false).await;
        assert_eq!(
            repo.capability_of(&u, Feature::Database).await.unwrap(),
            Capability::None
        );
    }

    #[tokio::test]
    async fn root_is_admin_everywhere_without_rows() {
        let pool = mem_pool().await;
        let repo = GrantsRepo::new(pool.clone());
        let root = seed_user(&pool, "root", true).await;
        assert_eq!(
            repo.capability_of(&root, Feature::Settings).await.unwrap(),
            Capability::Admin
        );
    }

    #[tokio::test]
    async fn set_and_read_grants() {
        let pool = mem_pool().await;
        let repo = GrantsRepo::new(pool.clone());
        let u = seed_user(&pool, "bob", false).await;

        repo.set_grants(
            &u.id,
            &[
                (Feature::Database, Capability::View),
                (Feature::Connections, Capability::Edit),
            ],
        )
        .await
        .unwrap();

        assert_eq!(
            repo.capability_of(&u, Feature::Database).await.unwrap(),
            Capability::View
        );
        assert_eq!(
            repo.capability_of(&u, Feature::Agents).await.unwrap(),
            Capability::None
        );
    }

    #[tokio::test]
    async fn set_grants_replaces_atomically() {
        let pool = mem_pool().await;
        let repo = GrantsRepo::new(pool.clone());
        let u = seed_user(&pool, "carol", false).await;

        // Initial grants.
        repo.set_grants(
            &u.id,
            &[
                (Feature::Database, Capability::Admin),
                (Feature::Git, Capability::View),
            ],
        )
        .await
        .unwrap();

        // Replace with a different set — Git should disappear, Database downgraded.
        repo.set_grants(&u.id, &[(Feature::Database, Capability::View)])
            .await
            .unwrap();

        assert_eq!(
            repo.capability_of(&u, Feature::Database).await.unwrap(),
            Capability::View
        );
        assert_eq!(
            repo.capability_of(&u, Feature::Git).await.unwrap(),
            Capability::None
        );
    }

    #[tokio::test]
    async fn grants_for_returns_all() {
        let pool = mem_pool().await;
        let repo = GrantsRepo::new(pool.clone());
        let u = seed_user(&pool, "dave", false).await;

        repo.set_grants(
            &u.id,
            &[
                (Feature::Connections, Capability::Edit),
                (Feature::Database, Capability::View),
            ],
        )
        .await
        .unwrap();

        let grants = repo.grants_for(&u.id).await.unwrap();
        assert_eq!(grants.len(), 2);
        // Sorted by feature text: "connections" < "database"
        assert_eq!(grants[0], (Feature::Connections, Capability::Edit));
        assert_eq!(grants[1], (Feature::Database, Capability::View));
    }

    // ── SG-11 grant cache ──────────────────────────────────────────────────

    #[tokio::test]
    async fn cached_repo_answers_every_feature_from_one_load() {
        let pool = mem_pool().await;
        let user = seed_user(&pool, "cached", false).await;
        let cache = GrantCache::new();
        let repo = GrantsRepo::new(pool.clone()).with_cache(cache.clone());
        repo.set_grants(&user.id, &[(Feature::Git, Capability::Edit)])
            .await
            .unwrap();
        assert_eq!(
            repo.capability_of(&user, Feature::Git).await.unwrap(),
            Capability::Edit
        );
        assert_eq!(
            repo.capability_of(&user, Feature::Agents).await.unwrap(),
            Capability::None
        );
        assert_eq!(
            repo.capability_of(&user, Feature::Git).await.unwrap(),
            Capability::Edit
        );
        assert_eq!(cache.loads(), 1, "one row load answers every feature");
    }

    #[tokio::test]
    async fn set_grants_evicts_so_a_revocation_is_immediate() {
        let pool = mem_pool().await;
        let user = seed_user(&pool, "revoked", false).await;
        let cache = GrantCache::new();
        let repo = GrantsRepo::new(pool.clone()).with_cache(cache.clone());
        repo.set_grants(&user.id, &[(Feature::Git, Capability::Admin)])
            .await
            .unwrap();
        assert_eq!(
            repo.capability_of(&user, Feature::Git).await.unwrap(),
            Capability::Admin
        );
        // A DIFFERENT repo handle sharing the cache (the grants route builds its
        // own) revokes: the guard's next lookup must see it, not the cache.
        let admin = GrantsRepo::new(pool.clone()).with_cache(cache.clone());
        admin.set_grants(&user.id, &[]).await.unwrap();
        assert_eq!(
            repo.capability_of(&user, Feature::Git).await.unwrap(),
            Capability::None
        );
        admin
            .set_plugin_grants(&user.id, &[("deploy".into(), Capability::View)])
            .await
            .unwrap();
        assert_eq!(
            repo.capability_of_plugin(&user, "deploy").await.unwrap(),
            Capability::View
        );
        admin.set_plugin_grants(&user.id, &[]).await.unwrap();
        assert_eq!(
            repo.capability_of_plugin(&user, "deploy").await.unwrap(),
            Capability::None
        );
    }

    #[tokio::test]
    async fn a_fill_racing_an_invalidation_is_not_cached() {
        let pool = mem_pool().await;
        let user = seed_user(&pool, "racer", false).await;
        let cache = GrantCache::new();
        let started = cache.generation();
        cache.invalidate_user(&user.id); // a grant change lands mid-fill
        cache.store(
            &user.id,
            started,
            HashMap::from([("git".to_string(), "admin".to_string())]),
            None,
        );
        assert!(
            cache.lookup(&user.id, "git", false).is_none(),
            "stale fill dropped"
        );
        cache.store(&user.id, cache.generation(), HashMap::new(), None);
        assert_eq!(
            cache.lookup(&user.id, "git", false),
            Some(None),
            "a clean fill is kept"
        );
        cache.clear();
        assert!(cache.lookup(&user.id, "git", false).is_none());
    }

    #[tokio::test]
    async fn root_never_touches_the_cache_and_uncached_repo_is_unchanged() {
        let pool = mem_pool().await;
        let root = seed_user(&pool, "root2", true).await;
        let cache = GrantCache::new();
        let repo = GrantsRepo::new(pool.clone()).with_cache(cache.clone());
        assert_eq!(
            repo.capability_of(&root, Feature::Git).await.unwrap(),
            Capability::Admin
        );
        assert_eq!(cache.loads(), 0);
        let plain = GrantsRepo::new(pool.clone());
        let user = seed_user(&pool, "plain", false).await;
        plain
            .set_grants(&user.id, &[(Feature::Git, Capability::View)])
            .await
            .unwrap();
        assert_eq!(
            plain.capability_of(&user, Feature::Git).await.unwrap(),
            Capability::View
        );
    }
}
