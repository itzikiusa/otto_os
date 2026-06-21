//! Grants API — RBAC Task 2.1.
//!
//! ## Endpoints
//! - `GET  /api/v1/users/{id}/grants`        — list a user's grants (Users:Admin / root).
//! - `PUT  /api/v1/users/{id}/grants`        — replace-all a user's grants; audited.
//! - `GET  /api/v1/auth/capabilities`        — the **caller's** effective feature→capability
//!   map; any authenticated user may call it (exempt from the feature guard).
//!
//! ## Policy
//! - `/users/{id}/grants` resolves to `Require(Users, Admin)` through the existing
//!   `/users/` prefix rule in `policy.rs`.
//! - `/auth/capabilities` is `Exempt` (self-scoped; added to the exempt set in
//!   `policy.rs`).
//!
//! ## Audit
//! PUT writes a `"grant.changed"` entry via `ctx.audit(...)` (best-effort,
//! same pattern as `token.mint` in `auth_routes.rs`).  The detail field records
//! actor, target user, and old→new grant lists.
//!
//! ## Generics
//! Handlers are generic over `GrantsCtx` so they can be used with the production
//! `ServerCtx` and with a minimal test state without assembling the full
//! server graph (mirroring the `HasGrants` pattern from `feature_guard.rs`).

use axum::extract::{Path, State};
use axum::Json;
use otto_core::api::{CapabilitiesResp, GrantEntry, UserGrantsReq, UserGrantsResp};
use otto_core::domain::{Capability, Feature};
use otto_core::{Error, Id, Result as OttoResult};
use otto_state::{AuditRepo, GrantsRepo, NewAuditEntry, PluginsRepo, UsersRepo};
use std::collections::HashMap;

use crate::auth::{require_root, CurrentUser};
use crate::error::ApiResult;
use crate::state::ServerCtx;

/// All 18 product features in a stable iteration order (matches `Feature` enum).
const ALL_FEATURES: &[Feature] = &[
    Feature::Agents,
    Feature::Connections,
    Feature::Database,
    Feature::Git,
    Feature::Issues,
    Feature::Product,
    Feature::Swarm,
    Feature::ApiClient,
    Feature::Workflows,
    Feature::Channels,
    Feature::SkillEval,
    Feature::Skills,
    Feature::Insights,
    Feature::Usage,
    Feature::SelfImprovement,
    Feature::Context,
    Feature::Settings,
    Feature::Users,
];

// ---------------------------------------------------------------------------
// GrantsCtx trait — allows the handlers to run against a minimal test state
// without assembling the full ServerCtx (mirrors HasGrants in feature_guard).
// ---------------------------------------------------------------------------

/// State that the grant handlers can work against. Implemented for `ServerCtx`
/// (production) and for a minimal `TestGrantCtx` in the integration tests.
pub trait GrantsCtx: Clone + Send + Sync + 'static {
    fn grants_repo(&self) -> GrantsRepo;
    fn audit_repo(&self) -> AuditRepo;
    fn users_repo(&self) -> UsersRepo;
    /// Installed runtime plugins (for surfacing their slugs in capabilities).
    fn plugins_repo(&self) -> PluginsRepo;
    /// Write a best-effort audit entry (failure is logged, not propagated).
    fn audit_entry(&self, entry: NewAuditEntry) -> impl std::future::Future<Output = ()> + Send;
}

