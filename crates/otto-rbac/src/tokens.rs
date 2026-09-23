//! Bearer-token auth sessions over the `auth_sessions` table.
//!
//! Tokens are 32 random bytes hex-encoded (64 chars); only the SHA-256 hex of
//! the token is stored. Expiry is sliding: 30 days from last_seen, refreshed
//! at most once per hour to throttle writes.
//!
//! # Auth-lookup cache
//!
//! `AuthRepo` optionally holds an [`AuthCache`] to short-circuit the three
//! SQLite hits per request (token row + user join, optional target user for
//! impersonation, grant lookup). The cache is guarded by strict invariants:
//!
//! - **`kind='share'` and `kind='impersonation'` tokens are NEVER cached.**
//!   They are explicitly revocable mid-session and the cost of a missed
//!   invalidation (stale access after revoke) outweighs any latency benefit.
//!   Every authenticated request for these kinds always hits the DB.
//! - Every revocation path evicts the affected entry **before** returning:
//!   `revoke`, `revoke_api_token`, and `revoke_all_for_user` all call
//!   `cache.evict(hash)` / `cache.evict_user(uid)` synchronously, so there is
//!   no window where a revoked token is served from cache.
//! - Grant changes invalidate the user's cached context via
//!   [`GrantsInvalidator::invalidate_user`], implemented by `AuthCache`.
//! - The cache is disabled entirely (all paths hit the DB) when
//!   [`AUTH_CACHE_ENABLED`] is `false`.

use chrono::{DateTime, Duration, Utc};
use otto_core::api::{ApiTokenInfo, McpTokenInfo, ShareInfo};
use otto_core::auth::{AuthContext, McpScope, SessionScope};
use otto_core::domain::{User, WorkspaceRole};
use otto_core::{new_id, Error, Id, Result};
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

use crate::cache::AuthCache;

/// Sliding expiry window for interactive (`kind='session'`) login tokens.
const TOKEN_TTL_DAYS: i64 = 30;
/// Fixed lifetime for `kind='api'` personal access tokens (~10 years). Long
/// enough to behave as "create once"; the expiry is never slid for these.
const API_TOKEN_TTL_DAYS: i64 = 3650;
/// Label used by the legacy outward-server token minted through
/// `PATCH /mcp/otto-server`.
pub const LEGACY_OTTO_MCP_SERVER_LABEL: &str = "otto-mcp-server";
/// Default fixed lifetime for `kind='impersonation'` tokens (30 minutes). Short
/// and **never slid** — an admin acting-as a user gets a tight window, after
/// which the overlay simply expires (the admin's own token is unaffected).
pub const IMPERSONATION_TOKEN_TTL_MINS: i64 = 30;
/// Hard ceiling on a `kind='share'` token's lifetime (24h). A share is a public
/// capability URL that can leak, so its TTL is **short and FIXED** (never slid).
/// Requests above this are clamped down.
pub const SHARE_TOKEN_TTL_MAX_SECS: i64 = 24 * 60 * 60;
/// Floor on a share token's lifetime (60s). A request below this is clamped up
/// so a share is always usable for at least a moment after minting.
pub const SHARE_TOKEN_TTL_MIN_SECS: i64 = 60;
/// Hard ceiling on the email-OTP share **session window** (`max_expires_at`):
/// 12 hours (mobile plan Task 7.2 / design addendum). A leaked link gated by an
/// emailed code can at most grant a 12h window per verification; requests above
/// this are clamped down.
pub const SHARE_OTP_WINDOW_MAX_SECS: i64 = 12 * 60 * 60;
/// Lifetime of a single emailed OTP (10 minutes). Short by design — the code is
/// a second factor delivered out-of-band, single-use, and rate-limited.
pub const SHARE_OTP_TTL_SECS: i64 = 600;
/// Minimum age of `last_seen_at` before we touch the row again (throttles
/// writes; for session tokens this also slides the expiry).
const TOUCH_THROTTLE_SECS: i64 = 3600;

/// SHA-256 hex of a raw token string.
pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Generate a uniformly-distributed 6-digit numeric OTP (`"000000"`..="999999")
/// from `OsRng`. Rejection-samples to avoid the modulo bias a bare `% 1_000_000`
/// would introduce, so every code is equally likely. The plaintext is returned
/// to the caller exactly once (to email); only its SHA-256 is ever stored.
pub fn generate_otp() -> String {
    let mut rng = rand::rngs::OsRng;
    // Largest multiple of 1_000_000 that fits in u32, used as the rejection
    // bound so the sampled value maps onto [0, 1_000_000) without bias.
    const BOUND: u32 = (u32::MAX / 1_000_000) * 1_000_000;
    loop {
        let n = rng.next_u32();
        if n < BOUND {
            return format!("{:06}", n % 1_000_000);
        }
    }
}

fn parse_ts(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| Error::Internal(format!("bad timestamp '{s}': {e}")))
}

/// Conservative display-only recognition of historical Otto token labels.
/// Never use a human-controlled label for automatic revocation.
fn legacy_session_label(label: &str) -> Option<&str> {
    let id = label.strip_prefix("otto-mcp:")?;
    (id.len() == 26 && id.as_bytes()[0] <= b'7'
        && id.bytes().all(|b| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&b)))
        .then_some(id)
}

/// Repository for `auth_sessions`.
///
/// Holds an optional short-TTL [`AuthCache`] to avoid redundant SQLite reads on
/// hot paths. Construct with [`AuthRepo::new`] (no cache) or
/// [`AuthRepo::with_cache`] (cache enabled). The `RbacAuthenticator` wired into
/// the server always uses the cached variant; direct repo construction in tests
/// uses the uncached form so tests prove DB-level correctness without touching
/// the cache layer.
#[derive(Clone)]
pub struct AuthRepo {
    pool: SqlitePool,
    /// `None` = caching disabled for this instance (all paths hit the DB).
    cache: Option<AuthCache>,
}

