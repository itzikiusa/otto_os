//! Storage-path test for the email-sender route (mobile plan Task 7.1).
//!
//! The `PUT /api/v1/email-sender` handler does three storage things before it
//! touches the network: (1) writes the Gmail App Password to the `SecretStore`
//! (Keychain in prod) under `email-sender-{user_id}`, (2) upserts the
//! `email_senders` row carrying only that opaque `secret_ref` (never the
//! password), and (3) on a successful SMTP login, flips `verified_at`.
//!
//! This test exercises exactly that storage sequence with an **in-memory**
//! `SecretStore` (so no real Keychain is needed in CI) and asserts the security
//! contract: the password lives in the secret store, the DB holds only the ref,
//! and the row never contains the password.
//!
//! The real `GmailSender::verify()` SMTP login is deliberately NOT exercised here
//! — it requires a live Gmail App Password + network. That path is covered by the
//! `otto-channels::email` unit tests (message/transport construction, network
//! free) and manually / in the consolidated E2E. The handler gates `set_verified`
//! behind that verify, so failing to reach Gmail leaves the row unverified — the
//! same fail-closed behavior this test asserts for the pre-verify state.

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::Utc;
use otto_core::secrets::SecretStore;
use otto_core::{new_id, Result};
use otto_state::{EmailSendersRepo, SqlitePool};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Row;

/// Minimal in-memory SecretStore (mirrors the `NullSecrets` pattern in
/// `otto-connections/tests/isolation.rs`, but records values so we can assert
/// the password landed in the store rather than the DB).
#[derive(Default)]
struct MemSecrets {
    map: Mutex<HashMap<String, String>>,
}

impl SecretStore for MemSecrets {
    fn put(&self, key: &str, value: &str) -> Result<()> {
        self.map.lock().unwrap().insert(key.to_string(), value.to_string());
        Ok(())
    }
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.map.lock().unwrap().get(key).cloned())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.map.lock().unwrap().remove(key);
        Ok(())
    }
}

/// The handler's Keychain-ref scheme. Kept in sync with
/// `routes::email_sender::secret_ref_for`.
fn secret_ref_for(user_id: &str) -> String {
    format!("email-sender-{user_id}")
}

async fn mk_pool() -> SqlitePool {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect sqlite");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("migrations");
    pool
}

async fn seed_user(pool: &SqlitePool, username: &str) -> String {
    let id = new_id();
    let now = Utc::now().to_rfc3339();
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

/// Replays the handler's storage sequence: `secrets.put` then `repo.upsert`.
/// Asserts the app password lives in the secret store under the per-user ref and
/// that the DB row carries only the opaque `secret_ref` — never the password.
#[tokio::test]
async fn put_stores_password_in_keychain_not_db() {
    let pool = mk_pool().await;
    let secrets = MemSecrets::default();
    let repo = EmailSendersRepo::new(pool.clone());

    let uid = seed_user(&pool, "alice").await;
    let app_password = "abcd efgh ijkl mnop"; // a fake 16-char-style app password
    let gmail = "alice@gmail.com";
    let secret_ref = secret_ref_for(&uid);

    // --- handler steps 1 + 2 (pre-verify) ---
    secrets.put(&secret_ref, app_password).unwrap();
    repo.upsert(&uid, gmail, &secret_ref).await.unwrap();

    // The password is in the secret store under the expected ref.
    assert_eq!(secrets.get(&secret_ref).unwrap().as_deref(), Some(app_password));
    assert_eq!(secret_ref, format!("email-sender-{uid}"));

    // The DB row holds only the ref + address, never the password.
    let row = sqlx::query("SELECT gmail_address, secret_ref, verified_at FROM email_senders WHERE user_id = ?")
        .bind(&uid)
        .fetch_one(&pool)
        .await
        .unwrap();
    let stored_addr: String = row.get("gmail_address");
    let stored_ref: String = row.get("secret_ref");
    let verified_at: Option<i64> = row.get("verified_at");
    assert_eq!(stored_addr, gmail);
    assert_eq!(stored_ref, secret_ref);
    assert!(verified_at.is_none(), "row is unverified until SMTP verify succeeds");

    // Defense in depth: the raw password must NOT appear anywhere in the row.
    assert_ne!(stored_ref, app_password);
    assert_ne!(stored_addr, app_password);

    // --- handler step 3 (only after a successful verify) ---
    let when = Utc::now();
    repo.set_verified(&uid, when).await.unwrap();
    let got = repo.get(&uid).await.unwrap().expect("sender");
    assert_eq!(got.verified_at.map(|t| t.timestamp()), Some(when.timestamp()));
    // The repo's typed view never exposes a password field — only the ref.
    assert_eq!(got.secret_ref, secret_ref);
}

/// Re-configuring resets verification (the new address/password is unproven).
#[tokio::test]
async fn reconfigure_resets_verification() {
    let pool = mk_pool().await;
    let repo = EmailSendersRepo::new(pool.clone());
    let uid = seed_user(&pool, "bob").await;

    repo.upsert(&uid, "old@gmail.com", &secret_ref_for(&uid)).await.unwrap();
    repo.set_verified(&uid, Utc::now()).await.unwrap();
    assert!(repo.get(&uid).await.unwrap().unwrap().verified_at.is_some());

    repo.upsert(&uid, "new@gmail.com", &secret_ref_for(&uid)).await.unwrap();
    assert!(
        repo.get(&uid).await.unwrap().unwrap().verified_at.is_none(),
        "re-config must clear verified_at"
    );
}
