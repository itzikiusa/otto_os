//! Per-user email-sender repository (mobile plan Task 7.1).
//!
//! One row per user: the Gmail address they send OTP mail from, plus an opaque
//! `secret_ref` pointing at the Gmail **App Password** that lives in the macOS
//! Keychain (`otto-keychain`) — **never** the password itself. `verified_at` is
//! set once a real SMTP login with that app password succeeds. This mirrors the
//! secret-indirection pattern `ConnectionsService` uses (`conn-{id}` refs).
//!
//! The route layer (`PUT /api/v1/email-sender`) owns the Keychain write and the
//! SMTP verify; this repo only persists the metadata + the ref.

use chrono::{DateTime, Utc};
use otto_core::{Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::dberr;

/// A configured email sender. Carries only the `secret_ref` (Keychain key), so
/// this struct NEVER holds the app password — the real secret stays in the
/// Keychain and is fetched on demand by the sender.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailSender {
    pub user_id: Id,
    pub gmail_address: String,
    /// Keychain reference (e.g. `email-sender-{user_id}`), NOT the password.
    pub secret_ref: String,
    /// `Some` once a real SMTP login with the app password succeeded.
    pub verified_at: Option<DateTime<Utc>>,
}

#[derive(Clone)]
pub struct EmailSendersRepo {
    pool: SqlitePool,
}

fn row_to_sender(r: &sqlx::sqlite::SqliteRow) -> Result<EmailSender> {
    // `verified_at` is stored as a unix-seconds INTEGER (nullable) per the
    // migration; convert to a UTC timestamp when present.
    let verified_at = r
        .get::<Option<i64>, _>("verified_at")
        .and_then(|s| DateTime::<Utc>::from_timestamp(s, 0));
    Ok(EmailSender {
        user_id: r.get("user_id"),
        gmail_address: r.get("gmail_address"),
        secret_ref: r.get("secret_ref"),
        verified_at,
    })
}

impl EmailSendersRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Insert or replace the sender for `user_id`.
    ///
    /// An upsert on the `user_id` PK. **Re-configuring resets verification** —
    /// `verified_at` is cleared, because the new address / app password has not
    /// yet been proven via SMTP. The caller sets it again via [`set_verified`]
    /// after a successful login.
    pub async fn upsert(&self, user_id: &str, gmail_address: &str, secret_ref: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO email_senders (user_id, gmail_address, secret_ref, verified_at)
             VALUES (?, ?, ?, NULL)
             ON CONFLICT(user_id) DO UPDATE SET
               gmail_address = excluded.gmail_address,
               secret_ref    = excluded.secret_ref,
               verified_at   = NULL",
        )
        .bind(user_id)
        .bind(gmail_address)
        .bind(secret_ref)
        .execute(&self.pool)
        .await
        .map_err(dberr("email_senders.upsert"))?;
        Ok(())
    }

    /// Fetch the sender for `user_id`, or `None` if not configured.
    pub async fn get(&self, user_id: &str) -> Result<Option<EmailSender>> {
        let row = sqlx::query(
            "SELECT user_id, gmail_address, secret_ref, verified_at
             FROM email_senders WHERE user_id = ?",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("email_senders.get"))?;
        match row {
            Some(r) => Ok(Some(row_to_sender(&r)?)),
            None => Ok(None),
        }
    }

    /// Mark the sender verified at `ts` (unix seconds). Called after a real SMTP
    /// login with the configured app password succeeds.
    pub async fn set_verified(&self, user_id: &str, ts: DateTime<Utc>) -> Result<()> {
        sqlx::query("UPDATE email_senders SET verified_at = ? WHERE user_id = ?")
            .bind(ts.timestamp())
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("email_senders.set_verified"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::fmt;
    use otto_core::new_id;

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

    /// Insert a user row and return its id (FK target for `email_senders`).
    async fn seed_user(pool: &SqlitePool, username: &str) -> Id {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, disabled, created_at)
             VALUES (?, ?, ?, ?, 0, 0, ?)",
        )
        .bind(&id)
        .bind(username)
        .bind("hash")
        .bind(username)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        id
    }

    #[tokio::test]
    async fn get_missing_is_none() {
        let pool = mem_pool().await;
        let repo = EmailSendersRepo::new(pool.clone());
        let uid = seed_user(&pool, "alice").await;
        assert!(repo.get(&uid).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn upsert_then_get_roundtrips_without_password() {
        let pool = mem_pool().await;
        let repo = EmailSendersRepo::new(pool.clone());
        let uid = seed_user(&pool, "bob").await;

        repo.upsert(&uid, "bob@gmail.com", "email-sender-secret-ref")
            .await
            .unwrap();

        let got = repo.get(&uid).await.unwrap().expect("sender");
        assert_eq!(got.user_id, uid);
        assert_eq!(got.gmail_address, "bob@gmail.com");
        // Only the opaque ref is stored — never the app password.
        assert_eq!(got.secret_ref, "email-sender-secret-ref");
        assert!(got.verified_at.is_none(), "fresh upsert is unverified");
    }

    #[tokio::test]
    async fn set_verified_records_timestamp() {
        let pool = mem_pool().await;
        let repo = EmailSendersRepo::new(pool.clone());
        let uid = seed_user(&pool, "carol").await;
        repo.upsert(&uid, "carol@gmail.com", "ref").await.unwrap();

        let when = Utc::now();
        repo.set_verified(&uid, when).await.unwrap();

        let got = repo.get(&uid).await.unwrap().expect("sender");
        assert_eq!(
            got.verified_at.map(|t| t.timestamp()),
            Some(when.timestamp())
        );
    }

    #[tokio::test]
    async fn upsert_replaces_and_resets_verification() {
        let pool = mem_pool().await;
        let repo = EmailSendersRepo::new(pool.clone());
        let uid = seed_user(&pool, "dave").await;

        repo.upsert(&uid, "old@gmail.com", "ref-old").await.unwrap();
        repo.set_verified(&uid, Utc::now()).await.unwrap();
        assert!(repo.get(&uid).await.unwrap().unwrap().verified_at.is_some());

        // Re-configuring with a new address resets verification.
        repo.upsert(&uid, "new@gmail.com", "ref-new").await.unwrap();
        let got = repo.get(&uid).await.unwrap().expect("sender");
        assert_eq!(got.gmail_address, "new@gmail.com");
        assert_eq!(got.secret_ref, "ref-new");
        assert!(
            got.verified_at.is_none(),
            "re-config must reset verification"
        );
    }

    #[tokio::test]
    async fn cascades_on_user_delete() {
        let pool = mem_pool().await;
        let repo = EmailSendersRepo::new(pool.clone());
        let uid = seed_user(&pool, "erin").await;
        repo.upsert(&uid, "erin@gmail.com", "ref").await.unwrap();

        sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(&uid)
            .execute(&pool)
            .await
            .unwrap();

        assert!(
            repo.get(&uid).await.unwrap().is_none(),
            "sender row should cascade-delete with the user"
        );
    }
}