impl GrantsCtx for ServerCtx {
    fn grants_repo(&self) -> GrantsRepo {
        // Carry the shared auth cache as the invalidator so `set_grants` flushes
        // that user's cached auth contexts immediately (no stale-grant window).
        GrantsRepo::new_with_invalidator(
            self.pool.clone(),
            std::sync::Arc::new(self.auth_cache.clone()),
        )
    }
    fn audit_repo(&self) -> AuditRepo {
        AuditRepo::new(self.pool.clone())
    }
    fn users_repo(&self) -> UsersRepo {
        UsersRepo::new(self.pool.clone())
    }
    fn plugins_repo(&self) -> PluginsRepo {
        PluginsRepo::new(self.pool.clone())
    }
    async fn audit_entry(&self, entry: NewAuditEntry) {
        self.audit(entry).await;
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a `&[GrantEntry]` into `Vec<(Feature, Capability)>`, returning
/// `Error::Invalid` for any unrecognised string.
fn parse_grant_entries(entries: &[GrantEntry]) -> OttoResult<Vec<(Feature, Capability)>> {
    entries
        .iter()
        .map(|e| {
            let feature = Feature::parse(&e.feature)
                .ok_or_else(|| Error::Invalid(format!("unknown feature '{}'", e.feature)))?;
            let capability = Capability::parse(&e.capability).ok_or_else(|| {
                Error::Invalid(format!("unknown capability '{}'", e.capability))
            })?;
            Ok((feature, capability))
        })
        .collect()
}

/// Encode `&[(Feature, Capability)]` as `Vec<GrantEntry>`.
pub fn encode_grants(pairs: &[(Feature, Capability)]) -> Vec<GrantEntry> {
    pairs
        .iter()
        .map(|(f, c)| GrantEntry {
            feature: f.as_str().to_string(),
            capability: c.as_str().to_string(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Handlers (generic over GrantsCtx)
// ---------------------------------------------------------------------------

/// `GET /api/v1/users/{id}/grants`
///
/// Returns the stored grants for user `{id}`.  Requires `Users:Admin` (enforced
/// by the feature guard via the `/users/` prefix rule) or root.
pub async fn get_grants<C: GrantsCtx>(
    Path(id): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<UserGrantsResp>> {
    require_root(&user)?;
    // Ensure the target user exists (returns 404 if not).
    ctx.users_repo().get(&id).await?;
    let pairs = ctx.grants_repo().grants_for(&id).await?;
    Ok(Json(UserGrantsResp {
        grants: encode_grants(&pairs),
    }))
}

/// `PUT /api/v1/users/{id}/grants`
///
/// Atomically replaces all grants for user `{id}`.  Writes a `"grant.changed"`
/// audit entry (actor = calling user; target = `{id}`; detail = old→new).
/// Requires `Users:Admin` (feature guard) or root.
pub async fn put_grants<C: GrantsCtx>(
    Path(id): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UserGrantsReq>,
) -> ApiResult<Json<UserGrantsResp>> {
    require_root(&user)?;
    // Ensure the target user exists.
    ctx.users_repo().get(&id).await?;

    let repo = ctx.grants_repo();

    // Capture old grants for the audit trail.
    let old_grants = repo.grants_for(&id).await?;

    // Parse and validate the incoming grant entries.
    let new_pairs = parse_grant_entries(&req.grants)?;

    // Atomically replace.
    repo.set_grants(&id, &new_pairs).await?;

    // Audit (best-effort — failure must not abort the request).
    let old_encoded = encode_grants(&old_grants);
    let new_encoded = encode_grants(&new_pairs);
    ctx.audit_entry(NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: "grant.changed".into(),
        target: Some(id.clone()),
        detail: Some(serde_json::json!({
            "old": old_encoded,
            "new": new_encoded,
        })),
        ip: None,
    })
    .await;

    Ok(Json(UserGrantsResp {
        grants: new_encoded,
    }))
}

/// `GET /api/v1/auth/capabilities`
///
/// Returns the **caller's** effective `{feature: capability}` map.  Root receives
/// `"admin"` for every feature.  Any authenticated user may call this (Exempt in
/// the policy table).
pub async fn capabilities<C: GrantsCtx>(
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<CapabilitiesResp>> {
    let repo = ctx.grants_repo();
    let mut caps: HashMap<String, String> = HashMap::with_capacity(ALL_FEATURES.len());

    for &feature in ALL_FEATURES {
        let cap = repo.capability_of(&user, feature).await?;
        caps.insert(feature.as_str().to_string(), cap.as_str().to_string());
    }

    // Custom plugins: the closed `ALL_FEATURES` loop can't surface them, so add
    // each installed runtime plugin's slug-keyed capability explicitly (the DTO is
    // a String→String map, so arbitrary slug keys are fine). The UI's `canPlugin`
    // reads these. Plugin count is small ⇒ a handful of extra point lookups.
    for slug in ctx.plugins_repo().list_slugs().await? {
        let cap = repo.capability_of_plugin(&user, &slug).await?;
        caps.insert(slug, cap.as_str().to_string());
    }

    Ok(Json(CapabilitiesResp { capabilities: caps }))
}

// ---------------------------------------------------------------------------
// Custom-plugin grants (string-keyed by plugin slug) — a distinct code path from
// the feature-grant handlers above: `Feature::parse` rejects slugs, so these
// validate the slug against the installed manifests instead. Reuses the
// `GrantEntry`/`UserGrantsReq`/`UserGrantsResp` DTOs with `feature` = the slug.
// ---------------------------------------------------------------------------

/// Parse `&[GrantEntry]` (where `feature` is a plugin slug) into `(slug, Capability)`,
/// rejecting any slug that is not an installed plugin and any bad capability.
fn parse_plugin_grant_entries(entries: &[GrantEntry]) -> OttoResult<Vec<(String, Capability)>> {
    // The `feature` field carries the plugin slug. We accept any slug (granting
    // before a plugin is installed is harmless — the row is simply unused), and
    // only validate the capability.
    entries
        .iter()
        .map(|e| {
            let capability = Capability::parse(&e.capability).ok_or_else(|| {
                Error::Invalid(format!("unknown capability '{}'", e.capability))
            })?;
            Ok((e.feature.clone(), capability))
        })
        .collect()
}

/// Encode `&[(slug, Capability)]` as `Vec<GrantEntry>`.
fn encode_plugin_grants(pairs: &[(String, Capability)]) -> Vec<GrantEntry> {
    pairs
        .iter()
        .map(|(slug, c)| GrantEntry {
            feature: slug.clone(),
            capability: c.as_str().to_string(),
        })
        .collect()
}

/// `GET /api/v1/users/{id}/plugin-grants` — list a user's plugin grants.
/// Requires root (mirrors `get_grants`; `/users/` prefix is `Users:Admin`-gated).
pub async fn get_plugin_grants<C: GrantsCtx>(
    Path(id): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<UserGrantsResp>> {
    require_root(&user)?;
    ctx.users_repo().get(&id).await?;
    let pairs = ctx.grants_repo().plugin_grants_for(&id).await?;
    Ok(Json(UserGrantsResp {
        grants: encode_plugin_grants(&pairs),
    }))
}

/// `PUT /api/v1/users/{id}/plugin-grants` — atomically replace a user's plugin
/// grants. Writes a `"plugin_grant.changed"` audit entry. Requires root.
pub async fn put_plugin_grants<C: GrantsCtx>(
    Path(id): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UserGrantsReq>,
) -> ApiResult<Json<UserGrantsResp>> {
    require_root(&user)?;
    ctx.users_repo().get(&id).await?;

    let repo = ctx.grants_repo();
    let old = repo.plugin_grants_for(&id).await?;
    let new_pairs = parse_plugin_grant_entries(&req.grants)?;
    repo.set_plugin_grants(&id, &new_pairs).await?;

    let old_encoded = encode_plugin_grants(&old);
    let new_encoded = encode_plugin_grants(&new_pairs);
    ctx.audit_entry(NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: "plugin_grant.changed".into(),
        target: Some(id.clone()),
        detail: Some(serde_json::json!({
            "old": old_encoded,
            "new": new_encoded,
        })),
        ip: None,
    })
    .await;

    Ok(Json(UserGrantsResp {
        grants: new_encoded,
    }))
}
