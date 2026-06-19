//! Bearer-token auth sessions over the `auth_sessions` table.
//!
//! Tokens are 32 random bytes hex-encoded (64 chars); only the SHA-256 hex of
//! the token is stored. Expiry is sliding: 30 days from last_seen, refreshed
//! at most once per hour to throttle writes.

use chrono::{DateTime, Duration, Utc};
use otto_core::api::ApiTokenInfo;
use otto_core::domain::User;
use otto_core::{new_id, Error, Id, Result};
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

/// Sliding expiry window for interactive (`kind='session'`) login tokens.
const TOKEN_TTL_DAYS: i64 = 30;
/// Fixed lifetime for `kind='api'` personal access tokens (~10 years). Long
/// enough to behave as "create once"; the expiry is never slid for these.
const API_TOKEN_TTL_DAYS: i64 = 3650;
/// Minimum age of `last_seen_at` before we touch the row again (throttles
/// writes; for session tokens this also slides the expiry).
const TOUCH_THROTTLE_SECS: i64 = 3600;

/// SHA-256 hex of a raw token string.
pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn parse_ts(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| Error::Internal(format!("bad timestamp '{s}': {e}")))
}

/// Repository for `auth_sessions`.
#[derive(Clone)]
pub struct AuthRepo {
    pool: SqlitePool,
}

