//! Audited admin impersonation with anti-escalation guardrails — RBAC Task 5.2.
//!
//! ## Endpoints
//! - `POST /api/v1/admin/impersonate/{user_id}` — mint a short-lived
//!   **impersonation token** for the caller acting as `{user_id}`; returns
//!   `{ token }`. Audited `impersonate.start` (real + target).
//! - `POST /api/v1/admin/impersonate/stop` — revoke the presented impersonation
//!   token. Audited `impersonate.stop`.
//!
//! ## Mechanism (effective-user overlay)
//! The admin does **not** re-login. Start mints an `auth_sessions` row of
//! `kind='impersonation'` whose `user_id` is the admin (real) and
//! `acting_as_user_id` is the target (effective). `AuthRepo::authenticate`
//! resolves that into `AuthContext{ real_user: admin, effective_user: target }`,
//! so **every authorization decision runs against the target** while **every
//! audit entry records the admin** (the design-spec §6 invariant). The UI swaps
//! its bearer to the returned token; `stop` revokes it and the UI restores the
//! admin's own token.
//!
//! ## Guardrails (enforced in [`start`] — each has a test in
//! `tests/impersonation.rs`)
//! 1. **Caller authority** — root OR `Users:Admin`. Enforced by the central
//!    feature guard (`policy.rs` maps `/admin/impersonate*` → `Require(Users,
//!    Admin)`); we rely on it and add no extra `require_root`.
//! 2. **No impersonating up/sideways** — reject if the target `is_root` OR holds
//!    `Users:Admin` (`capability_of(target, Users) == Admin`). Prevents privilege
//!    laundering. → 403.
//! 3. **No nesting** — reject if the CALLER is already impersonating
//!    (`real_user().id != effective_user().id`): an impersonation token cannot
//!    start another impersonation. → 403.
//! 4. **Disabled / absent target** — absent ⇒ 404, disabled ⇒ 403.
//! 5. **Self-impersonation** (target == caller) ⇒ 403 (pointless / confusing).
//! 6. **Short fixed TTL** — from the mint (`IMPERSONATION_TOKEN_TTL_MINS`); never
//!    slid (see `AuthRepo::authenticate`).
//!
//! Guardrail "impersonation token cannot mint PATs" lives in the PAT-mint handler
//! (`routes/auth_routes.rs::create_token`), which rejects an impersonated request.
//!
//! ## Generics
//! Handlers are generic over `ImpersonateCtx` so they run against the production
//! `ServerCtx` and a minimal test state, mirroring the `GrantsCtx` /
//! `AdminSessionsCtx` pattern used elsewhere in the RBAC work.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Duration;
use otto_core::api::ImpersonateResp;
use otto_core::domain::{Capability, Feature, User};
use otto_core::{Error, Id};
use otto_rbac::{AuthRepo, IMPERSONATION_TOKEN_TTL_MINS};
use otto_state::{GrantsRepo, NewAuditEntry, UsersRepo};

use crate::auth::{BearerToken, CurrentAuthContext};
use crate::error::ApiResult;
use crate::state::ServerCtx;

// ---------------------------------------------------------------------------
// ImpersonateCtx trait — lets the handlers run against a minimal test state
// without assembling the full ServerCtx (mirrors GrantsCtx / AdminSessionsCtx).
// ---------------------------------------------------------------------------

/// State the impersonation handlers work against. Implemented for the
/// production [`ServerCtx`] and for a minimal test context.
pub trait ImpersonateCtx: Clone + Send + Sync + 'static {
    fn auth_repo(&self) -> AuthRepo;
    fn grants_repo(&self) -> GrantsRepo;
    fn users_repo(&self) -> UsersRepo;
    /// Write a best-effort audit entry (failure logged, not propagated).
    fn audit_entry(&self, entry: NewAuditEntry) -> impl std::future::Future<Output = ()> + Send;
}

impl ImpersonateCtx for ServerCtx {
    fn auth_repo(&self) -> AuthRepo {
        AuthRepo::new(self.pool.clone())
    }
    fn grants_repo(&self) -> GrantsRepo {
        GrantsRepo::new(self.pool.clone())
    }
    fn users_repo(&self) -> UsersRepo {
        UsersRepo::new(self.pool.clone())
    }
    async fn audit_entry(&self, entry: NewAuditEntry) {
        self.audit(entry).await;
    }
}

