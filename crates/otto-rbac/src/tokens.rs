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
use otto_core::api::{ApiTokenInfo, ShareInfo};
use otto_core::auth::{AuthContext, SessionScope};
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
                    a.recipient_email, a.verified_at, a.max_expires_at,
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

        let last_seen = parse_ts(&row.get::<String, _>("last_seen_at"))?;
        // Share tokens have a SHORT, FIXED TTL: never touch/slide them. Skipping
        // the touch entirely (not just the slide) keeps their expiry verifiably
        // immutable — `authenticate` is a pure read for kind='share'.
        if !is_share && now - last_seen > Duration::seconds(TOUCH_THROTTLE_SECS) {
            // Always record "last used"; only SLIDE the expiry for interactive
            // session tokens. API tokens keep their long fixed lifetime and
            // impersonation tokens have a SHORT FIXED TTL — neither is ever slid
            // (impersonation must time out predictably).
            let slide = !is_impersonation && kind != "api";
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
            });
        }

        // Normal token (kind='session'/'api'): the real (token owner) and
        // effective (acted-as) user are the same, and it reaches the whole
        // authorized surface (no scope). These are the ONLY kinds we cache.
        let ctx = AuthContext {
            real_user: real_user.clone(),
            effective_user: real_user,
            scope: None,
        };

        // Populate the cache so the next request for this token avoids the DB.
        // We use the `user_id` column read from the join above (the `id` field
        // of `real_user`, which is always the token-owner row's `user_id`).
        if let Some(cache) = &self.cache {
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

        let res = sqlx::query(
            "DELETE FROM auth_sessions WHERE id = ? AND user_id = ? AND kind = 'api'",
        )
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
        .bind(token_hash(&otp))
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
        if token_hash(otp) != stored_hash {
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
    pub async fn extend_share_otp(
        &self,
        token: &str,
    ) -> Result<Option<(String, String, Id)>> {
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
        let window_secs = original_window.clamp(SHARE_TOKEN_TTL_MIN_SECS, SHARE_OTP_WINDOW_MAX_SECS);

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
        .bind(token_hash(&otp))
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
    pub async fn share_session_id(
        &self,
        owner_user_id: &Id,
        share_id: &str,
    ) -> Result<Option<Id>> {
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
        assert_eq!(info.token_prefix, token.chars().take(12).collect::<String>());

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
            .issue_share_token(&root_owner, &Id::from("S1"), WorkspaceRole::Viewer, 3600, None)
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
            .issue_share_token(&owner, &Id::from("S1"), WorkspaceRole::Viewer, 864_000, None)
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
            .issue_share_token(&owner, &Id::from("S1"), WorkspaceRole::Viewer, 3600, Some("a".into()))
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
            assert!(otp.chars().all(|c| c.is_ascii_digit()), "OTP must be numeric: {otp}");
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
        let stored: String =
            sqlx::query("SELECT otp_hash FROM auth_sessions WHERE token_hash = ?")
                .bind(token_hash(&raw))
                .fetch_one(&pool)
                .await
                .unwrap()
                .get("otp_hash");
        assert_eq!(stored, token_hash(&otp), "otp_hash must be sha256(otp)");
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
        assert!(!scope.otp_pending, "after verify the share is no longer pending");
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
        assert!(repo.authenticate(&raw).await.unwrap().scope.unwrap().otp_pending);
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
        assert!(!repo.authenticate(&raw).await.unwrap().scope.unwrap().otp_pending);

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
            repo.authenticate(&raw).await.unwrap().scope.unwrap().otp_pending,
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
        assert_eq!(window, SHARE_OTP_WINDOW_MAX_SECS, "window must clamp to 12h");
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