impl AuthRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Issue a new token for `user_id` and return the RAW token (the only
    /// time it exists in plaintext).
    pub async fn issue(&self, user_id: &Id) -> Result<String> {
        let mut buf = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut buf);
        let token = hex::encode(buf);
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO auth_sessions (id, user_id, token_hash, created_at, expires_at, last_seen_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(new_id())
        .bind(user_id)
        .bind(token_hash(&token))
        .bind(now.to_rfc3339())
        .bind((now + Duration::days(TOKEN_TTL_DAYS)).to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("issue token: {e}")))?;
        Ok(token)
    }

    /// Validate a raw token: lookup by hash, check expiry and that the user
    /// is not disabled, then slide expiry (throttled to once per hour).
    pub async fn authenticate(&self, token: &str) -> Result<User> {
        let hash = token_hash(token);
        let row = sqlx::query(
            "SELECT a.token_hash, a.expires_at, a.last_seen_at, a.kind,
                    u.id, u.username, u.display_name, u.is_root, u.disabled, u.created_at
             FROM auth_sessions a JOIN users u ON u.id = a.user_id
             WHERE a.token_hash = ?",
        )
        .bind(&hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("authenticate: {e}")))?
        .ok_or(Error::Unauthorized)?;

        let now = Utc::now();
        let expires_at = parse_ts(&row.get::<String, _>("expires_at"))?;
        if expires_at <= now {
            return Err(Error::Unauthorized);
        }
        if row.get::<i64, _>("disabled") != 0 {
            return Err(Error::Unauthorized);
        }

        let last_seen = parse_ts(&row.get::<String, _>("last_seen_at"))?;
        if now - last_seen > Duration::seconds(TOUCH_THROTTLE_SECS) {
            // Always record "last used"; only SLIDE the expiry for interactive
            // session tokens. API tokens keep their long fixed lifetime so they
            // never silently expire from regular use.
            let is_api = row.get::<String, _>("kind") == "api";
            if is_api {
                sqlx::query("UPDATE auth_sessions SET last_seen_at = ? WHERE token_hash = ?")
                    .bind(now.to_rfc3339())
                    .bind(&hash)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| Error::Internal(format!("touch token: {e}")))?;
            } else {
                sqlx::query(
                    "UPDATE auth_sessions SET last_seen_at = ?, expires_at = ? WHERE token_hash = ?",
                )
                .bind(now.to_rfc3339())
                .bind((now + Duration::days(TOKEN_TTL_DAYS)).to_rfc3339())
                .bind(&hash)
                .execute(&self.pool)
                .await
                .map_err(|e| Error::Internal(format!("touch token: {e}")))?;
            }
        }

        Ok(User {
            id: row.get("id"),
            username: row.get("username"),
            display_name: row.get("display_name"),
            is_root: row.get::<i64, _>("is_root") != 0,
            disabled: false,
            created_at: parse_ts(&row.get::<String, _>("created_at"))?,
        })
    }

    /// Revoke (delete) the auth session matching `token`. Idempotent.
    pub async fn revoke(&self, token: &str) -> Result<()> {
        sqlx::query("DELETE FROM auth_sessions WHERE token_hash = ?")
            .bind(token_hash(token))
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Internal(format!("revoke token: {e}")))?;
        Ok(())
    }

    /// Revoke ALL of `user_id`'s sessions — both interactive login tokens and
    /// long-lived API tokens. Used when a credential changes (password reset) or
    /// the account is disabled, so every previously-issued token is invalidated.
    /// Returns the number of sessions deleted.
    pub async fn revoke_all_for_user(&self, user_id: &Id) -> Result<u64> {
        let res = sqlx::query("DELETE FROM auth_sessions WHERE user_id = ?")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Internal(format!("revoke all for user: {e}")))?;
        Ok(res.rows_affected())
    }

    /// Mint a long-lived API (personal access) token for `user_id`. Returns the
    /// RAW token (shown to the caller exactly once) plus its metadata.
    pub async fn issue_api_token(
        &self,
        user_id: &Id,
        label: Option<&str>,
    ) -> Result<(String, ApiTokenInfo)> {
        let mut buf = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut buf);
        let token = hex::encode(buf);
        let prefix: String = token.chars().take(12).collect();
        let id = new_id();
        let now = Utc::now();
        let expires_at = now + Duration::days(API_TOKEN_TTL_DAYS);
        sqlx::query(
            "INSERT INTO auth_sessions
               (id, user_id, token_hash, created_at, expires_at, last_seen_at, kind, label, token_prefix)
             VALUES (?, ?, ?, ?, ?, ?, 'api', ?, ?)",
        )
        .bind(&id)
        .bind(user_id)
        .bind(token_hash(&token))
        .bind(now.to_rfc3339())
        .bind(expires_at.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(label)
        .bind(&prefix)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("issue api token: {e}")))?;
        Ok((
            token,
            ApiTokenInfo {
                id,
                label: label.map(str::to_owned),
                token_prefix: prefix,
                created_at: now,
                last_seen_at: now,
                expires_at,
            },
        ))
    }

    /// List a user's API tokens (newest first). Never includes the secret.
    pub async fn list_api_tokens(&self, user_id: &Id) -> Result<Vec<ApiTokenInfo>> {
        let rows = sqlx::query(
            "SELECT id, label, token_prefix, created_at, last_seen_at, expires_at
             FROM auth_sessions
             WHERE user_id = ? AND kind = 'api'
             ORDER BY created_at DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("list api tokens: {e}")))?;

        rows.into_iter()
            .map(|row| {
                Ok(ApiTokenInfo {
                    id: row.get("id"),
                    label: row.get("label"),
                    token_prefix: row.get("token_prefix"),
                    created_at: parse_ts(&row.get::<String, _>("created_at"))?,
                    last_seen_at: parse_ts(&row.get::<String, _>("last_seen_at"))?,
                    expires_at: parse_ts(&row.get::<String, _>("expires_at"))?,
                })
            })
            .collect()
    }

    /// Revoke one of `user_id`'s API tokens by id. Returns whether a row was
    /// deleted (false = not found / not owned / not an API token).
    pub async fn revoke_api_token(&self, user_id: &Id, id: &Id) -> Result<bool> {
        let res = sqlx::query(
            "DELETE FROM auth_sessions WHERE id = ? AND user_id = ? AND kind = 'api'",
        )
        .bind(id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("revoke api token: {e}")))?;
        Ok(res.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::new_id;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    // In-memory pool helper — mirrors otto-state's test setup. The migrations
    // live in otto-state, so reference them by relative path (the `sqlx::migrate!`
    // macro embeds them at compile time).
    async fn mem_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .unwrap();
        pool
    }

    /// Seed a minimal (non-root, enabled) user and return its id.
    async fn seed_user(pool: &SqlitePool, username: &str) -> Id {
        let id = new_id();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
             VALUES (?, ?, ?, ?, 0, ?)",
        )
        .bind(&id)
        .bind(username)
        .bind("hash")
        .bind("Test User")
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        id
    }

    /// Interactive login token: mint → authenticate → revoke → auth fails.
    #[tokio::test]
    async fn session_token_lifecycle() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let uid = seed_user(&pool, "alice").await;

        let token = repo.issue(&uid).await.unwrap();
        let user = repo.authenticate(&token).await.unwrap();
        assert_eq!(user.id, uid);

        repo.revoke(&token).await.unwrap();
        assert!(
            matches!(repo.authenticate(&token).await, Err(Error::Unauthorized)),
            "revoked session token must no longer authenticate"
        );
    }

    /// API (personal access) token: mint → appears in list (no secret) →
    /// authenticate → revoke by id → auth fails.
    #[tokio::test]
    async fn api_token_lifecycle_mint_list_auth_revoke() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let uid = seed_user(&pool, "bob").await;

        let (token, info) = repo.issue_api_token(&uid, Some("cli")).await.unwrap();
        assert_eq!(info.label.as_deref(), Some("cli"));
        // The prefix is the first 12 chars of the raw token, never the rest.
        assert_eq!(info.token_prefix, token.chars().take(12).collect::<String>());

        // It shows up in the user's token list (metadata only, never the secret).
        let listed = repo.list_api_tokens(&uid).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, info.id);
        assert_eq!(listed[0].token_prefix, info.token_prefix);

        // It authenticates like any bearer token.
        let user = repo.authenticate(&token).await.unwrap();
        assert_eq!(user.id, uid);

        // Revoking by id invalidates it; the list goes empty.
        assert!(repo.revoke_api_token(&uid, &info.id).await.unwrap());
        assert!(
            matches!(repo.authenticate(&token).await, Err(Error::Unauthorized)),
            "revoked API token must no longer authenticate"
        );
        assert!(repo.list_api_tokens(&uid).await.unwrap().is_empty());
        // A second revoke of the same id is a no-op (returns false).
        assert!(!repo.revoke_api_token(&uid, &info.id).await.unwrap());
    }

    /// `revoke_api_token` is scoped to the owner: it must not delete another
    /// user's token, and that token keeps authenticating.
    #[tokio::test]
    async fn api_token_revoke_is_owner_scoped() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let owner = seed_user(&pool, "owner").await;
        let other = seed_user(&pool, "other").await;

        let (token, info) = repo.issue_api_token(&owner, Some("cli")).await.unwrap();
        // `other` cannot revoke `owner`'s token.
        assert!(!repo.revoke_api_token(&other, &info.id).await.unwrap());
        // And it still works.
        assert_eq!(repo.authenticate(&token).await.unwrap().id, owner);
    }

    /// Revocation on credential change: `revoke_all_for_user` invalidates EVERY
    /// outstanding token (session + API) for that user, but leaves other users'
    /// tokens untouched.
    #[tokio::test]
    async fn revoke_all_for_user_invalidates_every_token() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let victim = seed_user(&pool, "victim").await;
        let bystander = seed_user(&pool, "bystander").await;

        // Victim has a session token and two API tokens; all authenticate.
        let session = repo.issue(&victim).await.unwrap();
        let (api1, _) = repo.issue_api_token(&victim, Some("ci")).await.unwrap();
        let (api2, _) = repo.issue_api_token(&victim, None).await.unwrap();
        let bystander_token = repo.issue(&bystander).await.unwrap();
        for t in [&session, &api1, &api2, &bystander_token] {
            assert!(repo.authenticate(t).await.is_ok());
        }

        // A credential change wipes all of the victim's sessions.
        let deleted = repo.revoke_all_for_user(&victim).await.unwrap();
        assert_eq!(deleted, 3, "session + 2 API tokens revoked");

        for t in [&session, &api1, &api2] {
            assert!(
                matches!(repo.authenticate(t).await, Err(Error::Unauthorized)),
                "every victim token must be invalid after revoke_all_for_user"
            );
        }
        // The bystander's token is unaffected.
        assert!(repo.authenticate(&bystander_token).await.is_ok());
        assert!(repo.list_api_tokens(&victim).await.unwrap().is_empty());
    }
}
