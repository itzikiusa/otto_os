//! Per-user Gmail App Password email sender (mobile plan Task 7.1).
//!
//! The foundation of the email-OTP share gate: a user configures ONE Gmail
//! sender (their address + a 16-char Gmail App Password). The app password is
//! stored in the macOS **Keychain** (`otto-keychain`) under a per-user ref —
//! never in the DB, which holds only the opaque `secret_ref`. This mirrors how
//! `otto-connections` stores DB/SSH secrets (`conn-{id}` refs).
//!
//! `PUT` stores the secret, upserts the row, then validates the pair via a real
//! Gmail SMTP login ([`GmailSender::verify`]); only on success is `verified_at`
//! recorded. `GET` returns the configured address + verified flag, never the
//! password.
//!
//! Both routes are **self-owned** (any authed user manages their OWN sender) and
//! therefore `Exempt` in `policy.rs`, like `/auth/tokens`.

use axum::extract::State;
use axum::Json;
use chrono::Utc;
use otto_channels::GmailSender;
use otto_core::api::{EmailSenderResp, SetEmailSenderReq};
use otto_core::Error;
use otto_state::{EmailSendersRepo, NewAuditEntry};

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Keychain reference for a user's email-sender app password. The real secret
/// lives in the Keychain under this key; the DB stores only this string.
fn secret_ref_for(user_id: &str) -> String {
    format!("email-sender-{user_id}")
}

/// `PUT /api/v1/email-sender` — configure the caller's Gmail sender.
///
/// Flow: store `app_password` in the Keychain → upsert the `email_senders` row
/// with the `secret_ref` (clearing any prior `verified_at`) → run a real SMTP
/// login via [`GmailSender::verify`]. On success, set `verified_at` and return
/// `{ gmail_address, verified: true }`. On verify failure the row stays
/// unverified and the error surfaces to the caller (the secret remains in the
/// Keychain so the user can retry without re-entering it, but the sender is not
/// usable until a verify succeeds).
pub async fn set_email_sender(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<SetEmailSenderReq>,
) -> ApiResult<Json<EmailSenderResp>> {
    let gmail_address = req.gmail_address.trim().to_string();
    if gmail_address.is_empty() {
        return Err(ApiError(Error::Invalid("gmail_address is required".into())));
    }
    if req.app_password.is_empty() {
        return Err(ApiError(Error::Invalid("app_password is required".into())));
    }

    let repo = EmailSendersRepo::new(ctx.pool.clone());
    let secret_ref = secret_ref_for(&user.id);

    // 1. Store the app password in the Keychain (never the DB).
    otto_core::secrets::put_async(&ctx.secrets, &secret_ref, &req.app_password).await?;

    // 2. Upsert the row with the Keychain ref (resets verification to NULL).
    repo.upsert(&user.id, &gmail_address, &secret_ref).await?;

    ctx.audit(NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: "email_sender.set".into(),
        target: Some(gmail_address.clone()),
        detail: None,
        ip: None,
    })
    .await;

    // 3. Validate via a real Gmail SMTP login (STARTTLS + AUTH). On failure the
    //    row stays unverified and the error is surfaced; the sender is not usable
    //    until a verify succeeds.
    let sender = GmailSender::new(gmail_address.clone(), req.app_password);
    sender.verify().await.map_err(|e| {
        // Make the failure caller-facing and actionable without leaking SMTP
        // internals or the password.
        ApiError(Error::Upstream(format!(
            "Gmail SMTP verification failed — check the address and the 16-char App Password (2-Step Verification required): {e}"
        )))
    })?;

    // 4. Verified — record the timestamp.
    let now = Utc::now();
    repo.set_verified(&user.id, now).await?;

    ctx.audit(NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: "email_sender.verified".into(),
        target: Some(gmail_address.clone()),
        detail: None,
        ip: None,
    })
    .await;

    Ok(Json(EmailSenderResp {
        gmail_address: Some(gmail_address),
        verified: true,
    }))
}

