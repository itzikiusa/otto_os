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

use std::sync::Arc;

use otto_core::auth::GrantsInvalidator;
use otto_core::domain::{Capability, Feature, User};
use otto_core::{Error, Result};
use sqlx::SqlitePool;

/// No-op [`GrantsInvalidator`] used when no cache is wired in.
struct NoopInvalidator;

impl GrantsInvalidator for NoopInvalidator {
    fn invalidate_user(&self, _user_id: &str) {}
}

#[derive(Clone)]
pub struct GrantsRepo {
    pool: SqlitePool,
    /// Called in `set_grants` after committing. The default is a no-op.
    invalidator: Arc<dyn GrantsInvalidator>,
}

impl GrantsRepo {
    /// Construct with a no-op invalidator (no auth cache). All callers that do
    /// not opt in to caching should use this constructor; behaviour is identical
    /// to the previous single-constructor API.
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            invalidator: Arc::new(NoopInvalidator),
        }
    }

    /// Construct with an explicit [`GrantsInvalidator`]. Pass the `AuthCache`
    /// from `otto-rbac` here so grant changes immediately flush the affected
    /// user's cached auth context.
    pub fn new_with_invalidator(pool: SqlitePool, inv: Arc<dyn GrantsInvalidator>) -> Self {
        Self {
            pool,
            invalidator: inv,
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

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use otto_core::new_id;

    use crate::convert::fmt;

    async fn mem_pool() -> SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool
    }

    /// Insert a user row and return a `User` (mirroring the pattern in connections.rs).
    async fn seed_user(pool: &SqlitePool, username: &str, is_root: bool) -> User {
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
}
