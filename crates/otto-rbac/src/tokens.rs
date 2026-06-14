//! Bearer-token auth sessions over the `auth_sessions` table.
//!
//! Tokens are 32 random bytes hex-encoded (64 chars); only the SHA-256 hex of
//! the token is stored. Expiry is sliding: 30 days from last_seen, refreshed
//! at most once per hour to throttle writes.

use chrono::{DateTime, Duration, Utc};
use otto_core::domain::User;
use otto_core::{new_id, Error, Id, Result};
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

/// Sliding expiry window.
const TOKEN_TTL_DAYS: i64 = 30;
/// Minimum age of `last_seen_at` before we slide the expiry again.
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
            "SELECT a.token_hash, a.expires_at, a.last_seen_at,
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
}