/// `POST /api/v1/email-sender/verify` — re-run the Gmail SMTP check for the
/// caller's EXISTING sender with the app password already in the Keychain.
///
/// The Settings "Re-verify" button used to `PUT` the address alone, which serde
/// rejects (`app_password` is required) — so an unverified sender could never
/// be re-checked without re-typing the password. 400 when no sender is set up
/// or its Keychain entry is gone (the user must re-enter the password); 502
/// when SMTP still fails (the row stays unverified — fail closed).
pub async fn verify_email_sender(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<EmailSenderResp>> {
    let repo = EmailSendersRepo::new(ctx.pool.clone());
    let Some(row) = repo.get(&user.id).await? else {
        return Err(ApiError(Error::Invalid(
            "no email sender is configured — set one up first".into(),
        )));
    };
    let password = otto_core::secrets::get_async(&ctx.secrets, &row.secret_ref)
        .await?
        .filter(|p| !p.is_empty())
        .ok_or_else(|| {
            ApiError(Error::Invalid(
                "the stored App Password is missing — re-enter it with Replace…".into(),
            ))
        })?;
    let sender = GmailSender::new(row.gmail_address.clone(), password);
    sender.verify().await.map_err(|e| {
        ApiError(Error::Upstream(format!(
            "Gmail SMTP verification failed — the App Password may have been revoked: {e}"
        )))
    })?;
    repo.set_verified(&user.id, Utc::now()).await?;
    ctx.audit(NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: "email_sender.verified".into(),
        target: Some(row.gmail_address.clone()),
        detail: Some("re-verify".into()),
        ip: None,
    })
    .await;
    Ok(Json(EmailSenderResp {
        gmail_address: Some(row.gmail_address),
        verified: true,
    }))
}

/// `GET /api/v1/email-sender` — the caller's configured sender, never the
/// password. `gmail_address` is absent when no sender is set up.
pub async fn get_email_sender(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<EmailSenderResp>> {
    let repo = EmailSendersRepo::new(ctx.pool.clone());
    let sender = repo.get(&user.id).await?;
    Ok(Json(match sender {
        Some(s) => EmailSenderResp {
            gmail_address: Some(s.gmail_address),
            verified: s.verified_at.is_some(),
        },
        None => EmailSenderResp {
            gmail_address: None,
            verified: false,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::domain::User;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    async fn ctx_with_user() -> (tempfile::TempDir, ServerCtx, User) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .in_memory(true)
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .unwrap();
        let pool: otto_state::DbPool = pool.into();
        let user = User {
            id: "u1".into(),
            username: "u1".into(),
            display_name: "u1".into(),
            is_root: false,
            disabled: false,
            created_at: Utc::now(),
        };
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, disabled, created_at)
             VALUES ('u1', 'u1', 'h', 'u1', 0, 0, ?)",
        )
        .bind(Utc::now().to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let ctx = ServerCtx::for_tests(&pool, tmp.path()).await;
        (tmp, ctx, user)
    }

    /// S17-05: re-verify needs no password in the body, and fails closed
    /// (400, never `verified`) when there's no sender or no stored secret.
    #[tokio::test]
    async fn reverify_without_sender_or_secret_is_invalid_and_stays_unverified() {
        let (_tmp, ctx, user) = ctx_with_user().await;
        let err = verify_email_sender(State(ctx.clone()), CurrentUser(user.clone()))
            .await
            .unwrap_err();
        assert!(matches!(err.0, Error::Invalid(_)), "{err:?}");

        // A row whose Keychain entry is gone → Invalid, row stays unverified.
        let repo = EmailSendersRepo::new(ctx.pool.clone());
        repo.upsert(&user.id, "u1@gmail.com", &secret_ref_for(&user.id))
            .await
            .unwrap();
        let err = verify_email_sender(State(ctx.clone()), CurrentUser(user.clone()))
            .await
            .unwrap_err();
        assert!(matches!(err.0, Error::Invalid(_)), "{err:?}");
        assert!(repo
            .get(&user.id)
            .await
            .unwrap()
            .unwrap()
            .verified_at
            .is_none());
    }
}