impl AuthRepo {
    /// Construct without a cache (every authenticate call hits the DB). Used in
    /// unit tests and any context where caching is not desired.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool, cache: None }
    }

    /// Construct with an attached [`AuthCache`]. The cache is shared via
    /// `Arc`-interior cloning, so `AuthRepo::clone()` and the `GrantsInvalidator`
    /// impl point at the same backing map.
    pub fn with_cache(pool: SqlitePool, cache: AuthCache) -> Self {
        Self {
            pool,
            cache: Some(cache),
        }
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
    ///
    /// Returns an [`AuthContext`]:
    /// - For a **normal** token (`kind` `'session'`/`'api'`) the context's
    ///   `real_user` and `effective_user` are the same looked-up user.
    /// - For an **impersonation** token (`kind='impersonation'`, Task 5.2) the
    ///   `real_user` is the admin that owns the row (`user_id`) and the
    ///   `effective_user` is the impersonation target (`acting_as_user_id`).
    ///   The row is rejected (Unauthorized) if it has expired or if **either**
    ///   the admin or the target user is disabled (the target must exist).
    ///
    /// # Caching
    ///
    /// When an [`AuthCache`] is attached, a cache hit for a `login`/`api` token
    /// returns the stored [`AuthContext`] without touching the DB. `share` and
    /// `impersonation` tokens are NEVER served from cache — they always hit the
    /// DB. On a DB miss (or for non-cacheable kinds) the result is inserted into
    /// the cache (for `login`/`api` only) before returning.
    pub async fn authenticate(&self, token: &str) -> Result<AuthContext> {
        let hash = token_hash(token);

        // Cache fast-path: only populated for login/api tokens (never for share
        // or impersonation). A hit means: the token was valid at insert-time,
        // TTL has not elapsed, and evict() has not been called for this hash
        // (which revoke paths do synchronously before returning). Safe to serve.
        if let Some(cache) = &self.cache {
            if let Some(ctx) = cache.get(&hash) {
                return Ok(ctx);
            }
        }
        // Resolve the row's own fields (kind/expiry/target) plus the REAL user
        // (the token owner) in one shot. The target user (impersonation only) is
        // loaded separately below to keep the common path a single join.
        let row = sqlx::query(
            "SELECT a.token_hash, a.expires_at, a.last_seen_at, a.kind, a.acting_as_user_id,
                    a.session_scope, a.scope_role, a.revoked,
                    a.recipient_email, a.verified_at, a.max_expires_at, a.mcp_scope,
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
        // The REAL user (token owner) must not be disabled.
        if row.get::<i64, _>("disabled") != 0 {
            return Err(Error::Unauthorized);
        }
        // A share token's explicit kill switch: `revoked=1` is a hard reject,
        // checked before any upgrade so a revoked link can never open a socket.
        if row.get::<i64, _>("revoked") != 0 {
            return Err(Error::Unauthorized);
        }

        let kind: String = row.get("kind");
        let is_impersonation = kind == "impersonation";
        let is_share = kind == "share";
        // Session-managed credentials retain the owner's permissions, but only
        // while their originating session exists. Never cache this liveness
        // check: deleting a session must close access even after a restart.
        let managed_session: Option<String> = if kind == "api" {
            row.get("session_scope")
        } else { None };
        if let Some(sid) = &managed_session {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ? AND created_by = ?)",
            ).bind(sid).bind(row.get::<String, _>("id"))
                .fetch_one(&self.pool).await
                .map_err(|e| Error::Internal(format!("managed token session: {e}")))?;
            if !exists { return Err(Error::Unauthorized); }
        }

        let last_seen = parse_ts(&row.get::<String, _>("last_seen_at"))?;
        // Share tokens have a SHORT, FIXED TTL: never touch/slide them. Skipping
        // the touch entirely (not just the slide) keeps their expiry verifiably
        // immutable — `authenticate` is a pure read for kind='share'.
        if !is_share && now - last_seen > Duration::seconds(TOUCH_THROTTLE_SECS) {
            // Always record "last used"; only SLIDE the expiry for interactive
            // session tokens. API tokens keep their long fixed lifetime and
            // impersonation tokens have a SHORT FIXED TTL — neither is ever slid
            // (impersonation must time out predictably).
            let slide = !is_impersonation && kind != "api" && kind != "mcp" && kind != "agent_mcp";
            if slide {
                sqlx::query(
                    "UPDATE auth_sessions SET last_seen_at = ?, expires_at = ? WHERE token_hash = ?",
                )
                .bind(now.to_rfc3339())
                .bind((now + Duration::days(TOKEN_TTL_DAYS)).to_rfc3339())
                .bind(&hash)
                .execute(&self.pool)
                .await
                .map_err(|e| Error::Internal(format!("touch token: {e}")))?;
            } else {
                sqlx::query("UPDATE auth_sessions SET last_seen_at = ? WHERE token_hash = ?")
                    .bind(now.to_rfc3339())
                    .bind(&hash)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| Error::Internal(format!("touch token: {e}")))?;
            }
        }

        let real_user = User {
            id: row.get("id"),
            username: row.get("username"),
            display_name: row.get("display_name"),
            is_root: row.get::<i64, _>("is_root") != 0,
            disabled: false,
            created_at: parse_ts(&row.get::<String, _>("created_at"))?,
        };

        if is_impersonation {
            // Impersonation overlay: load the target (effective) user and reject
            // if it is missing or disabled. Authorization runs against the
            // target; audit records the admin (real_user).
            let target_id: Option<String> = row.get("acting_as_user_id");
            let target_id = target_id.ok_or(Error::Unauthorized)?;
            let target = sqlx::query(
                "SELECT id, username, display_name, is_root, disabled, created_at
                 FROM users WHERE id = ?",
            )
            .bind(&target_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| Error::Internal(format!("authenticate target: {e}")))?
            .ok_or(Error::Unauthorized)?;
            if target.get::<i64, _>("disabled") != 0 {
                return Err(Error::Unauthorized);
            }
            let effective_user = User {
                id: target.get("id"),
                username: target.get("username"),
                display_name: target.get("display_name"),
                is_root: target.get::<i64, _>("is_root") != 0,
                disabled: false,
                created_at: parse_ts(&target.get::<String, _>("created_at"))?,
            };
            return Ok(AuthContext {
                real_user,
                effective_user,
                // Impersonation tokens are never scoped (mutually exclusive with
                // a share-link scope; the share-token path will populate `scope`
                // in mobile plan Task 1.3).
                scope: None,
                mcp_only: false,
                mcp_scope: None,
                mcp_internal: false,
                mcp_session_id: None,
            managed_session_id: None,
            });
        }

        if is_share {
            // A share token is the OWNER's own scoped capability — NOT
            // impersonation. `real_user == effective_user == owner`; containment
            // to the single session comes entirely from the `scope` (enforced by
            // the deny-by-default scope guard in a later task). The capped role
            // is read from `scope_role` ('viewer'|'editor'); an unparsable or
            // missing scope is a hard reject (a share must always be bounded).
            let session_scope: Option<String> = row.get("session_scope");
            let session_id = session_scope.ok_or(Error::Unauthorized)?;
            let scope_role: Option<String> = row.get("scope_role");
            let role = scope_role
                .as_deref()
                .and_then(WorkspaceRole::parse)
                .ok_or(Error::Unauthorized)?;
            // SECURITY: a share principal is **never root** (mobile plan Task 1.4).
            // The primary account is often root, and a share is a public capability
            // URL that can leak — so even though the share carries the *owner's*
            // identity (its `id` is preserved, keeping the scoped session's
            // `created_by == id` ownership check working), the root flag is DROPPED.
            // This way a leaked share link can never carry root's blanket bypass:
            // it is confined to its one session by the scope guard with no root
            // escape hatch behind it. Clone the owner and force `is_root=false`.
            let mut share_user = real_user;
            share_user.is_root = false;
            // Email-OTP gate (mobile plan Tasks 7.2/7.3). When the share was
            // minted with a locked `recipient_email`, the guest must redeem an
            // emailed OTP via `/share/verify` before the scope reaches anything.
            // `otp_pending` is computed here and the two guards (feature-guard
            // scope branch + the terminal-WS gate) deny everything while it is
            // true (only `/share/verify`, Exempt, stays reachable). A plain share
            // (no recipient email) is never OTP-pending → unchanged behaviour.
            let recipient_email: Option<String> = row.get("recipient_email");
            let otp_pending = if recipient_email.is_some() {
                let verified_at: Option<i64> = row.get("verified_at");
                let max_expires_at: Option<i64> = row.get("max_expires_at");
                let now_secs = now.timestamp();
                // Pending until verified, and re-pending once the verified window
                // (`max_expires_at`, ≤12h) has elapsed. A missing window is
                // treated as expired (fail closed).
                verified_at.is_none() || max_expires_at.map(|m| m <= now_secs).unwrap_or(true)
            } else {
                false
            };
            return Ok(AuthContext {
                real_user: share_user.clone(),
                effective_user: share_user,
                scope: Some(SessionScope {
                    session_id: Id::from(session_id),
                    role,
                    otp_pending,
                }),
                mcp_only: false,
                mcp_scope: None,
                mcp_internal: false,
                mcp_session_id: None,
            managed_session_id: None,
            });
        }

        // Normal token (kind='session'/'api'/'mcp'): the real (token owner) and
        // effective (acted-as) user are the same, and it reaches the whole
        // authorized surface (no session scope). These are the ONLY kinds we cache.
        //
        // For a `kind='mcp'` token we additionally resolve its per-token
        // [`McpScope`] from the `mcp_scope` JSON column: a NULL/absent/garbled
        // column resolves to [`McpScope::unrestricted`] so a legacy token (minted
        // before per-token scopes existed) keeps full access. The scope is cached
        // alongside the context — it is immutable for the life of a token (changing
        // access = revoke + re-mint), so the cache never goes stale on it.
        let mcp_internal = kind == "agent_mcp";
        let mcp_only = kind == "mcp" || mcp_internal;
        let mcp_scope = if mcp_internal {
            let raw: Option<String> = row.get("mcp_scope");
            Some(
                raw.as_deref()
                    .and_then(|value| serde_json::from_str::<McpScope>(value).ok())
                    .ok_or(Error::Unauthorized)?,
            )
        } else if mcp_only {
            let raw: Option<String> = row.get("mcp_scope");
            Some(
                raw.as_deref()
                    .and_then(|s| serde_json::from_str::<McpScope>(s).ok())
                    .unwrap_or_else(McpScope::unrestricted),
            )
        } else {
            None
        };
        let mcp_session_id = if mcp_internal {
            Some(Id::from(
                row.get::<Option<String>, _>("session_scope")
                    .filter(|value| !value.is_empty())
                    .ok_or(Error::Unauthorized)?,
            ))
        } else {
            None
        };
        let ctx = AuthContext {
            real_user: real_user.clone(),
            effective_user: real_user,
            scope: None,
            // A `kind='mcp'` token is route-restricted by the feature guard to the
            // governed invoke choke point + the HTTP transport (design §14 F1).
            mcp_only,
            mcp_scope,
            mcp_internal,
            managed_session_id: managed_session.clone().or_else(|| mcp_session_id.clone()),
            mcp_session_id,
        };

        // Populate the cache so the next request for this token avoids the DB.
        // We use the `user_id` column read from the join above (the `id` field
        // of `real_user`, which is always the token-owner row's `user_id`).
        if let Some(cache) = self.cache.as_ref().filter(|_| managed_session.is_none()) {
            cache.insert(hash, ctx.real_user.id.clone(), ctx.clone());
        }

        Ok(ctx)
    }

    /// Revoke (delete) the auth session matching `token`. Idempotent.
    ///
    /// Evicts the token from the auth cache **before** returning so no window
    /// exists where a revoked token could be served from cache.
    pub async fn revoke(&self, token: &str) -> Result<()> {
        let hash = token_hash(token);
        sqlx::query("DELETE FROM auth_sessions WHERE token_hash = ?")
            .bind(&hash)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Internal(format!("revoke token: {e}")))?;
        if let Some(cache) = &self.cache {
            cache.evict(&hash);
        }
        Ok(())
    }

    /// Revoke ALL of `user_id`'s sessions — interactive login tokens, long-lived
    /// API tokens, AND scoped share-link tokens the user minted. Used when a
    /// credential changes (password reset), the account is disabled, or the
    /// owner hits "revoke all" after losing a device, so every previously-issued
    /// token (every `kind`) is invalidated in one shot. The blanket `DELETE`
    /// removes share rows too, which is strictly stronger than flipping their
    /// `revoked` flag. Returns the number of sessions deleted.
    ///
    /// Evicts all of `user_id`'s cached auth entries immediately, closing any
    /// window between the DB `DELETE` and future cache lookups.
    pub async fn revoke_all_for_user(&self, user_id: &Id) -> Result<u64> {
        let res = sqlx::query("DELETE FROM auth_sessions WHERE user_id = ?")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Internal(format!("revoke all for user: {e}")))?;
        if let Some(cache) = &self.cache {
            cache.evict_user(user_id);
        }
        Ok(res.rows_affected())
    }

    /// Mint a long-lived API (personal access) token for `user_id`. Returns the
    /// RAW token (shown to the caller exactly once) plus its metadata.
    pub async fn issue_api_token(
        &self,
        user_id: &Id,
        label: Option<&str>,
    ) -> Result<(String, ApiTokenInfo)> {
        self.issue_api_token_inner(user_id, label, None).await
    }

    /// A durable lifecycle association, separate from the human-readable label.
    /// The credential is replaced on spawn and revoked on removal/failed spawn.
    pub async fn issue_session_api_token(
        &self, user_id: &Id, session_id: &Id,
    ) -> Result<(String, ApiTokenInfo)> {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ? AND created_by = ?)",
        ).bind(session_id).bind(user_id).fetch_one(&self.pool).await
            .map_err(|e| Error::Internal(format!("issue session token: {e}")))?;
        if !exists { return Err(Error::Unauthorized); }
        self.issue_api_token_inner(user_id, Some(&format!("otto-mcp:{session_id}")), Some(session_id)).await
    }

    async fn issue_api_token_inner(
        &self, user_id: &Id, label: Option<&str>, session_id: Option<&Id>,
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
               (id, user_id, token_hash, created_at, expires_at, last_seen_at, kind, label, token_prefix, session_scope)
             VALUES (?, ?, ?, ?, ?, ?, 'api', ?, ?, ?)",
        )
        .bind(&id)
        .bind(user_id)
        .bind(token_hash(&token))
        .bind(now.to_rfc3339())
        .bind(expires_at.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(label)
        .bind(&prefix)
        .bind(session_id)
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
                session_id: session_id.cloned(),
                legacy_session_id: None,
                session_exists: session_id.map(|_| true),
            },
        ))
    }

    /// Revoke ALL managed credentials for this session, including those minted
    /// by another daemon instance. A matching label alone never grants ownership.
    pub async fn revoke_session_tokens(&self, owner: &Id, session_id: &Id) -> Result<u64> {
        let hashes: Vec<String> = sqlx::query_scalar(
            "DELETE FROM auth_sessions WHERE user_id = ? AND session_scope = ?
             AND kind IN ('api', 'agent_mcp') RETURNING token_hash",
        ).bind(owner).bind(session_id).fetch_all(&self.pool).await
            .map_err(|e| Error::Internal(format!("revoke session tokens: {e}")))?;
        if let Some(cache) = &self.cache {
            for hash in &hashes { cache.evict(hash); }
        }
        Ok(hashes.len() as u64)
    }

    /// Daemon-boot sweep of **managed** per-session credentials (`session_scope`
    /// set, kind `api`/`agent_mcp`). Each is injected into exactly one agent
    /// process, no agent process survives a daemon restart, and every resume
    /// mints a fresh one — so at boot all of them are dead credentials that
    /// would otherwise keep the owner's full permissions for their 10-year TTL
    /// while their session row lingers (exited / reconnectable / archived).
    /// Marked revoked + expired, not deleted (the token list keeps showing
    /// them until the next spawn's rotation or the user removes them).
    /// Returns the number revoked.
    pub async fn expire_managed_session_tokens(&self) -> Result<u64> {
        let hashes: Vec<String> = sqlx::query_scalar(
            "UPDATE auth_sessions SET revoked = 1, expires_at = ?
             WHERE session_scope IS NOT NULL AND kind IN ('api', 'agent_mcp') AND revoked = 0
             RETURNING token_hash",
        )
        .bind(Utc::now().to_rfc3339())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("expire managed session tokens: {e}")))?;
        if let Some(cache) = &self.cache {
            for hash in &hashes {
                cache.evict(hash);
            }
        }
        Ok(hashes.len() as u64)
    }

    /// Daemon-boot sweep of the **legacy** per-session MCP tokens: full-owner
    /// `kind='api'` tokens labelled `otto-mcp:<session ULID>` with no
    /// `session_scope`, minted by Otto before managed ownership existed
    /// (`created_at < cutover`). Nothing mints them any more and their agent
    /// processes died with an earlier daemon, yet they kept authenticating —
    /// thousands of live 10-year credentials, many for deleted sessions.
    ///
    /// Label-based, so deliberately narrow: only an exact Otto label (see
    /// [`legacy_session_label`]), only rows older than `cutover` (a token a
    /// user names this way later is never touched), and never one whose label
    /// names ANOTHER user's existing session. Marked revoked + expired, not
    /// deleted, so a mistaken match stays visible in the token list. Returns
    /// the number revoked.
    pub async fn expire_legacy_session_label_tokens(&self, cutover: DateTime<Utc>) -> Result<u64> {
        let rows = sqlx::query(
            "SELECT a.id, a.label, a.token_hash, a.user_id, s.created_by AS session_owner
             FROM auth_sessions a LEFT JOIN sessions s ON s.id = substr(a.label, 10)
             WHERE a.kind = 'api' AND a.session_scope IS NULL AND a.revoked = 0
               AND a.label LIKE 'otto-mcp:%' AND a.created_at < ?",
        )
        .bind(cutover.to_rfc3339())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("list legacy session tokens: {e}")))?;
        let stale: Vec<(String, String)> = rows
            .into_iter()
            .filter(|row| {
                let label: Option<String> = row.get("label");
                let owner: String = row.get("user_id");
                let session_owner: Option<String> = row.get("session_owner");
                label.as_deref().and_then(legacy_session_label).is_some()
                    && session_owner.is_none_or(|o| o == owner)
            })
            .map(|row| (row.get("id"), row.get("token_hash")))
            .collect();
        if stale.is_empty() {
            return Ok(0);
        }
        let now = Utc::now().to_rfc3339();
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| Error::Internal(format!("expire legacy session tokens: {e}")))?;
        for (id, _) in &stale {
            sqlx::query("UPDATE auth_sessions SET revoked = 1, expires_at = ? WHERE id = ?")
                .bind(&now)
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(|e| Error::Internal(format!("expire legacy session token: {e}")))?;
        }
        tx.commit()
            .await
            .map_err(|e| Error::Internal(format!("expire legacy session tokens: {e}")))?;
        if let Some(cache) = &self.cache {
            for (_, hash) in &stale {
                cache.evict(hash);
            }
        }
        Ok(stale.len() as u64)
    }

    /// Mint a **restricted MCP** token (`kind='mcp'`) for the outward "Otto as
    /// MCP server". It carries `user_id`'s identity (so the invoke handler can
    /// re-check that user's RBAC per tool) but the feature guard authorizes it
    /// for ONLY `POST /mcp/otto-tools/invoke` (+ `GET /mcp/otto-server`) — every
    /// other route is 403. Returns the RAW token (shown once). Revoking the row
    /// (or rotating) disables the outward server. Design §14 F1.
    pub async fn issue_mcp_token(&self, user_id: &Id, label: Option<&str>) -> Result<String> {
        // Legacy/unrestricted: full access to every globally-enabled tool. Used by
        // the back-compat rotate path; the scoped variant powers multi-token UX.
        let (token, _info) = self
            .issue_mcp_token_with_scope(user_id, label, &McpScope::unrestricted())
            .await?;
        Ok(token)
    }

    /// Mint a **scoped** MCP token for `user_id`: a `kind='mcp'` token whose
    /// per-token [`McpScope`] is persisted in the `mcp_scope` column and enforced
    /// at the governed invoke choke point. This is the primitive behind multiple
    /// MCP tokens with different accesses. Returns the RAW token (shown once) plus
    /// its [`McpTokenInfo`] metadata (no secret).
    pub async fn issue_mcp_token_with_scope(
        &self,
        user_id: &Id,
        label: Option<&str>,
        scope: &McpScope,
    ) -> Result<(String, McpTokenInfo)> {
        // Resolve the owning user's username up front (and prove the user exists)
        // so the returned metadata can be displayed without a second round-trip.
        let username: String = sqlx::query("SELECT username FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| Error::Internal(format!("issue mcp token (user): {e}")))?
            .map(|r| r.get::<String, _>("username"))
            .ok_or_else(|| Error::Invalid(format!("unknown user '{user_id}'")))?;

        let mut buf = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut buf);
        let token = hex::encode(buf);
        let prefix: String = token.chars().take(12).collect();
        let id = new_id();
        let now = Utc::now();
        let expires_at = now + Duration::days(API_TOKEN_TTL_DAYS);
        let scope_json = serde_json::to_string(scope)
            .map_err(|e| Error::Internal(format!("serialize mcp scope: {e}")))?;
        sqlx::query(
            "INSERT INTO auth_sessions
               (id, user_id, token_hash, created_at, expires_at, last_seen_at, kind, label, token_prefix, mcp_scope)
             VALUES (?, ?, ?, ?, ?, ?, 'mcp', ?, ?, ?)",
        )
        .bind(&id)
        .bind(user_id)
        .bind(token_hash(&token))
        .bind(now.to_rfc3339())
        .bind(expires_at.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(label)
        .bind(&prefix)
        .bind(&scope_json)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("issue mcp token: {e}")))?;
        Ok((
            token,
            McpTokenInfo {
                id,
                user_id: user_id.clone(),
                username,
                label: label.map(str::to_owned),
                token_prefix: prefix,
                scope: scope.clone(),
                created_at: now,
                last_seen_at: now,
                expires_at,
            },
        ))
    }

    /// Mint the internal, per-session credential used by a Vault documentation
    /// reviewer. The persisted token kind forces every HTTP request through the
    /// MCP-only route choke point; the persisted scope permits only Vault reads
    /// in one workspace. Neither property can be changed by editing the child
    /// process environment or its MCP configuration.
    pub async fn issue_vault_reviewer_token(
        &self,
        user_id: &Id,
        label: Option<&str>,
        session_id: &Id,
        workspace_id: &Id,
    ) -> Result<(String, Id)> {
        let scope = McpScope {
            tools: Some(
                [
                    "vault_list",
                    "vault_dir",
                    "vault_read",
                    "vault_search",
                    "vault_backlinks",
                    "vault_tags",
                    "vault_graph",
                    "vault_okf_validate",
                ]
                .into_iter()
                .map(str::to_string)
                .collect(),
            ),
            allow_writes: false,
            workspace_id: Some(workspace_id.clone()),
        };
        let scope_json = serde_json::to_string(&scope)
            .map_err(|e| Error::Internal(format!("serialize reviewer scope: {e}")))?;
        let mut buf = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut buf);
        let token = hex::encode(buf);
        let id = new_id();
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO auth_sessions
               (id, user_id, token_hash, created_at, expires_at, last_seen_at, kind,
                label, token_prefix, mcp_scope, session_scope)
             VALUES (?, ?, ?, ?, ?, ?, 'agent_mcp', ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(user_id)
        .bind(token_hash(&token))
        .bind(now.to_rfc3339())
        .bind((now + Duration::days(TOKEN_TTL_DAYS)).to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(label)
        .bind(token.chars().take(12).collect::<String>())
        .bind(scope_json)
        .bind(session_id)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("issue vault reviewer token: {e}")))?;
        Ok((token, id))
    }

    /// Revoke one internal Vault-reviewer credential by id and owner.
    pub async fn revoke_vault_reviewer_token(&self, user_id: &Id, id: &Id) -> Result<bool> {
        let cached_hash: Option<String> = sqlx::query(
            "SELECT token_hash FROM auth_sessions
             WHERE id = ? AND user_id = ? AND kind = 'agent_mcp'",
        )
        .bind(id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("revoke vault reviewer token lookup: {e}")))?
        .map(|row| row.get("token_hash"));
        let result = sqlx::query(
            "DELETE FROM auth_sessions
             WHERE id = ? AND user_id = ? AND kind = 'agent_mcp'",
        )
        .bind(id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("revoke vault reviewer token: {e}")))?;
        if let (Some(cache), Some(hash)) = (&self.cache, cached_hash) {
            cache.evict(&hash);
        }
        Ok(result.rows_affected() > 0)
    }

    /// List every outward MCP token (across all users), newest first, with the
    /// owning username + per-token scope. Never returns secrets. For the admin
    /// management UI.
    pub async fn list_mcp_tokens(&self) -> Result<Vec<McpTokenInfo>> {
        let rows = sqlx::query(
            "SELECT a.id, a.user_id, a.label, a.token_prefix, a.mcp_scope,
                    a.created_at, a.last_seen_at, a.expires_at, u.username
             FROM auth_sessions a JOIN users u ON u.id = a.user_id
             WHERE a.kind = 'mcp'
             ORDER BY a.created_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("list mcp tokens: {e}")))?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let raw: Option<String> = r.get("mcp_scope");
            let scope = raw
                .as_deref()
                .and_then(|s| serde_json::from_str::<McpScope>(s).ok())
                .unwrap_or_else(McpScope::unrestricted);
            out.push(McpTokenInfo {
                id: r.get("id"),
                user_id: r.get("user_id"),
                username: r.get("username"),
                label: r.get("label"),
                token_prefix: r.get("token_prefix"),
                scope,
                created_at: parse_ts(&r.get::<String, _>("created_at"))?,
                last_seen_at: parse_ts(&r.get::<String, _>("last_seen_at"))?,
                expires_at: parse_ts(&r.get::<String, _>("expires_at"))?,
            });
        }
        Ok(out)
    }

    /// Revoke ONE outward MCP token by id. Returns `true` iff a `kind='mcp'` row
    /// with that id existed and was deleted. Evicts the owner from the auth cache
    /// so the revocation takes effect immediately (the token's cached context, if
    /// any, is dropped). Used by the per-token management UI.
    pub async fn revoke_mcp_token_by_id(&self, id: &Id) -> Result<bool> {
        // Read the owner first so we can evict the cache (keyed by token hash, but
        // evict_user is the precise, cheap coarse hammer we already expose).
        let owner: Option<String> =
            sqlx::query("SELECT user_id FROM auth_sessions WHERE id = ? AND kind = 'mcp'")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| Error::Internal(format!("revoke mcp token (lookup): {e}")))?
                .map(|r| r.get::<String, _>("user_id"));
        let res = sqlx::query("DELETE FROM auth_sessions WHERE id = ? AND kind = 'mcp'")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Internal(format!("revoke mcp token: {e}")))?;
        if let (Some(cache), Some(uid)) = (&self.cache, owner) {
            cache.evict_user(&uid);
        }
        Ok(res.rows_affected() > 0)
    }

    /// Replace exactly one outward MCP token while preserving its owner, label,
    /// and scope. No other token is touched. Returns `None` when `id` does not
    /// identify a `kind='mcp'` token.
    pub async fn rotate_mcp_token(&self, id: &Id) -> Result<Option<(String, McpTokenInfo)>> {
        let row = sqlx::query(
            "SELECT user_id, label, mcp_scope
             FROM auth_sessions
             WHERE id = ? AND kind = 'mcp'",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("rotate mcp token (lookup): {e}")))?;
        let Some(row) = row else {
            return Ok(None);
        };

        let user_id: Id = row.get("user_id");
        let label: Option<String> = row.get("label");
        let raw_scope: Option<String> = row.get("mcp_scope");
        let scope = raw_scope
            .as_deref()
            .and_then(|s| serde_json::from_str::<McpScope>(s).ok())
            .unwrap_or_else(McpScope::unrestricted);
        let (token, info) = self
            .issue_mcp_token_with_scope(&user_id, label.as_deref(), &scope)
            .await?;

        sqlx::query("DELETE FROM auth_sessions WHERE id = ? AND kind = 'mcp'")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Internal(format!("rotate mcp token (delete old): {e}")))?;
        if let Some(cache) = &self.cache {
            cache.evict_user(&user_id);
        }
        Ok(Some((token, info)))
    }

    /// Revoke the user's LEGACY outward-server tokens (label
    /// `otto-mcp-server`, minted by the `rotate_token` path of `PATCH
    /// /mcp/otto-server`). Scoped tokens (`POST /mcp/tokens`) are never touched
    /// — rotating the legacy token must not wipe them (it used to). Per-token
    /// rotation is [`Self::rotate_mcp_token`].
    pub async fn revoke_mcp_tokens(&self, user_id: &Id) -> Result<u64> {
        let res = sqlx::query(
            "DELETE FROM auth_sessions
             WHERE user_id = ? AND kind = 'mcp' AND label = ?",
        )
        .bind(user_id)
        .bind(LEGACY_OTTO_MCP_SERVER_LABEL)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("revoke mcp tokens: {e}")))?;
        if let Some(cache) = &self.cache {
            cache.evict_user(user_id);
        }
        Ok(res.rows_affected())
    }

    /// The 12-char prefix of the user's current legacy stdio token, if any (for
    /// the UI to show which token is active without revealing it).
    pub async fn mcp_token_prefix(&self, user_id: &Id) -> Result<Option<String>> {
        let row = sqlx::query(
            "SELECT token_prefix FROM auth_sessions
             WHERE user_id = ? AND kind = 'mcp' AND label = ?
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(user_id)
        .bind(LEGACY_OTTO_MCP_SERVER_LABEL)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("mcp token prefix: {e}")))?;
        Ok(row.map(|r| r.get::<String, _>("token_prefix")))
    }

    /// Mint a short-lived **impersonation** token: the REAL owner is `real_user_id`
    /// (the admin) and the EFFECTIVE / acted-as user is `target_user_id`. Returns
    /// the RAW token (shown to the caller exactly once).
    ///
    /// The token's TTL is a SHORT fixed window (`ttl`, e.g. 30 min); it is never
    /// slid in [`authenticate`], so the overlay always times out predictably.
    /// All guardrails (caller authority, no impersonating up/sideways, no nesting,
    /// disabled/absent/self target) are enforced by the *route* before this is
    /// called — this method only persists the row.
    pub async fn issue_impersonation_token(
        &self,
        real_user_id: &Id,
        target_user_id: &Id,
        ttl: Duration,
    ) -> Result<String> {
        let mut buf = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut buf);
        let token = hex::encode(buf);
        let prefix: String = token.chars().take(12).collect();
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO auth_sessions
               (id, user_id, token_hash, created_at, expires_at, last_seen_at, kind, token_prefix, acting_as_user_id)
             VALUES (?, ?, ?, ?, ?, ?, 'impersonation', ?, ?)",
        )
        .bind(new_id())
        .bind(real_user_id)
        .bind(token_hash(&token))
        .bind(now.to_rfc3339())
        .bind((now + ttl).to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(&prefix)
        .bind(target_user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("issue impersonation token: {e}")))?;
        Ok(token)
    }

    /// List a user's API tokens (newest first). Never includes the secret.
    pub async fn list_api_tokens(&self, user_id: &Id) -> Result<Vec<ApiTokenInfo>> {
        let rows = sqlx::query(
            "SELECT a.id, a.label, a.token_prefix, a.created_at, a.last_seen_at, a.expires_at,
                    a.session_scope, EXISTS(SELECT 1 FROM sessions s WHERE s.created_by = a.user_id
                      AND s.id = COALESCE(a.session_scope, substr(a.label, 10))) AS session_exists
             FROM auth_sessions a
             WHERE a.user_id = ? AND a.kind = 'api'
             ORDER BY a.created_at DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("list api tokens: {e}")))?;

        rows.into_iter()
            .map(|row| {
                let label: Option<String> = row.get("label");
                let session_id: Option<Id> = row.get("session_scope");
                let legacy_session_id = if session_id.is_none() {
                    label.as_deref().and_then(legacy_session_label).map(str::to_owned)
                } else { None };
                let session_exists = if session_id.is_some() || legacy_session_id.is_some() {
                    Some(row.get::<i64, _>("session_exists") != 0)
                } else { None };
                Ok(ApiTokenInfo {
                    id: row.get("id"),
                    label,
                    token_prefix: row.get("token_prefix"),
                    created_at: parse_ts(&row.get::<String, _>("created_at"))?,
                    last_seen_at: parse_ts(&row.get::<String, _>("last_seen_at"))?,
                    expires_at: parse_ts(&row.get::<String, _>("expires_at"))?,
                    session_id,
                    legacy_session_id,
                    session_exists,
                })
            })
            .collect()
    }

    /// Count active (unexpired) API tokens across ALL users. Instance-wide, for
    /// the root-only security-posture summary. `expires_at` is stored RFC3339,
    /// which sorts lexicographically with `now`, so a TEXT comparison is exact.
    pub async fn count_active_api_tokens(&self) -> Result<i64> {
        let now = Utc::now().to_rfc3339();
        let row = sqlx::query(
            "SELECT COUNT(*) AS n FROM auth_sessions
             WHERE kind = 'api' AND expires_at > ?",
        )
        .bind(now)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("count active api tokens: {e}")))?;
        Ok(row.get::<i64, _>("n"))
    }

    /// Revoke one of `user_id`'s API tokens by id. Returns whether a row was
    /// deleted (false = not found / not owned / not an API token).
    ///
    /// When a cache is attached, fetches the `token_hash` first (one extra read)
    /// so the cache entry can be evicted by hash. The read is scoped by `user_id`
    /// and `kind='api'` so it cannot accidentally reveal another user's hash.
    pub async fn revoke_api_token(&self, user_id: &Id, id: &Id) -> Result<bool> {
        // Pre-fetch the hash for cache eviction. This is a single indexed lookup
        // and only runs when the cache is present; it is a no-op read otherwise.
        let cached_hash: Option<String> = if self.cache.is_some() {
            let row = sqlx::query(
                "SELECT token_hash FROM auth_sessions
                 WHERE id = ? AND user_id = ? AND kind = 'api'",
            )
            .bind(id)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| Error::Internal(format!("revoke api token lookup: {e}")))?;
            row.map(|r| r.get::<String, _>("token_hash"))
        } else {
            None
        };

        let res =
            sqlx::query("DELETE FROM auth_sessions WHERE id = ? AND user_id = ? AND kind = 'api'")
                .bind(id)
                .bind(user_id)
                .execute(&self.pool)
                .await
                .map_err(|e| Error::Internal(format!("revoke api token: {e}")))?;

        if res.rows_affected() > 0 {
            if let (Some(cache), Some(h)) = (&self.cache, cached_hash) {
                cache.evict(&h);
            }
        }
        Ok(res.rows_affected() > 0)
    }

    /// Mint a scoped **share-link** token: a capability bound to ONE session,
    /// owned by `owner_user_id`, capped at `role` (`Viewer` or `Editor` only).
    /// Returns the RAW token (shown to the caller exactly once) plus its metadata.
    ///
    /// Security shape (mobile plan Task 1.3, design §4.1/§4.4):
    /// - **Never `Admin`** — an `Admin` role is rejected with `Forbidden`; a
    ///   share can never escalate.
    /// - **Short FIXED TTL** — `ttl_secs` is clamped to
    ///   `[SHARE_TOKEN_TTL_MIN_SECS, SHARE_TOKEN_TTL_MAX_SECS]` and `expires_at`
    ///   is `now + ttl`; [`authenticate`] never slides it.
    /// - Stored hashed exactly like a PAT (`kind='share'`, `session_scope`,
    ///   `scope_role`, `revoked=0`); the raw 64-hex token never persists.
    ///
    /// The share's effective identity is the **owner who minted it** (the
    /// `user_id`) — this is *not* impersonation; containment to the one session
    /// is provided entirely by the scope the [`authenticate`] path attaches.
    pub async fn issue_share_token(
        &self,
        owner_user_id: &Id,
        session_id: &Id,
        role: WorkspaceRole,
        ttl_secs: i64,
        label: Option<String>,
    ) -> Result<(String, ShareInfo)> {
        // A share is Viewer or Editor only — never Admin (no escalation).
        if role == WorkspaceRole::Admin {
            return Err(Error::Forbidden(
                "a share link cannot grant Admin role".into(),
            ));
        }
        // Clamp the TTL to a sane fixed window: never longer than the 24h ceiling
        // (a public capability URL), never shorter than the floor.
        let ttl_secs = ttl_secs.clamp(SHARE_TOKEN_TTL_MIN_SECS, SHARE_TOKEN_TTL_MAX_SECS);

        let mut buf = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut buf);
        let token = hex::encode(buf);
        let prefix: String = token.chars().take(12).collect();
        let id = new_id();
        let now = Utc::now();
        let expires_at = now + Duration::seconds(ttl_secs);
        sqlx::query(
            "INSERT INTO auth_sessions
               (id, user_id, token_hash, created_at, expires_at, last_seen_at,
                kind, label, token_prefix, session_scope, scope_role, revoked)
             VALUES (?, ?, ?, ?, ?, ?, 'share', ?, ?, ?, ?, 0)",
        )
        .bind(&id)
        .bind(owner_user_id)
        .bind(token_hash(&token))
        .bind(now.to_rfc3339())
        .bind(expires_at.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(&label)
        .bind(&prefix)
        .bind(session_id)
        .bind(role.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("issue share token: {e}")))?;
        Ok((
            token,
            ShareInfo {
                id,
                session_id: session_id.clone(),
                role,
                token_prefix: prefix,
                label,
                created_at: now,
                expires_at,
            },
        ))
    }

    /// Mint a scoped share-link token **gated by an emailed OTP** (mobile plan
    /// Task 7.2 / design addendum "Email-OTP gate for share links").
    ///
    /// Like [`issue_share_token`] but additionally:
    /// - generates a **6-digit OTP** from `OsRng` and stores only its SHA-256
    ///   (`otp_hash`) — the plaintext is returned exactly once so the caller can
    ///   email it (the DB never holds the raw code);
    /// - locks the share to `recipient_email` (immutable; Task 7.4 extension only
    ///   ever re-emails this address);
    /// - sets `otp_expires_at = now + SHARE_OTP_TTL_SECS` (~10 min) and
    ///   `max_expires_at = now + min(duration_secs, SHARE_OTP_WINDOW_MAX_SECS)`
    ///   (≤12h) — the session window the guest gets once verified;
    /// - leaves `verified_at = NULL` so the share is **OTP-pending** until the
    ///   guest redeems the code via `verify_share_otp`.
    ///
    /// The scoped-token `expires_at` (the bearer-token TTL the base
    /// [`authenticate`] checks) is set to the same window end so the token row
    /// can never outlive its session window. Returns the RAW token, the RAW OTP
    /// (caller emails it), and the [`ShareInfo`] metadata.
    #[allow(clippy::too_many_arguments)]
    pub async fn issue_share_otp_token(
        &self,
        owner_user_id: &Id,
        session_id: &Id,
        role: WorkspaceRole,
        duration_secs: i64,
        label: Option<String>,
        recipient_email: &str,
    ) -> Result<(String, String, ShareInfo)> {
        // A share is Viewer or Editor only — never Admin (no escalation).
        if role == WorkspaceRole::Admin {
            return Err(Error::Forbidden(
                "a share link cannot grant Admin role".into(),
            ));
        }
        // Clamp the session window to (0, 12h]. A non-positive request is clamped
        // up to the 60s floor so a freshly-minted share is always usable.
        let window_secs = duration_secs.clamp(SHARE_TOKEN_TTL_MIN_SECS, SHARE_OTP_WINDOW_MAX_SECS);

        let mut buf = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut buf);
        let token = hex::encode(buf);
        let prefix: String = token.chars().take(12).collect();
        let otp = generate_otp();
        let id = new_id();
        let now = Utc::now();
        // The bearer-token TTL and the session window share the same end: the
        // token may never outlive the window it grants.
        let expires_at = now + Duration::seconds(window_secs);
        let max_expires_at = expires_at.timestamp();
        let otp_expires_at = (now + Duration::seconds(SHARE_OTP_TTL_SECS)).timestamp();
        sqlx::query(
            "INSERT INTO auth_sessions
               (id, user_id, token_hash, created_at, expires_at, last_seen_at,
                kind, label, token_prefix, session_scope, scope_role, revoked,
                recipient_email, otp_hash, otp_expires_at, verified_at, max_expires_at)
             VALUES (?, ?, ?, ?, ?, ?, 'share', ?, ?, ?, ?, 0, ?, ?, ?, NULL, ?)",
        )
        .bind(&id)
        .bind(owner_user_id)
        .bind(token_hash(&token))
        .bind(now.to_rfc3339())
        .bind(expires_at.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(&label)
        .bind(&prefix)
        .bind(session_id)
        .bind(role.as_str())
        .bind(recipient_email)
        .bind(crate::passwords::hash_password(&otp)?)
        .bind(otp_expires_at)
        .bind(max_expires_at)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("issue share otp token: {e}")))?;
        Ok((
            token,
            otp,
            ShareInfo {
                id,
                session_id: session_id.clone(),
                role,
                token_prefix: prefix,
                label,
                created_at: now,
                expires_at,
            },
        ))
    }

    /// Redeem an emailed OTP for a share token (mobile plan Task 7.3 /
    /// `POST /api/v1/share/verify`). On success sets `verified_at = now` and
    /// **clears `otp_hash`** so the same code can never be reused (single-use).
    ///
    /// Returns `Ok(true)` when the code matched a live, OTP-pending share and the
    /// share is now verified; `Ok(false)` when the token isn't an OTP-gated share
    /// or has no live code. Rejects (`Ok(false)`) when:
    /// - the share has no `otp_hash` (already redeemed / never had one);
    /// - the code is expired (`otp_expires_at <= now`);
    /// - the hash does not match.
    ///
    /// The presented `token` is the auth (the caller must hold the share link);
    /// the comparison is constant-shape (both sides SHA-256 hex). The caller is
    /// responsible for IP rate-limiting (the share throttle) around this call.
    pub async fn verify_share_otp(&self, token: &str, otp: &str) -> Result<bool> {
        let hash = token_hash(token);
        let row = sqlx::query(
            "SELECT otp_hash, otp_expires_at, revoked, recipient_email
             FROM auth_sessions
             WHERE token_hash = ? AND kind = 'share'",
        )
        .bind(&hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("verify share otp lookup: {e}")))?;

        let Some(row) = row else { return Ok(false) };
        // A revoked share never verifies; an OTP-less share (plain or already
        // redeemed) has nothing to match.
        if row.get::<i64, _>("revoked") != 0 {
            return Ok(false);
        }
        let recipient: Option<String> = row.get("recipient_email");
        if recipient.is_none() {
            return Ok(false); // not an OTP-gated share
        }
        let stored_hash: Option<String> = row.get("otp_hash");
        let Some(stored_hash) = stored_hash else {
            return Ok(false); // already redeemed (single-use) — no code to match
        };
        let otp_expires_at: Option<i64> = row.get("otp_expires_at");
        let now = Utc::now().timestamp();
        if otp_expires_at.map(|e| e <= now).unwrap_or(true) {
            return Ok(false); // expired code
        }
        // argon2id, not a plain digest: a 6-digit code behind a fast hash is a
        // one-million-guess offline job for anyone who reads the row.
        if !crate::passwords::verify_password(otp, &stored_hash).unwrap_or(false) {
            return Ok(false); // wrong code
        }

        // Match: mark verified and CLEAR the code (single-use). Guard the UPDATE
        // on `otp_hash` still being the matched value so two racing verifies
        // can't both succeed on the same code.
        let res = sqlx::query(
            "UPDATE auth_sessions
             SET verified_at = ?, otp_hash = NULL
             WHERE token_hash = ? AND kind = 'share' AND otp_hash = ?",
        )
        .bind(now)
        .bind(&hash)
        .bind(&stored_hash)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("verify share otp update: {e}")))?;

        Ok(res.rows_affected() > 0)
    }

    /// Extend an email-OTP share with a **fresh** code, re-emailed to the LOCKED
    /// original recipient (mobile plan Task 7.4 / `POST /api/v1/share/extend`).
    ///
    /// The recipient address is **immutable on extend** — it is read from the row
    /// and returned to the caller so the route emails the new code there and
    /// NOWHERE else. There is no parameter to redirect delivery; this is the
    /// locked-recipient guarantee that prevents access hijack to another mailbox.
    ///
    /// On a matching, live OTP share this:
    /// - generates a new 6-digit OTP and stores only its SHA-256 (`otp_hash`);
    /// - sets `otp_expires_at = now + SHARE_OTP_TTL_SECS` (~10 min);
    /// - **clears `verified_at`** so the share is OTP-pending again (the guest
    ///   must re-verify the new code before it re-opens);
    /// - opens a **fresh ≤12h window**: `max_expires_at` and the bearer-token
    ///   `expires_at` are both pushed to `now + min(window, SHARE_OTP_WINDOW_MAX_SECS)`,
    ///   where `window` is the share's original duration (reconstructed from the
    ///   row's `created_at`→`expires_at` span), clamped so each granted window
    ///   stays ≤12h.
    ///
    /// Returns `Ok(Some((new_otp, recipient_email, owner_user_id)))` on success
    /// (the caller emails the code to that recipient via the owner's verified
    /// sender); `Ok(None)` when the token is not an extendable OTP share (no row /
    /// not `kind='share'` / no `recipient_email` / revoked) — the route maps
    /// `None` to a `400`. The raw OTP is returned exactly once; the DB only ever
    /// holds its hash.
    pub async fn extend_share_otp(&self, token: &str) -> Result<Option<(String, String, Id)>> {
        let hash = token_hash(token);
        let row = sqlx::query(
            "SELECT user_id, recipient_email, revoked, created_at, expires_at
             FROM auth_sessions
             WHERE token_hash = ? AND kind = 'share'",
        )
        .bind(&hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("extend share otp lookup: {e}")))?;

        let Some(row) = row else { return Ok(None) };
        // A revoked share is not extendable.
        if row.get::<i64, _>("revoked") != 0 {
            return Ok(None);
        }
        // Only OTP shares (those locked to a recipient) are extendable.
        let recipient: Option<String> = row.get("recipient_email");
        let Some(recipient) = recipient else {
            return Ok(None); // plain (non-OTP) share — nothing to re-email
        };
        let owner_id: Id = Id::from(row.get::<String, _>("user_id"));

        // Reconstruct the original session-window duration from the row's span so
        // the extension grants the SAME length window the share was minted with,
        // re-clamped to ≤12h (so each granted window stays bounded).
        let created_at = parse_ts(&row.get::<String, _>("created_at"))?;
        let prev_expires_at = parse_ts(&row.get::<String, _>("expires_at"))?;
        let original_window = (prev_expires_at - created_at).num_seconds();
        let window_secs =
            original_window.clamp(SHARE_TOKEN_TTL_MIN_SECS, SHARE_OTP_WINDOW_MAX_SECS);

        let otp = generate_otp();
        let now = Utc::now();
        // Fresh ≤12h window; the bearer-token TTL tracks it so the token can never
        // outlive the window it grants.
        let expires_at = now + Duration::seconds(window_secs);
        let max_expires_at = expires_at.timestamp();
        let otp_expires_at = (now + Duration::seconds(SHARE_OTP_TTL_SECS)).timestamp();

        let res = sqlx::query(
            "UPDATE auth_sessions
             SET otp_hash = ?, otp_expires_at = ?, verified_at = NULL,
                 max_expires_at = ?, expires_at = ?
             WHERE token_hash = ? AND kind = 'share' AND revoked = 0
                   AND recipient_email IS NOT NULL",
        )
        .bind(crate::passwords::hash_password(&otp)?)
        .bind(otp_expires_at)
        .bind(max_expires_at)
        .bind(expires_at.to_rfc3339())
        .bind(&hash)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("extend share otp update: {e}")))?;

        if res.rows_affected() == 0 {
            // Lost a race (revoked / recipient cleared between read and write).
            return Ok(None);
        }
        Ok(Some((otp, recipient, owner_id)))
    }

    /// List the **live** (non-revoked, non-expired) share tokens for one session,
    /// newest first. Metadata only — never the secret.
    pub async fn list_shares_for_session(&self, session_id: &Id) -> Result<Vec<ShareInfo>> {
        let now = Utc::now().to_rfc3339();
        let rows = sqlx::query(
            "SELECT id, session_scope, scope_role, token_prefix, label, created_at, expires_at
             FROM auth_sessions
             WHERE kind = 'share' AND revoked = 0 AND session_scope = ? AND expires_at > ?
             ORDER BY created_at DESC",
        )
        .bind(session_id)
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("list shares for session: {e}")))?;

        rows.into_iter()
            .map(|row| {
                let scope_role: String = row.get("scope_role");
                let role = WorkspaceRole::parse(&scope_role)
                    .ok_or_else(|| Error::Internal(format!("bad scope_role '{scope_role}'")))?;
                Ok(ShareInfo {
                    id: row.get("id"),
                    session_id: Id::from(row.get::<String, _>("session_scope")),
                    role,
                    token_prefix: row.get("token_prefix"),
                    label: row.get("label"),
                    created_at: parse_ts(&row.get::<String, _>("created_at"))?,
                    expires_at: parse_ts(&row.get::<String, _>("expires_at"))?,
                })
            })
            .collect()
    }

    /// Revoke one of `owner_user_id`'s share tokens by id (flips `revoked=1`).
    /// Owner-scoped and kind-scoped: it never touches another user's row or a
    /// non-share token. Idempotent (a second revoke is a no-op).
    pub async fn revoke_share(&self, owner_user_id: &Id, share_id: &str) -> Result<()> {
        sqlx::query(
            "UPDATE auth_sessions SET revoked = 1
             WHERE id = ? AND user_id = ? AND kind = 'share'",
        )
        .bind(share_id)
        .bind(owner_user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("revoke share: {e}")))?;
        Ok(())
    }

    /// Revoke **all** of `owner_user_id`'s live share tokens (flip `revoked=1`).
    /// Returns the distinct `session_scope` ids whose share rows were revoked, so
    /// the caller can evict any still-attached viewers for those sessions.
    /// Owner-scoped: never touches another user's tokens.
    pub async fn revoke_all_shares_for_user(&self, owner_user_id: &Id) -> Result<Vec<Id>> {
        // Collect the session ids we are about to revoke BEFORE the update, so
        // the caller can evict attached viewers for those sessions.
        let rows = sqlx::query(
            "SELECT DISTINCT session_scope FROM auth_sessions
             WHERE user_id = ? AND kind = 'share' AND revoked = 0",
        )
        .bind(owner_user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("list user shares: {e}")))?;

        let session_ids: Vec<Id> = rows
            .iter()
            .filter_map(|r| r.get::<Option<String>, _>("session_scope"))
            .collect();

        sqlx::query(
            "UPDATE auth_sessions SET revoked = 1
             WHERE user_id = ? AND kind = 'share' AND revoked = 0",
        )
        .bind(owner_user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("revoke all shares: {e}")))?;

        Ok(session_ids)
    }

    /// Look up the `session_scope` (session id) for a single share row by its
    /// `share_id` and `owner_user_id`. Returns `None` when the share doesn't
    /// exist or isn't owned by this user. Used by the revoke-one handler to
    /// locate which session to evict.
    pub async fn share_session_id(&self, owner_user_id: &Id, share_id: &str) -> Result<Option<Id>> {
        let row = sqlx::query(
            "SELECT session_scope FROM auth_sessions
             WHERE id = ? AND user_id = ? AND kind = 'share'",
        )
        .bind(share_id)
        .bind(owner_user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Internal(format!("share_session_id: {e}")))?;

        Ok(row.and_then(|r| r.get::<Option<String>, _>("session_scope")))
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

    async fn seed_managed_session(pool: &SqlitePool, owner: &Id) -> Id {
        let ws = new_id();
        let sid = new_id();
        let now = Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, 'test', '/tmp', ?)")
            .bind(&ws).bind(&now).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO sessions (id, workspace_id, kind, provider, title, status, cwd, created_by, created_at, last_active_at) VALUES (?, ?, 'agent', 'claude', 'test', 'idle', '/tmp', ?, ?, ?)")
            .bind(&sid).bind(&ws).bind(owner).bind(&now).bind(&now)
            .execute(pool).await.unwrap();
        sid
    }

    #[tokio::test]
    async fn token_inventory_distinguishes_durable_ownership_from_legacy_label_candidates() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let owner = seed_user(&pool, "inventory").await;
        let sid = seed_managed_session(&pool, &owner).await;
        let (_, managed) = repo.issue_session_api_token(&owner, &sid).await.unwrap();
        let (legacy_raw, legacy) = repo.issue_api_token(&owner, Some(&format!("otto-mcp:{sid}"))).await.unwrap();
        let (_, personal) = repo.issue_api_token(&owner, Some("CI deploy")).await.unwrap();
        let list = repo.list_api_tokens(&owner).await.unwrap();
        let managed = list.iter().find(|t| t.id == managed.id).unwrap();
        assert_eq!(managed.session_id.as_ref(), Some(&sid));
        assert_eq!(managed.session_exists, Some(true));
        let legacy = list.iter().find(|t| t.id == legacy.id).unwrap();
        assert!(legacy.session_id.is_none());
        assert_eq!(legacy.legacy_session_id.as_ref(), Some(&sid));
        assert_eq!(legacy.session_exists, Some(true));
        assert_eq!(list.iter().find(|t| t.id == personal.id).unwrap().session_exists, None);
        sqlx::query("DELETE FROM sessions WHERE id = ?").bind(&sid).execute(&pool).await.unwrap();
        let list = repo.list_api_tokens(&owner).await.unwrap();
        assert_eq!(list.iter().filter(|t| t.session_exists == Some(false)).count(), 2);
        assert!(repo.authenticate(&legacy_raw).await.is_ok(), "a matching label is not durable ownership or permission to automatically revoke");
    }

    #[tokio::test]
    async fn managed_token_origin_distinguishes_agent_calls_from_personal_tokens() {
        let pool = mem_pool().await;
        let (repo, _) = cached_repo(pool.clone());
        let owner = seed_user(&pool, "managed_origin").await;
        let sid = seed_managed_session(&pool, &owner).await;
        let (agent, _) = repo.issue_session_api_token(&owner, &sid).await.unwrap();
        let (personal, _) = repo.issue_api_token(&owner, Some(&format!("otto-mcp:{sid}"))).await.unwrap();
        for _ in 0..2 {
            assert_eq!(repo.authenticate(&agent).await.unwrap().managed_session_id, Some(sid.clone()));
            assert_eq!(repo.authenticate(&personal).await.unwrap().managed_session_id, None);
        }
    }

    #[tokio::test]
    async fn managed_session_token_rejects_deleted_session_even_after_authentication() {
        let pool = mem_pool().await;
        let (repo, _) = cached_repo(pool.clone());
        let uid = seed_user(&pool, "managed_deleted").await;
        let sid = seed_managed_session(&pool, &uid).await;
        let (token, info) = repo.issue_api_token(&uid, Some("managed fixture")).await.unwrap();
        sqlx::query("UPDATE auth_sessions SET session_scope = ? WHERE id = ?")
            .bind(&sid).bind(&info.id).execute(&pool).await.unwrap();
        assert!(repo.authenticate(&token).await.is_ok());
        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(&sid).execute(&pool).await.unwrap();
        assert!(matches!(repo.authenticate(&token).await, Err(Error::Unauthorized)),
            "a deleted session must not retain API access through its managed token");
    }

    #[tokio::test]
    async fn managed_session_tokens_revoke_across_restart_without_touching_personal_tokens() {
        let pool = mem_pool().await;
        let (repo, cache) = cached_repo(pool.clone());
        let owner = seed_user(&pool, "managed_owner").await;
        let other = seed_user(&pool, "managed_other").await;
        let sid = seed_managed_session(&pool, &owner).await;
        let (first, _) = repo.issue_session_api_token(&owner, &sid).await.unwrap();
        let (second, _) = repo.issue_session_api_token(&owner, &sid).await.unwrap();
        let (personal, _) = repo.issue_api_token(&owner, Some(&format!("otto-mcp:{sid}"))).await.unwrap();
        assert!(repo.authenticate(&first).await.is_ok());
        assert!(repo.authenticate(&second).await.is_ok());
        assert!(repo.issue_session_api_token(&other, &sid).await.is_err());
        assert_eq!(repo.revoke_session_tokens(&other, &sid).await.unwrap(), 0);
        let restarted = AuthRepo::with_cache(pool, cache);
        assert_eq!(restarted.revoke_session_tokens(&owner, &sid).await.unwrap(), 2);
        assert!(matches!(repo.authenticate(&first).await, Err(Error::Unauthorized)));
        assert!(matches!(repo.authenticate(&second).await, Err(Error::Unauthorized)));
        assert!(repo.authenticate(&personal).await.is_ok(), "labels do not confer lifecycle ownership");
    }

    /// Boot sweep: managed credentials of sessions whose processes died with
    /// the previous daemon stop authenticating, rows kept; personal tokens
    /// and share links are untouched.
    #[tokio::test]
    async fn boot_sweep_expires_managed_session_tokens_only() {
        let pool = mem_pool().await;
        let (repo, _) = cached_repo(pool.clone());
        let owner = seed_user(&pool, "boot_managed").await;
        let sid = seed_managed_session(&pool, &owner).await;
        let (managed, info) = repo.issue_session_api_token(&owner, &sid).await.unwrap();
        let (personal, _) = repo.issue_api_token(&owner, Some("CI deploy")).await.unwrap();
        assert!(repo.authenticate(&managed).await.is_ok());

        assert_eq!(repo.expire_managed_session_tokens().await.unwrap(), 1);
        assert!(matches!(repo.authenticate(&managed).await, Err(Error::Unauthorized)));
        assert!(repo.authenticate(&personal).await.is_ok());
        let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM auth_sessions WHERE id = ?")
            .bind(&info.id).fetch_one(&pool).await.unwrap();
        assert_eq!(kept, 1, "the sweep revokes, it does not delete rows");
        // Idempotent.
        assert_eq!(repo.expire_managed_session_tokens().await.unwrap(), 0);
        // A resume mints a fresh, working credential.
        let (fresh, _) = repo.issue_session_api_token(&owner, &sid).await.unwrap();
        assert!(repo.authenticate(&fresh).await.is_ok());
    }

    /// Boot sweep of legacy `otto-mcp:<session>` tokens: pre-cutover Otto
    /// labels die (session gone or the owner's own); a later hand-named token,
    /// a non-ULID label and a label naming ANOTHER user's session survive.
    #[tokio::test]
    async fn boot_sweep_expires_pre_cutover_legacy_label_tokens() {
        let pool = mem_pool().await;
        let (repo, _) = cached_repo(pool.clone());
        let owner = seed_user(&pool, "legacy_owner").await;
        let other = seed_user(&pool, "legacy_other").await;
        let own_sid = seed_managed_session(&pool, &owner).await;
        let others_sid = seed_managed_session(&pool, &other).await;
        let gone_sid = new_id();
        let old = "2026-09-01T00:00:00+00:00";
        let cutover = DateTime::parse_from_rfc3339("2026-09-23T00:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc);

        let mint_old = |label: String| {
            let repo = repo.clone();
            let pool = pool.clone();
            let owner = owner.clone();
            async move {
                let (raw, info) = repo.issue_api_token(&owner, Some(&label)).await.unwrap();
                sqlx::query("UPDATE auth_sessions SET created_at = ? WHERE id = ?")
                    .bind(old).bind(&info.id).execute(&pool).await.unwrap();
                raw
            }
        };
        let gone = mint_old(format!("otto-mcp:{gone_sid}")).await;
        let own = mint_old(format!("otto-mcp:{own_sid}")).await;
        let foreign = mint_old(format!("otto-mcp:{others_sid}")).await;
        let not_ulid = mint_old("otto-mcp:my-laptop".to_string()).await;
        // Named like a session token AFTER the cutover: a human's choice.
        let (recent, _) = repo
            .issue_api_token(&owner, Some(&format!("otto-mcp:{gone_sid}")))
            .await
            .unwrap();

        assert_eq!(repo.expire_legacy_session_label_tokens(cutover).await.unwrap(), 2);
        assert!(matches!(repo.authenticate(&gone).await, Err(Error::Unauthorized)));
        assert!(matches!(repo.authenticate(&own).await, Err(Error::Unauthorized)));
        assert!(repo.authenticate(&foreign).await.is_ok());
        assert!(repo.authenticate(&not_ulid).await.is_ok());
        assert!(repo.authenticate(&recent).await.is_ok());
        assert_eq!(repo.expire_legacy_session_label_tokens(cutover).await.unwrap(), 0);
    }

    /// Interactive login token: mint → authenticate → revoke → auth fails.
    #[tokio::test]
    async fn session_token_lifecycle() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let uid = seed_user(&pool, "alice").await;

        let token = repo.issue(&uid).await.unwrap();
        let ctx = repo.authenticate(&token).await.unwrap();
        assert_eq!(ctx.effective_user.id, uid);
        // Normal token: real == effective (the plumbing is a no-op today).
        assert_eq!(ctx.real_user.id, ctx.effective_user.id);

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
        assert_eq!(
            info.token_prefix,
            token.chars().take(12).collect::<String>()
        );

        // It shows up in the user's token list (metadata only, never the secret).
        let listed = repo.list_api_tokens(&uid).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, info.id);
        assert_eq!(listed[0].token_prefix, info.token_prefix);

        // It authenticates like any bearer token.
        let ctx = repo.authenticate(&token).await.unwrap();
        assert_eq!(ctx.effective_user.id, uid);
        assert_eq!(ctx.real_user.id, ctx.effective_user.id);

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

    /// A scoped MCP token: mint with a non-trivial [`McpScope`] → authenticate →
    /// the resolved [`AuthContext`] carries `mcp_only` AND the exact scope → it
    /// appears in the cross-user list with the owning username → revoke by id.
    #[tokio::test]
    async fn mcp_token_scope_round_trips_through_authenticate() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let uid = seed_user(&pool, "carol").await;

        let scope = McpScope {
            tools: Some(vec!["list_workflows".into(), "get_workflow".into()]),
            allow_writes: false,
            workspace_id: Some("ws-1".into()),
        };
        let (token, info) = repo
            .issue_mcp_token_with_scope(&uid, Some("ci-readonly"), &scope)
            .await
            .unwrap();
        assert_eq!(info.user_id, uid);
        assert_eq!(info.username, "carol");
        assert_eq!(info.scope, scope);
        assert_eq!(
            info.token_prefix,
            token.chars().take(12).collect::<String>()
        );

        // authenticate() resolves the per-token scope onto the AuthContext.
        let ctx = repo.authenticate(&token).await.unwrap();
        assert!(ctx.mcp_only, "an mcp token must be mcp_only");
        let resolved = ctx.mcp_scope.expect("mcp token must carry a scope");
        assert_eq!(resolved, scope);
        // The scope actually denies an out-of-scope / mutating tool.
        assert!(resolved.deny_reason("list_repos", false, None).is_some());
        assert!(resolved.deny_reason("run_workflow", true, None).is_some());
        assert!(resolved
            .deny_reason("list_workflows", false, Some("ws-1"))
            .is_none());

        // It is listed across users with owner metadata (never the secret).
        let listed = repo.list_mcp_tokens().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, info.id);
        assert_eq!(listed[0].username, "carol");
        assert_eq!(listed[0].scope, scope);

        // Revoke by id; it stops authenticating and leaves the list empty.
        assert!(repo.revoke_mcp_token_by_id(&info.id).await.unwrap());
        assert!(
            matches!(repo.authenticate(&token).await, Err(Error::Unauthorized)),
            "revoked mcp token must no longer authenticate"
        );
        assert!(repo.list_mcp_tokens().await.unwrap().is_empty());
        assert!(!repo.revoke_mcp_token_by_id(&info.id).await.unwrap());
    }

    #[tokio::test]
    async fn rotate_mcp_token_rotates_only_the_selected_token() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let alice = seed_user(&pool, "rotate-alice").await;
        let bob = seed_user(&pool, "rotate-bob").await;
        let a1_scope = McpScope {
            tools: Some(vec!["list_workflows".into()]),
            allow_writes: false,
            workspace_id: Some("ws-a".into()),
        };
        let a2_scope = McpScope {
            tools: Some(vec!["get_workflow".into()]),
            allow_writes: true,
            workspace_id: None,
        };
        let b1_scope = McpScope::unrestricted();
        let (a1_token, a1_info) = repo
            .issue_mcp_token_with_scope(&alice, Some("a1"), &a1_scope)
            .await
            .unwrap();
        let (a2_token, a2_info) = repo
            .issue_mcp_token_with_scope(&alice, Some("a2"), &a2_scope)
            .await
            .unwrap();
        let (b1_token, b1_info) = repo
            .issue_mcp_token_with_scope(&bob, Some("b1"), &b1_scope)
            .await
            .unwrap();

        let (new_a1_token, new_a1_info) = repo
            .rotate_mcp_token(&a1_info.id)
            .await
            .unwrap()
            .expect("selected MCP token exists");
        assert_ne!(new_a1_info.id, a1_info.id);
        assert_ne!(new_a1_info.token_prefix, a1_info.token_prefix);
        assert_eq!(new_a1_info.user_id, alice);
        assert_eq!(new_a1_info.label.as_deref(), Some("a1"));
        assert_eq!(new_a1_info.scope, a1_scope);

        assert!(matches!(
            repo.authenticate(&a1_token).await,
            Err(Error::Unauthorized)
        ));
        assert_eq!(
            repo.authenticate(&new_a1_token).await.unwrap().mcp_scope,
            Some(a1_scope)
        );
        assert_eq!(
            repo.authenticate(&a2_token)
                .await
                .unwrap()
                .effective_user
                .id,
            alice
        );
        assert_eq!(
            repo.authenticate(&b1_token)
                .await
                .unwrap()
                .effective_user
                .id,
            bob
        );

        let mut listed_ids: Vec<Id> = repo
            .list_mcp_tokens()
            .await
            .unwrap()
            .into_iter()
            .map(|info| info.id)
            .collect();
        listed_ids.sort();
        let mut expected_ids = vec![new_a1_info.id, a2_info.id, b1_info.id];
        expected_ids.sort();
        assert_eq!(listed_ids, expected_ids);
    }

    #[tokio::test]
    async fn rotate_mcp_token_unknown_id_is_none() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool);

        assert!(repo
            .rotate_mcp_token(&Id::from("unknown-mcp-token"))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn revoke_mcp_tokens_leaves_scoped_tokens_alone() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let alice = seed_user(&pool, "legacy-alice").await;
        let legacy = repo
            .issue_mcp_token(&alice, Some(LEGACY_OTTO_MCP_SERVER_LABEL))
            .await
            .unwrap();
        let (scoped, scoped_info) = repo
            .issue_mcp_token_with_scope(
                &alice,
                Some("ci"),
                &McpScope {
                    tools: Some(vec!["list_workflows".into()]),
                    allow_writes: false,
                    workspace_id: None,
                },
            )
            .await
            .unwrap();

        assert_eq!(repo.revoke_mcp_tokens(&alice).await.unwrap(), 1);
        assert!(matches!(
            repo.authenticate(&legacy).await,
            Err(Error::Unauthorized)
        ));
        assert_eq!(
            repo.authenticate(&scoped).await.unwrap().effective_user.id,
            alice
        );
        assert_eq!(repo.list_mcp_tokens().await.unwrap()[0].id, scoped_info.id);
    }

    #[tokio::test]
    async fn vault_reviewer_token_is_server_bound_and_read_only() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let uid = seed_user(&pool, "vault-reviewer").await;
        let session_id = Id::from("review-session-1");
        let workspace_id = Id::from("workspace-1");

        let (token, token_id) = repo
            .issue_vault_reviewer_token(&uid, Some("vault-review"), &session_id, &workspace_id)
            .await
            .unwrap();

        let ctx = repo.authenticate(&token).await.unwrap();
        assert!(
            ctx.mcp_only,
            "reviewer token must be denied on direct feature routes"
        );
        assert!(
            ctx.mcp_internal,
            "reviewer token must use the internal governed MCP path"
        );
        assert_eq!(ctx.mcp_session_id.as_ref(), Some(&session_id));
        let scope = ctx
            .mcp_scope
            .expect("reviewer token carries an immutable scope");
        assert_eq!(scope.workspace_id.as_ref(), Some(&workspace_id));
        assert!(scope
            .deny_reason("vault_read", false, Some("workspace-1"))
            .is_none());
        assert!(scope
            .deny_reason("vault_write", true, Some("workspace-1"))
            .is_some());
        assert!(scope
            .deny_reason("vault_read", false, Some("workspace-2"))
            .is_some());

        assert!(repo
            .revoke_vault_reviewer_token(&uid, &token_id)
            .await
            .unwrap());
        assert!(matches!(
            repo.authenticate(&token).await,
            Err(Error::Unauthorized)
        ));
    }

    #[tokio::test]
    async fn vault_reviewer_token_fails_closed_when_persisted_scope_is_missing() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let uid = seed_user(&pool, "vault-reviewer-corrupt").await;
        let (token, token_id) = repo
            .issue_vault_reviewer_token(
                &uid,
                None,
                &Id::from("review-session-corrupt"),
                &Id::from("workspace-corrupt"),
            )
            .await
            .unwrap();
        sqlx::query("UPDATE auth_sessions SET mcp_scope = NULL WHERE id = ?")
            .bind(token_id)
            .execute(&pool)
            .await
            .unwrap();

        assert!(matches!(
            repo.authenticate(&token).await,
            Err(Error::Unauthorized)
        ));
    }

    /// A legacy `issue_mcp_token` (no explicit scope) resolves to the unrestricted
    /// scope, preserving pre-migration behaviour (full access, writes allowed).
    #[tokio::test]
    async fn legacy_mcp_token_is_unrestricted() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let uid = seed_user(&pool, "dave").await;

        let token = repo.issue_mcp_token(&uid, Some("legacy")).await.unwrap();
        let ctx = repo.authenticate(&token).await.unwrap();
        let scope = ctx.mcp_scope.expect("mcp token carries a scope");
        assert_eq!(scope, McpScope::unrestricted());
        // Unrestricted ⇒ even a mutating tool passes the scope gate.
        assert!(scope
            .deny_reason("run_workflow", true, Some("any"))
            .is_none());
    }

    /// Multiple MCP tokens for DIFFERENT users coexist, each with its own access.
    #[tokio::test]
    async fn multiple_mcp_tokens_per_user_coexist() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let alice = seed_user(&pool, "alice2").await;
        let bob = seed_user(&pool, "bob2").await;

        let ro = McpScope {
            tools: None,
            allow_writes: false,
            workspace_id: None,
        };
        let rw = McpScope::unrestricted();
        let (t_alice, _) = repo
            .issue_mcp_token_with_scope(&alice, Some("alice-ro"), &ro)
            .await
            .unwrap();
        let (t_bob, _) = repo
            .issue_mcp_token_with_scope(&bob, Some("bob-rw"), &rw)
            .await
            .unwrap();

        // Alice's token authenticates as Alice and is read-only.
        let ca = repo.authenticate(&t_alice).await.unwrap();
        assert_eq!(ca.effective_user.id, alice);
        assert!(ca
            .mcp_scope
            .unwrap()
            .deny_reason("run_workflow", true, None)
            .is_some());
        // Bob's token authenticates as Bob and permits writes.
        let cb = repo.authenticate(&t_bob).await.unwrap();
        assert_eq!(cb.effective_user.id, bob);
        assert!(cb
            .mcp_scope
            .unwrap()
            .deny_reason("run_workflow", true, None)
            .is_none());

        // Both are visible in the cross-user list.
        assert_eq!(repo.list_mcp_tokens().await.unwrap().len(), 2);
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
        assert_eq!(
            repo.authenticate(&token).await.unwrap().effective_user.id,
            owner
        );
    }

    /// Seed a user with an explicit `disabled` flag; returns its id.
    async fn seed_user_disabled(pool: &SqlitePool, username: &str, disabled: bool) -> Id {
        let id = new_id();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, disabled, created_at)
             VALUES (?, ?, ?, ?, 0, ?, ?)",
        )
        .bind(&id)
        .bind(username)
        .bind("hash")
        .bind("Test User")
        .bind(disabled as i64)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        id
    }

    /// Impersonation token: mint → authenticate resolves real=admin /
    /// effective=target → revoke (stop) → auth fails.
    #[tokio::test]
    async fn impersonation_token_resolves_real_and_effective() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let admin = seed_user(&pool, "admin").await;
        let target = seed_user(&pool, "target").await;

        let token = repo
            .issue_impersonation_token(&admin, &target, Duration::minutes(30))
            .await
            .unwrap();

        let ctx = repo.authenticate(&token).await.unwrap();
        // Authorization runs against the target; audit records the admin.
        assert_eq!(ctx.real_user.id, admin, "real_user is the admin (audit)");
        assert_eq!(
            ctx.effective_user.id, target,
            "effective_user is the target (authz)"
        );

        // `stop` revokes the presented impersonation token.
        repo.revoke(&token).await.unwrap();
        assert!(
            matches!(repo.authenticate(&token).await, Err(Error::Unauthorized)),
            "revoked impersonation token must no longer authenticate"
        );
    }

    /// An impersonation token whose TTL has elapsed is rejected (it is a SHORT
    /// fixed window and is never slid).
    #[tokio::test]
    async fn impersonation_token_expires() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let admin = seed_user(&pool, "admin").await;
        let target = seed_user(&pool, "target").await;

        // Negative TTL ⇒ already expired at mint time.
        let token = repo
            .issue_impersonation_token(&admin, &target, Duration::minutes(-1))
            .await
            .unwrap();
        assert!(
            matches!(repo.authenticate(&token).await, Err(Error::Unauthorized)),
            "an expired impersonation token must not authenticate"
        );
    }

    /// A disabled target user invalidates an otherwise-valid impersonation token
    /// (the effective identity must be a live account).
    #[tokio::test]
    async fn impersonation_rejects_disabled_target() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let admin = seed_user(&pool, "admin").await;
        let target = seed_user_disabled(&pool, "target", true).await;

        let token = repo
            .issue_impersonation_token(&admin, &target, Duration::minutes(30))
            .await
            .unwrap();
        assert!(
            matches!(repo.authenticate(&token).await, Err(Error::Unauthorized)),
            "impersonating a disabled target must be rejected"
        );
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

    // ---- share-link (scoped) tokens (mobile plan Task 1.3) ----------------

    use otto_core::domain::WorkspaceRole;

    /// Read the raw stored `expires_at` for the row matching `token`, so tests
    /// can prove the value is (or is not) advanced by `authenticate`.
    async fn stored_expires_at(pool: &SqlitePool, token: &str) -> String {
        let row = sqlx::query("SELECT expires_at FROM auth_sessions WHERE token_hash = ?")
            .bind(token_hash(token))
            .fetch_one(pool)
            .await
            .unwrap();
        row.get::<String, _>("expires_at")
    }

    /// A viewer share authenticates into a SCOPED context: real == effective ==
    /// the minting owner (NOT impersonation), and `scope` carries the one
    /// session id + the capped Viewer role. The 12-char prefix is preserved.
    #[tokio::test]
    async fn share_token_authenticates_with_viewer_scope() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let owner = seed_user(&pool, "owner").await;

        let (raw, info) = repo
            .issue_share_token(&owner, &Id::from("S1"), WorkspaceRole::Viewer, 3600, None)
            .await
            .unwrap();

        // Metadata mirrors the row; the prefix is exactly the first 12 chars.
        assert_eq!(info.session_id, Id::from("S1"));
        assert_eq!(info.role, WorkspaceRole::Viewer);
        assert_eq!(info.token_prefix.len(), 12);
        assert_eq!(info.token_prefix, raw.chars().take(12).collect::<String>());

        let ctx = repo.authenticate(&raw).await.unwrap();
        // A share is the OWNER's own capability, not impersonation.
        assert_eq!(ctx.real_user.id, owner);
        assert_eq!(ctx.effective_user.id, owner);
        let scope = ctx.scope.expect("share token must carry a scope");
        assert_eq!(scope.session_id, Id::from("S1"));
        assert_eq!(scope.role, WorkspaceRole::Viewer);
    }

    /// Seed a ROOT user (the common primary-account shape) and return its id.
    async fn seed_root_user(pool: &SqlitePool, username: &str) -> Id {
        let id = new_id();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
             VALUES (?, ?, ?, ?, 1, ?)",
        )
        .bind(&id)
        .bind(username)
        .bind("hash")
        .bind("Root User")
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        id
    }

    /// SECURITY: a share minted by a **root** owner must NOT carry root. The
    /// principal's `id` is preserved (so the scoped session's `created_by == id`
    /// ownership check still passes) but `is_root` is forced to `false` on BOTH
    /// the real and effective user — a leaked share link can never carry root's
    /// blanket bypass. It still authenticates and still carries a `scope`.
    #[tokio::test]
    async fn root_owned_share_does_not_grant_root() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let root_owner = seed_root_user(&pool, "root").await;

        let (raw, _info) = repo
            .issue_share_token(
                &root_owner,
                &Id::from("S1"),
                WorkspaceRole::Viewer,
                3600,
                None,
            )
            .await
            .unwrap();

        let ctx = repo.authenticate(&raw).await.unwrap();
        // The owner's id is preserved (ownership checks still pass) …
        assert_eq!(ctx.real_user.id, root_owner);
        assert_eq!(ctx.effective_user.id, root_owner);
        // … but the root flag is dropped on BOTH identities.
        assert!(
            !ctx.real_user.is_root,
            "a share principal must never be root (real_user)"
        );
        assert!(
            !ctx.effective_user.is_root,
            "a share principal must never be root (effective_user)"
        );
        // And it remains a scoped capability pinned to its one session.
        let scope = ctx.scope.expect("share token must carry a scope");
        assert_eq!(scope.session_id, Id::from("S1"));
        assert_eq!(scope.role, WorkspaceRole::Viewer);
    }

    /// An editor share carries the Editor role in its scope (input allowed).
    #[tokio::test]
    async fn share_token_carries_editor_scope() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let owner = seed_user(&pool, "owner").await;

        let (raw, _info) = repo
            .issue_share_token(&owner, &Id::from("S7"), WorkspaceRole::Editor, 3600, None)
            .await
            .unwrap();
        let scope = repo.authenticate(&raw).await.unwrap().scope.unwrap();
        assert_eq!(scope.session_id, Id::from("S7"));
        assert_eq!(scope.role, WorkspaceRole::Editor);
    }

    /// A share can only be Viewer or Editor — minting an Admin share is rejected
    /// (a share must never escalate).
    #[tokio::test]
    async fn share_token_rejects_admin_role() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let owner = seed_user(&pool, "owner").await;

        assert!(
            repo.issue_share_token(&owner, &Id::from("S1"), WorkspaceRole::Admin, 3600, None)
                .await
                .is_err(),
            "an Admin-role share must be rejected"
        );
    }

    /// A share TTL is FIXED, never slid: authenticating does not advance the
    /// stored `expires_at` (the sliding/touch path is skipped for kind='share').
    #[tokio::test]
    async fn share_token_ttl_is_fixed_not_sliding() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let owner = seed_user(&pool, "owner").await;

        let (raw, info) = repo
            .issue_share_token(&owner, &Id::from("S1"), WorkspaceRole::Viewer, 3600, None)
            .await
            .unwrap();
        let before = stored_expires_at(&pool, &raw).await;
        // The touch is throttled to once/hour, but for a share the row must NOT
        // be slid even after the throttle window — assert the value is stable
        // across an authenticate and that it equals the minted expiry.
        let ctx = repo.authenticate(&raw).await.unwrap();
        assert!(ctx.scope.is_some());
        let after = stored_expires_at(&pool, &raw).await;
        assert_eq!(before, after, "share expiry must never be slid");
        assert_eq!(
            after,
            info.expires_at.to_rfc3339(),
            "stored expiry must equal the fixed minted expiry"
        );
    }

    /// The clamp pins ttl to a sane ceiling: a request well above the max is
    /// capped (expiry is `created_at + MAX`, not the requested value).
    #[tokio::test]
    async fn share_token_ttl_is_clamped() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let owner = seed_user(&pool, "owner").await;

        // Ask for ~10 days; expect the 24h ceiling.
        let (_raw, info) = repo
            .issue_share_token(
                &owner,
                &Id::from("S1"),
                WorkspaceRole::Viewer,
                864_000,
                None,
            )
            .await
            .unwrap();
        let ttl = (info.expires_at - info.created_at).num_seconds();
        assert_eq!(ttl, SHARE_TOKEN_TTL_MAX_SECS, "ttl must clamp to the max");
    }

    /// A revoked share no longer authenticates (`revoked=1` is a hard reject).
    #[tokio::test]
    async fn revoked_share_token_fails_auth() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let owner = seed_user(&pool, "owner").await;

        let (raw, info) = repo
            .issue_share_token(&owner, &Id::from("S1"), WorkspaceRole::Viewer, 3600, None)
            .await
            .unwrap();
        assert!(repo.authenticate(&raw).await.is_ok());

        repo.revoke_share(&owner, &info.id).await.unwrap();
        assert!(
            matches!(repo.authenticate(&raw).await, Err(Error::Unauthorized)),
            "a revoked share must not authenticate"
        );
        // And it drops out of the session's live share listing.
        assert!(repo
            .list_shares_for_session(&Id::from("S1"))
            .await
            .unwrap()
            .is_empty());
    }

    /// `revoke_share` is owner-scoped: another user cannot revoke it.
    #[tokio::test]
    async fn revoke_share_is_owner_scoped() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let owner = seed_user(&pool, "owner").await;
        let other = seed_user(&pool, "other").await;

        let (raw, info) = repo
            .issue_share_token(&owner, &Id::from("S1"), WorkspaceRole::Viewer, 3600, None)
            .await
            .unwrap();
        // `other` cannot revoke `owner`'s share.
        repo.revoke_share(&other, &info.id).await.unwrap();
        assert!(repo.authenticate(&raw).await.is_ok(), "still valid");
    }

    /// An expired share is rejected. (The mint clamps ttl up to a 60s floor, so
    /// expiry is forced by backdating the row's `expires_at` into the past — the
    /// same `expires_at <= now` check that gates every token kind.)
    #[tokio::test]
    async fn expired_share_token_fails_auth() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let owner = seed_user(&pool, "owner").await;

        let (raw, _info) = repo
            .issue_share_token(&owner, &Id::from("S1"), WorkspaceRole::Viewer, 3600, None)
            .await
            .unwrap();
        // Force expiry: set the stored expiry one hour into the past.
        let past = (Utc::now() - Duration::hours(1)).to_rfc3339();
        sqlx::query("UPDATE auth_sessions SET expires_at = ? WHERE token_hash = ?")
            .bind(&past)
            .bind(token_hash(&raw))
            .execute(&pool)
            .await
            .unwrap();

        assert!(
            matches!(repo.authenticate(&raw).await, Err(Error::Unauthorized)),
            "an expired share must not authenticate"
        );
        // Expired shares are excluded from the session listing.
        assert!(repo
            .list_shares_for_session(&Id::from("S1"))
            .await
            .unwrap()
            .is_empty());
    }

    /// `list_shares_for_session` returns only live (non-revoked, non-expired)
    /// shares for the given session, newest first, with no secret.
    #[tokio::test]
    async fn list_shares_for_session_lists_live_only() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let owner = seed_user(&pool, "owner").await;

        let (_r1, live) = repo
            .issue_share_token(
                &owner,
                &Id::from("S1"),
                WorkspaceRole::Viewer,
                3600,
                Some("a".into()),
            )
            .await
            .unwrap();
        let (_r2, revoked) = repo
            .issue_share_token(&owner, &Id::from("S1"), WorkspaceRole::Editor, 3600, None)
            .await
            .unwrap();
        // A share for a DIFFERENT session must not appear.
        let (_r3, _other) = repo
            .issue_share_token(&owner, &Id::from("S2"), WorkspaceRole::Viewer, 3600, None)
            .await
            .unwrap();
        repo.revoke_share(&owner, &revoked.id).await.unwrap();

        let listed = repo.list_shares_for_session(&Id::from("S1")).await.unwrap();
        assert_eq!(listed.len(), 1, "only the live S1 share is listed");
        assert_eq!(listed[0].id, live.id);
        assert_eq!(listed[0].session_id, Id::from("S1"));
        assert_eq!(listed[0].label.as_deref(), Some("a"));
    }

    // ---- email-OTP share tokens (mobile plan Tasks 7.2/7.3) ---------------

    /// `generate_otp` always yields a 6-digit numeric string (zero-padded).
    #[test]
    fn generate_otp_is_six_digits() {
        for _ in 0..200 {
            let otp = generate_otp();
            assert_eq!(otp.len(), 6, "OTP must be 6 chars: {otp}");
            assert!(
                otp.chars().all(|c| c.is_ascii_digit()),
                "OTP must be numeric: {otp}"
            );
        }
    }

    /// Minting an OTP share: returns a raw OTP, stores only its hash, and the
    /// share authenticates as **OTP-pending** (reaches nothing until verified).
    #[tokio::test]
    async fn otp_share_is_pending_until_verified() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let owner = seed_user(&pool, "owner").await;

        let (raw, otp, info) = repo
            .issue_share_otp_token(
                &owner,
                &Id::from("S1"),
                WorkspaceRole::Viewer,
                3600,
                None,
                "guest@example.com",
            )
            .await
            .unwrap();
        assert_eq!(otp.len(), 6);
        assert_eq!(info.session_id, Id::from("S1"));

        // The raw OTP is never stored — only its hash is in otp_hash.
        let stored: String = sqlx::query("SELECT otp_hash FROM auth_sessions WHERE token_hash = ?")
            .bind(token_hash(&raw))
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("otp_hash");
        assert!(
            stored.starts_with("$argon2id$"),
            "otp_hash must be an argon2id PHC string, got {stored}"
        );
        assert!(
            crate::passwords::verify_password(&otp, &stored).unwrap(),
            "otp_hash must verify the raw OTP"
        );
        assert_ne!(stored, otp, "raw OTP must not be stored");

        // It authenticates but is OTP-pending (gated).
        let ctx = repo.authenticate(&raw).await.unwrap();
        let scope = ctx.scope.expect("scoped");
        assert!(scope.otp_pending, "an unverified OTP share must be pending");
    }

    /// Verify with the correct code clears the pending flag (single-use): the
    /// same code cannot be redeemed twice.
    #[tokio::test]
    async fn otp_verify_succeeds_once_then_is_single_use() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let owner = seed_user(&pool, "owner").await;

        let (raw, otp, _info) = repo
            .issue_share_otp_token(
                &owner,
                &Id::from("S1"),
                WorkspaceRole::Editor,
                3600,
                None,
                "guest@example.com",
            )
            .await
            .unwrap();

        // Wrong code → false (and stays pending).
        assert!(!repo.verify_share_otp(&raw, "000000").await.unwrap() || otp == "000000");
        // Correct code → true, and the share becomes non-pending.
        assert!(repo.verify_share_otp(&raw, &otp).await.unwrap());
        let scope = repo.authenticate(&raw).await.unwrap().scope.unwrap();
        assert!(
            !scope.otp_pending,
            "after verify the share is no longer pending"
        );
        assert_eq!(scope.role, WorkspaceRole::Editor);

        // Single-use: the same code cannot be redeemed again.
        assert!(
            !repo.verify_share_otp(&raw, &otp).await.unwrap(),
            "the OTP must be single-use"
        );
    }

    /// An expired OTP cannot be redeemed (otp_expires_at in the past).
    #[tokio::test]
    async fn expired_otp_cannot_be_verified() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let owner = seed_user(&pool, "owner").await;

        let (raw, otp, _info) = repo
            .issue_share_otp_token(
                &owner,
                &Id::from("S1"),
                WorkspaceRole::Viewer,
                3600,
                None,
                "guest@example.com",
            )
            .await
            .unwrap();

        // Backdate the code's expiry into the past.
        let past = (Utc::now() - Duration::minutes(1)).timestamp();
        sqlx::query("UPDATE auth_sessions SET otp_expires_at = ? WHERE token_hash = ?")
            .bind(past)
            .bind(token_hash(&raw))
            .execute(&pool)
            .await
            .unwrap();

        assert!(
            !repo.verify_share_otp(&raw, &otp).await.unwrap(),
            "an expired OTP must be rejected"
        );
        // And the share stays pending.
        assert!(
            repo.authenticate(&raw)
                .await
                .unwrap()
                .scope
                .unwrap()
                .otp_pending
        );
    }

    /// Once verified, the share re-pends after its `max_expires_at` window ends.
    #[tokio::test]
    async fn verified_otp_share_repends_after_window() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let owner = seed_user(&pool, "owner").await;

        let (raw, otp, _info) = repo
            .issue_share_otp_token(
                &owner,
                &Id::from("S1"),
                WorkspaceRole::Viewer,
                3600,
                None,
                "guest@example.com",
            )
            .await
            .unwrap();
        assert!(repo.verify_share_otp(&raw, &otp).await.unwrap());
        assert!(
            !repo
                .authenticate(&raw)
                .await
                .unwrap()
                .scope
                .unwrap()
                .otp_pending
        );

        // Move the session window (and the bearer expiry, to keep the token
        // alive for the test) — set max_expires_at to the past but keep
        // expires_at in the future so authenticate still resolves the row.
        let past = (Utc::now() - Duration::minutes(1)).timestamp();
        sqlx::query("UPDATE auth_sessions SET max_expires_at = ? WHERE token_hash = ?")
            .bind(past)
            .bind(token_hash(&raw))
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            repo.authenticate(&raw)
                .await
                .unwrap()
                .scope
                .unwrap()
                .otp_pending,
            "after the window elapses the share must re-pend"
        );
    }

    /// The OTP-share window clamps to ≤12h.
    #[tokio::test]
    async fn otp_share_window_clamps_to_12h() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let owner = seed_user(&pool, "owner").await;

        let (_raw, _otp, info) = repo
            .issue_share_otp_token(
                &owner,
                &Id::from("S1"),
                WorkspaceRole::Viewer,
                100 * 60 * 60, // ask 100h
                None,
                "guest@example.com",
            )
            .await
            .unwrap();
        let window = (info.expires_at - info.created_at).num_seconds();
        assert_eq!(
            window, SHARE_OTP_WINDOW_MAX_SECS,
            "window must clamp to 12h"
        );
    }

    /// A plain share (no recipient email) is NEVER OTP-pending (backward compat).
    #[tokio::test]
    async fn plain_share_is_never_otp_pending() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let owner = seed_user(&pool, "owner").await;

        let (raw, _info) = repo
            .issue_share_token(&owner, &Id::from("S1"), WorkspaceRole::Viewer, 3600, None)
            .await
            .unwrap();
        let scope = repo.authenticate(&raw).await.unwrap().scope.unwrap();
        assert!(!scope.otp_pending, "a plain share must not be OTP-pending");
        // And verify is a no-op on a non-OTP share.
        assert!(!repo.verify_share_otp(&raw, "123456").await.unwrap());
    }

    /// `revoke_all_for_user` also clears the user's shares (lost-device kill
    /// switch): every kind — session, API, AND share — stops authenticating.
    #[tokio::test]
    async fn revoke_all_for_user_clears_shares_too() {
        let pool = mem_pool().await;
        let repo = AuthRepo::new(pool.clone());
        let owner = seed_user(&pool, "owner").await;

        let session = repo.issue(&owner).await.unwrap();
        let (api, _) = repo.issue_api_token(&owner, None).await.unwrap();
        let (share, _) = repo
            .issue_share_token(&owner, &Id::from("S1"), WorkspaceRole::Viewer, 3600, None)
            .await
            .unwrap();
        for t in [&session, &api, &share] {
            assert!(repo.authenticate(t).await.is_ok());
        }

        repo.revoke_all_for_user(&owner).await.unwrap();

        for t in [&session, &api, &share] {
            assert!(
                matches!(repo.authenticate(t).await, Err(Error::Unauthorized)),
                "revoke_all_for_user must invalidate the share too"
            );
        }
    }

    // ---- auth-lookup cache correctness tests --------------------------------
    //
    // These tests construct `AuthRepo::with_cache` and prove the three required
    // invariants:
    //
    //   (a) A revoked login/api token is rejected IMMEDIATELY after revocation —
    //       the cache is evicted, not served stale.
    //   (b) share/impersonation tokens are NEVER cached — revoke takes effect
    //       the moment the DB row is gone, with zero cache window.
    //   (c) Repeated requests within the TTL avoid an unnecessary DB round-trip
    //       (the cache is actually populated on first auth and hit on subsequent
    //       requests within the TTL).

    use crate::cache::AuthCache;

    /// Helper: cached repo + shared cache for inspection.
    fn cached_repo(pool: SqlitePool) -> (AuthRepo, AuthCache) {
        let cache = AuthCache::new();
        let repo = AuthRepo::with_cache(pool, cache.clone());
        (repo, cache)
    }

    /// (a) A revoked **login** token is rejected immediately: `revoke` evicts the
    /// hash from the cache before returning, so a second `authenticate` does not
    /// see the now-invalid stale entry.
    #[tokio::test]
    async fn cached_revoked_login_token_rejected_immediately() {
        let pool = mem_pool().await;
        let (repo, _cache) = cached_repo(pool.clone());
        let uid = seed_user(&pool, "alice_cache").await;

        let token = repo.issue(&uid).await.unwrap();

        // First authenticate: DB hit → cache populated.
        assert!(
            repo.authenticate(&token).await.is_ok(),
            "fresh login token must authenticate"
        );

        // Revoke: DB delete + cache eviction.
        repo.revoke(&token).await.unwrap();

        // Second authenticate: the cache entry was evicted; DB row is gone.
        // Must fail — NOT be served from stale cache.
        assert!(
            matches!(repo.authenticate(&token).await, Err(Error::Unauthorized)),
            "revoked login token must be rejected even with cache present"
        );
    }

    /// (a) A revoked **API** token is rejected immediately after `revoke_api_token`.
    #[tokio::test]
    async fn cached_revoked_api_token_rejected_immediately() {
        let pool = mem_pool().await;
        let (repo, _cache) = cached_repo(pool.clone());
        let uid = seed_user(&pool, "bob_cache").await;

        let (token, info) = repo.issue_api_token(&uid, Some("ci")).await.unwrap();

        // Prime the cache.
        assert!(repo.authenticate(&token).await.is_ok());

        // Revoke by id (the normal PAT revocation path).
        assert!(repo.revoke_api_token(&uid, &info.id).await.unwrap());

        // Must not be served from cache.
        assert!(
            matches!(repo.authenticate(&token).await, Err(Error::Unauthorized)),
            "revoked API token must be rejected even with cache present"
        );
    }

    /// (a) `revoke_all_for_user` evicts ALL cached entries for that user;
    /// a bystander's token is unaffected.
    #[tokio::test]
    async fn cached_revoke_all_for_user_evicts_user_tokens() {
        let pool = mem_pool().await;
        let (repo, _cache) = cached_repo(pool.clone());
        let victim = seed_user(&pool, "victim_cache").await;
        let bystander = seed_user(&pool, "bystander_cache").await;

        let sess = repo.issue(&victim).await.unwrap();
        let (api, _) = repo.issue_api_token(&victim, None).await.unwrap();
        let bystander_tok = repo.issue(&bystander).await.unwrap();

        // Prime all three into the cache.
        for t in [&sess, &api, &bystander_tok] {
            assert!(repo.authenticate(t).await.is_ok());
        }

        // Revoke all for victim (DB + cache eviction).
        repo.revoke_all_for_user(&victim).await.unwrap();

        // Victim's tokens must now fail.
        for t in [&sess, &api] {
            assert!(
                matches!(repo.authenticate(t).await, Err(Error::Unauthorized)),
                "victim's token must be rejected after revoke_all_for_user"
            );
        }
        // Bystander is unaffected.
        assert!(
            repo.authenticate(&bystander_tok).await.is_ok(),
            "bystander token must still work after victim's revoke_all"
        );
    }

    /// (b) **share** tokens are NEVER inserted into the cache. A share revoke
    /// (`revoke_share`) takes effect instantly even with a cache-bearing repo.
    #[tokio::test]
    async fn share_token_is_never_cached_revoke_is_instant() {
        let pool = mem_pool().await;
        let (repo, cache) = cached_repo(pool.clone());
        let owner = seed_user(&pool, "owner_cache").await;

        let (raw, info) = repo
            .issue_share_token(&owner, &Id::from("S1"), WorkspaceRole::Viewer, 3600, None)
            .await
            .unwrap();

        // Authenticate the share to confirm it works.
        assert!(repo.authenticate(&raw).await.is_ok());

        // The cache must NOT contain the share token's hash.
        let h = token_hash(&raw);
        assert!(
            cache.get(&h).is_none(),
            "share token must never be present in the auth cache"
        );

        // Revoke the share (flips revoked=1 in DB).
        repo.revoke_share(&owner, &info.id).await.unwrap();

        // Must be reflected immediately (no cache window for share tokens).
        assert!(
            matches!(repo.authenticate(&raw).await, Err(Error::Unauthorized)),
            "revoked share token must be rejected immediately (no cache window)"
        );
    }

    /// (b) **impersonation** tokens are NEVER inserted into the cache. Revoking
    /// (via `revoke`) takes effect instantly.
    #[tokio::test]
    async fn impersonation_token_is_never_cached_revoke_is_instant() {
        let pool = mem_pool().await;
        let (repo, cache) = cached_repo(pool.clone());
        let admin = seed_user(&pool, "admin_cache").await;
        let target = seed_user(&pool, "target_cache").await;

        let token = repo
            .issue_impersonation_token(&admin, &target, Duration::minutes(30))
            .await
            .unwrap();

        // Authenticate once — must succeed.
        assert!(repo.authenticate(&token).await.is_ok());

        // The cache must NOT contain the impersonation token's hash.
        let h = token_hash(&token);
        assert!(
            cache.get(&h).is_none(),
            "impersonation token must never be present in the auth cache"
        );

        // Revoke — must take immediate effect (no cache buffering).
        repo.revoke(&token).await.unwrap();
        assert!(
            matches!(repo.authenticate(&token).await, Err(Error::Unauthorized)),
            "revoked impersonation token must be rejected immediately"
        );
    }

    /// (c) Repeated requests for the same valid login token within the TTL are
    /// served from cache — the cache entry is populated on the first DB hit and
    /// present for subsequent requests within the TTL.
    #[tokio::test]
    async fn cached_login_token_served_from_cache_on_repeat() {
        let pool = mem_pool().await;
        let (repo, cache) = cached_repo(pool.clone());
        let uid = seed_user(&pool, "alice_repeat").await;

        let token = repo.issue(&uid).await.unwrap();
        let h = token_hash(&token);

        // Before the first authenticate: cache is empty for this hash.
        assert!(
            cache.get(&h).is_none(),
            "cache must be empty before first authenticate"
        );

        // First request → DB hit → inserts into cache.
        let ctx1 = repo.authenticate(&token).await.unwrap();
        assert_eq!(ctx1.effective_user.id, uid);

        // Cache is now populated.
        assert!(
            cache.get(&h).is_some(),
            "cache must be populated after first successful authenticate"
        );

        // Second request → cache hit (result must be consistent).
        let ctx2 = repo.authenticate(&token).await.unwrap();
        assert_eq!(
            ctx2.effective_user.id, uid,
            "cached result must match the original DB result"
        );
    }
}