// ---------------------------------------------------------------------------
// Handlers (generic over ImpersonateCtx)
// ---------------------------------------------------------------------------

/// `POST /api/v1/admin/impersonate/{user_id}`
///
/// Mint an impersonation token for the caller acting as `{user_id}`. Returns
/// `{ token }`. Requires `Users:Admin` (feature guard) or root; the handler then
/// enforces the anti-escalation guardrails (see module docs) and audits
/// `impersonate.start` with both the real and effective ids.
pub async fn start<C: ImpersonateCtx>(
    Path(target_id): Path<Id>,
    State(ctx): State<C>,
    auth: CurrentAuthContext,
) -> ApiResult<Json<ImpersonateResp>> {
    let real = auth.real_user().clone();
    let effective = auth.effective_user();

    // Guardrail 3 — NO NESTING. If real != effective the caller is *already*
    // impersonating (their bearer is an impersonation token); an impersonation
    // token may not start another impersonation.
    if real.id != effective.id {
        return Err(Error::Forbidden("cannot impersonate while impersonating".into()).into());
    }

    // Guardrail 4 — target must exist (404 if not).
    let target: User = ctx.users_repo().get(&target_id).await?;

    // Guardrail 5 — self-impersonation is pointless / confusing.
    if target.id == real.id {
        return Err(Error::Forbidden("cannot impersonate yourself".into()).into());
    }

    // Guardrail 4 — disabled target is rejected.
    if target.disabled {
        return Err(Error::Forbidden("target user is disabled".into()).into());
    }

    // Guardrail 2 — NEVER impersonate UP or SIDEWAYS into admin. Refuse root and
    // any user holding `Users:Admin` (so an admin can't launder privilege by
    // acting as a fellow admin / root).
    if target.is_root {
        return Err(Error::Forbidden("cannot impersonate the root user".into()).into());
    }
    let target_users_cap = ctx
        .grants_repo()
        .capability_of(&target, Feature::Users)
        .await?;
    if target_users_cap >= Capability::Admin {
        return Err(Error::Forbidden("cannot impersonate a Users-admin".into()).into());
    }

    // TODO(mobile): reject when ctx has a share scope (guest tokens may never
    // impersonate). `scope` does not exist until the mobile feature lands.

    // Guardrail 6 — SHORT FIXED TTL (never slid in `authenticate`). The token's
    // `user_id` is the admin (real); `acting_as_user_id` is the target.
    let token = ctx
        .auth_repo()
        .issue_impersonation_token(
            &real.id,
            &target.id,
            Duration::minutes(IMPERSONATION_TOKEN_TTL_MINS),
        )
        .await?;

    // Audit: the actor is the REAL user (the admin); the target identity is
    // recorded in both `target` and the detail so the ledger shows real+effective.
    ctx.audit_entry(NewAuditEntry {
        user_id: Some(real.id.clone()),
        action: "impersonate.start".into(),
        target: Some(target.id.clone()),
        detail: Some(serde_json::json!({
            "real_user_id": real.id,
            "effective_user_id": target.id,
            "effective_username": target.username,
        })),
        ip: None,
    })
    .await;

    Ok(Json(ImpersonateResp { token }))
}

/// `POST /api/v1/admin/impersonate/stop`
///
/// Revoke the presented impersonation token, ending the overlay (the admin's
/// own token, kept client-side, is unaffected). Audits `impersonate.stop` with
/// the real + effective ids. Returns `204`. Revoking a non-impersonation token
/// is a harmless no-op revoke of the presented credential.
pub async fn stop<C: ImpersonateCtx>(
    State(ctx): State<C>,
    auth: CurrentAuthContext,
    axum::Extension(BearerToken(token)): axum::Extension<BearerToken>,
) -> ApiResult<StatusCode> {
    let real = auth.real_user();
    let effective = auth.effective_user();

    // Revoke the presented token (the impersonation token). Idempotent.
    ctx.auth_repo().revoke(&token).await?;

    ctx.audit_entry(NewAuditEntry {
        user_id: Some(real.id.clone()),
        action: "impersonate.stop".into(),
        target: Some(effective.id.clone()),
        detail: Some(serde_json::json!({
            "real_user_id": real.id,
            "effective_user_id": effective.id,
        })),
        ip: None,
    })
    .await;

    Ok(StatusCode::NO_CONTENT)
}
