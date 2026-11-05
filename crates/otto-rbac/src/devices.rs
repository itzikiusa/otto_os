//! Phone pairing + device tokens (`phone_pairings`, `phone_devices`).
//!
//! The flow (docs/features/phone-access.md):
//!
//! 1. A person on the Mac creates a pairing → a single-use **code** (32 random
//!    bytes, hex) that the QR carries. Only its SHA-256 is stored; it expires
//!    after [`PAIRING_CODE_TTL_SECS`] and a user has at most one outstanding
//!    pairing (a new one expires the previous).
//! 2. The phone **claims** the code (public route, tailnet peers only): the row
//!    moves `waiting → claimed` atomically (so a code works once), records the
//!    device label / peer address and mints a 6-digit **confirmation code**
//!    both screens show, plus a **poll secret** (hashed) the phone polls with.
//! 3. The person on the Mac **approves** (or denies) within
//!    [`APPROVAL_WINDOW_SECS`].
//! 4. The phone's next **poll** atomically moves `approved → consumed` and only
//!    then mints the device token, so the raw token exists exactly once, in
//!    that one response, and is never stored.
//!
//! A device token authenticates as its owner (`AuthContext::device_id` set),
//! with a sliding idle expiry ([`DEVICE_IDLE_DAYS`]) capped by an absolute one
//! ([`DEVICE_ABSOLUTE_DAYS`]). It is never cached (a revoke is immediate) and
//! is refused from any peer that is neither the tailnet nor this Mac.

use chrono::{DateTime, Duration, Utc};
use otto_core::phone::{PairingStatus, PairingView, PhoneDevice};
use otto_core::{new_id, Error, Id, Result};
use otto_state::DbPool;
use rand::rand_core::UnwrapErr;
use rand::rngs::SysRng;
use rand::Rng;
use sqlx::Row;

use crate::tokens::{generate_otp, signal_revocation, token_hash};

/// How long a QR code can be opened after it is shown.
pub const PAIRING_CODE_TTL_SECS: i64 = 120;
/// How long the person on the Mac has to approve once a phone claimed it, and
/// how long the phone then has to collect its token.
pub const APPROVAL_WINDOW_SECS: i64 = 120;
/// Sliding idle expiry of a device token.
pub const DEVICE_IDLE_DAYS: i64 = 30;
/// Absolute lifetime of a device token.
pub const DEVICE_ABSOLUTE_DAYS: i64 = 180;
/// Minimum gap between two `last_seen_at` / `last_ip` writes.
const DEVICE_TOUCH_SECS: i64 = 300;
/// Longest stored device name.
pub const DEVICE_NAME_MAX: usize = 64;

/// Outcome of a phone's poll.
#[derive(Debug, Clone)]
pub enum PollOutcome {
    Pending,
    Denied,
    Expired,
    /// The device token (raw, shown once), the validated landing route and the
    /// new device's row.
    Approved {
        token: String,
        next: Option<String>,
        device: Box<PhoneDevice>,
    },
}

/// What a successful claim hands the phone.
#[derive(Debug, Clone)]
pub struct Claimed {
    pub pairing_id: Id,
    pub poll_secret: String,
    pub confirm_code: String,
    pub expires_at: String,
}

/// A device token resolved by [`DevicesRepo::authenticate`].
#[derive(Debug, Clone)]
pub struct DeviceAuth {
    pub device_id: Id,
    pub user_id: Id,
}

fn random_hex32() -> String {
    let mut buf = [0u8; 32];
    UnwrapErr(SysRng).fill_bytes(&mut buf);
    hex::encode(buf)
}

fn parse_ts(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| Error::Internal(format!("bad timestamp '{s}': {e}")))
}

fn db(ctx: &'static str) -> impl Fn(sqlx::Error) -> Error {
    move |e| Error::Internal(format!("{ctx}: {e}"))
}

/// Trim and bound a device name (control characters dropped).
pub fn clean_device_name(raw: &str) -> Option<String> {
    let s: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .take(DEVICE_NAME_MAX)
        .collect();
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

macro_rules! device_cols {
    () => {
        "id, name, user_agent, created_at, last_seen_at, last_ip, idle_expires_at, expires_at"
    };
}

fn device_from_row(r: &sqlx::sqlite::SqliteRow, current: Option<&str>) -> PhoneDevice {
    let id: String = r.get("id");
    PhoneDevice {
        current: current == Some(id.as_str()),
        id: Id::from(id),
        name: r.get("name"),
        user_agent: r.get("user_agent"),
        created_at: r.get("created_at"),
        last_seen_at: r.get("last_seen_at"),
        last_ip: r.get("last_ip"),
        idle_expires_at: r.get("idle_expires_at"),
        expires_at: r.get("expires_at"),
    }
}

/// Repository over the phone-access tables.
#[derive(Clone)]
pub struct DevicesRepo {
    pool: DbPool,
}

impl DevicesRepo {
    pub fn new(pool: impl Into<DbPool>) -> Self {
        Self { pool: pool.into() }
    }

    // ---- pairings ---------------------------------------------------------

    /// Start a pairing for `user_id`; returns `(id, raw code, expires_at)`.
    /// Any outstanding pairing of the user (waiting / claimed / approved) is
    /// expired first — one at a time.
    pub async fn create_pairing(
        &self,
        user_id: &Id,
        next: Option<&str>,
    ) -> Result<(Id, String, String)> {
        let now = Utc::now();
        sqlx::query(
            "UPDATE phone_pairings SET status = 'expired' \
             WHERE user_id = ? AND status IN ('waiting','claimed','approved')",
        )
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(db("expire pairings"))?;
        let id = new_id();
        let code = random_hex32();
        let expires = (now + Duration::seconds(PAIRING_CODE_TTL_SECS)).to_rfc3339();
        sqlx::query(
            "INSERT INTO phone_pairings (id, user_id, code_hash, status, next_route, created_at, expires_at) \
             VALUES (?, ?, ?, 'waiting', ?, ?, ?)",
        )
        .bind(&id)
        .bind(user_id)
        .bind(token_hash(&code))
        .bind(next)
        .bind(now.to_rfc3339())
        .bind(&expires)
        .execute(&self.pool)
        .await
        .map_err(db("create pairing"))?;
        Ok((id, code, expires))
    }

    /// The phone opens the QR. Atomic `waiting → claimed`: a code is usable
    /// exactly once, and only before it expires. Any failure is the same
    /// `NotFound` (no oracle for which codes exist).
    pub async fn claim(
        &self,
        code: &str,
        device_label: &str,
        user_agent: Option<&str>,
        peer_ip: &str,
    ) -> Result<Claimed> {
        let now = Utc::now();
        let poll_secret = random_hex32();
        let confirm = generate_otp();
        let expires = (now + Duration::seconds(APPROVAL_WINDOW_SECS)).to_rfc3339();
        let id: Option<String> = sqlx::query_scalar(
            "UPDATE phone_pairings SET status = 'claimed', claimed_at = ?, device_label = ?, \
                 user_agent = ?, peer_ip = ?, confirm_code = ?, poll_secret_hash = ?, expires_at = ? \
             WHERE code_hash = ? AND status = 'waiting' AND expires_at > ? \
             RETURNING id",
        )
        .bind(now.to_rfc3339())
        .bind(device_label)
        .bind(user_agent)
        .bind(peer_ip)
        .bind(&confirm)
        .bind(token_hash(&poll_secret))
        .bind(&expires)
        .bind(token_hash(code))
        .bind(now.to_rfc3339())
        .fetch_optional(&self.pool)
        .await
        .map_err(db("claim pairing"))?;
        let id = id.ok_or_else(|| Error::NotFound("invalid or expired pairing code".into()))?;
        Ok(Claimed {
            pairing_id: Id::from(id),
            poll_secret,
            confirm_code: confirm,
            expires_at: expires,
        })
    }

    /// The Mac-side view of one of `user_id`'s pairings.
    pub async fn pairing(&self, user_id: &Id, id: &Id) -> Result<PairingView> {
        let r = sqlx::query(
            "SELECT id, status, expires_at, device_label, peer_ip, confirm_code \
             FROM phone_pairings WHERE id = ? AND user_id = ?",
        )
        .bind(id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db("get pairing"))?
        .ok_or_else(|| Error::NotFound("pairing not found".into()))?;
        let expires: String = r.get("expires_at");
        let mut status =
            PairingStatus::parse(&r.get::<String, _>("status")).unwrap_or(PairingStatus::Expired);
        let open = matches!(
            status,
            PairingStatus::Waiting | PairingStatus::Claimed | PairingStatus::Approved
        );
        if open && parse_ts(&expires)? <= Utc::now() {
            status = PairingStatus::Expired;
        }
        let claimed = !matches!(status, PairingStatus::Waiting);
        Ok(PairingView {
            id: Id::from(r.get::<String, _>("id")),
            status,
            expires_at: expires,
            device_label: r.get("device_label"),
            peer_ip: r.get("peer_ip"),
            confirm_code: if claimed { r.get("confirm_code") } else { None },
        })
    }

    /// Approve or deny a claimed pairing (person on the Mac). Only a
    /// `claimed`, unexpired row of `user_id` moves; anything else is a
    /// `Conflict` carrying the current state.
    pub async fn decide(&self, user_id: &Id, id: &Id, approve: bool) -> Result<PairingView> {
        let now = Utc::now();
        let res = sqlx::query(
            "UPDATE phone_pairings SET status = ?, decided_at = ?, expires_at = ? \
             WHERE id = ? AND user_id = ? AND status = 'claimed' AND expires_at > ?",
        )
        .bind(if approve { "approved" } else { "denied" })
        .bind(now.to_rfc3339())
        .bind((now + Duration::seconds(APPROVAL_WINDOW_SECS)).to_rfc3339())
        .bind(id)
        .bind(user_id)
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(db("decide pairing"))?;
        let view = self.pairing(user_id, id).await?;
        if res.rows_affected() == 0 {
            return Err(Error::Conflict(format!(
                "this pairing is {} — it can no longer be decided",
                view.status.as_str()
            )));
        }
        Ok(view)
    }

    /// The phone polls with its secret. On approval the row is consumed and
    /// the device token minted in one transaction.
    pub async fn poll(&self, pairing_id: &Id, poll_secret: &str) -> Result<PollOutcome> {
        let r = sqlx::query(
            "SELECT user_id, status, expires_at, next_route, device_label, user_agent, peer_ip \
             FROM phone_pairings WHERE id = ? AND poll_secret_hash = ?",
        )
        .bind(pairing_id)
        .bind(token_hash(poll_secret))
        .fetch_optional(&self.pool)
        .await
        .map_err(db("poll pairing"))?
        .ok_or_else(|| Error::NotFound("pairing not found".into()))?;
        let status =
            PairingStatus::parse(&r.get::<String, _>("status")).unwrap_or(PairingStatus::Expired);
        let expired = parse_ts(&r.get::<String, _>("expires_at"))? <= Utc::now();
        match status {
            PairingStatus::Denied => return Ok(PollOutcome::Denied),
            PairingStatus::Claimed if !expired => return Ok(PollOutcome::Pending),
            PairingStatus::Approved if !expired => {}
            _ => return Ok(PollOutcome::Expired),
        }
        let user_id: String = r.get("user_id");
        let label: Option<String> = r.get("device_label");
        let ua: Option<String> = r.get("user_agent");
        let ip: Option<String> = r.get("peer_ip");
        let next: Option<String> = r.get("next_route");

        let mut tx = self.pool.begin().await.map_err(db("poll tx"))?;
        let won = sqlx::query(
            "UPDATE phone_pairings SET status = 'consumed' WHERE id = ? AND status = 'approved'",
        )
        .bind(pairing_id)
        .execute(&mut *tx)
        .await
        .map_err(db("consume pairing"))?;
        if won.rows_affected() == 0 {
            return Ok(PollOutcome::Expired);
        }
        let now = Utc::now();
        let device_id = new_id();
        let token = random_hex32();
        let expires = now + Duration::days(DEVICE_ABSOLUTE_DAYS);
        let idle = (now + Duration::days(DEVICE_IDLE_DAYS)).min(expires);
        let name = label
            .as_deref()
            .and_then(clean_device_name)
            .unwrap_or_else(|| "Phone".into());
        sqlx::query(
            "INSERT INTO phone_devices (id, user_id, token_hash, token_prefix, name, user_agent, \
                 created_at, last_seen_at, last_ip, idle_expires_at, expires_at, pairing_id) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&device_id)
        .bind(&user_id)
        .bind(token_hash(&token))
        .bind(&token[..8])
        .bind(&name)
        .bind(&ua)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(&ip)
        .bind(idle.to_rfc3339())
        .bind(expires.to_rfc3339())
        .bind(pairing_id)
        .execute(&mut *tx)
        .await
        .map_err(db("insert device"))?;
        sqlx::query("UPDATE phone_pairings SET device_id = ? WHERE id = ?")
            .bind(&device_id)
            .bind(pairing_id)
            .execute(&mut *tx)
            .await
            .map_err(db("link device"))?;
        tx.commit().await.map_err(db("poll commit"))?;
        let device = self
            .device(&Id::from(user_id), &device_id, None)
            .await?
            .ok_or_else(|| Error::Internal("new device vanished".into()))?;
        Ok(PollOutcome::Approved {
            token,
            next: next.filter(|n| !n.is_empty()),
            device: Box::new(device),
        })
    }

    // ---- devices ------------------------------------------------------------

    /// Resolve a raw device token: unexpired (idle and absolute). Slides the
    /// idle expiry and records the peer, at most every few minutes. `None` =
    /// not a device token (or a dead one).
    pub async fn authenticate(&self, token: &str, ip: Option<&str>) -> Result<Option<DeviceAuth>> {
        let hash = token_hash(token);
        let Some(r) = sqlx::query(
            "SELECT id, user_id, last_seen_at, last_ip, idle_expires_at, expires_at \
             FROM phone_devices WHERE token_hash = ?",
        )
        .bind(&hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(db("device authenticate"))?
        else {
            return Ok(None);
        };
        let now = Utc::now();
        let idle = parse_ts(&r.get::<String, _>("idle_expires_at"))?;
        let absolute = parse_ts(&r.get::<String, _>("expires_at"))?;
        if idle <= now || absolute <= now {
            return Ok(None);
        }
        let id: String = r.get("id");
        let last_seen = parse_ts(&r.get::<String, _>("last_seen_at"))?;
        let last_ip: Option<String> = r.get("last_ip");
        let ip_changed = ip.is_some() && ip != last_ip.as_deref();
        if ip_changed || now - last_seen > Duration::seconds(DEVICE_TOUCH_SECS) {
            let idle_next = (now + Duration::days(DEVICE_IDLE_DAYS)).min(absolute);
            sqlx::query(
                "UPDATE phone_devices SET last_seen_at = ?, idle_expires_at = ?, \
                     last_ip = COALESCE(?, last_ip) WHERE id = ?",
            )
            .bind(now.to_rfc3339())
            .bind(idle_next.to_rfc3339())
            .bind(ip)
            .bind(&id)
            .execute(&self.pool)
            .await
            .map_err(db("device touch"))?;
        }
        Ok(Some(DeviceAuth {
            device_id: Id::from(id),
            user_id: Id::from(r.get::<String, _>("user_id")),
        }))
    }

    /// One of `user_id`'s devices.
    pub async fn device(
        &self,
        user_id: &Id,
        id: &str,
        current: Option<&str>,
    ) -> Result<Option<PhoneDevice>> {
        let row = sqlx::query(concat!(
            "SELECT ",
            device_cols!(),
            " FROM phone_devices WHERE id = ? AND user_id = ?"
        ))
        .bind(id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db("get device"))?;
        Ok(row.map(|r| device_from_row(&r, current)))
    }

    /// `user_id`'s live devices, newest first.
    pub async fn list(&self, user_id: &Id, current: Option<&str>) -> Result<Vec<PhoneDevice>> {
        let now = Utc::now().to_rfc3339();
        let rows = sqlx::query(concat!(
            "SELECT ",
            device_cols!(),
            " FROM phone_devices WHERE user_id = ? AND expires_at > ? AND idle_expires_at > ? \
             ORDER BY created_at DESC"
        ))
        .bind(user_id)
        .bind(&now)
        .bind(&now)
        .fetch_all(&self.pool)
        .await
        .map_err(db("list devices"))?;
        Ok(rows.iter().map(|r| device_from_row(r, current)).collect())
    }

    /// Rename one of `user_id`'s devices.
    pub async fn rename(&self, user_id: &Id, id: &str, name: &str) -> Result<PhoneDevice> {
        let name = clean_device_name(name).ok_or_else(|| Error::Invalid("name is empty".into()))?;
        sqlx::query("UPDATE phone_devices SET name = ? WHERE id = ? AND user_id = ?")
            .bind(&name)
            .bind(id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(db("rename device"))?;
        self.device(user_id, id, None)
            .await?
            .ok_or_else(|| Error::NotFound("device not found".into()))
    }

    /// Revoke one of `user_id`'s devices (deletes the row: the token is dead at
    /// once — device tokens are never cached). `true` iff a row went.
    pub async fn revoke(&self, user_id: &Id, id: &str) -> Result<bool> {
        let res = sqlx::query("DELETE FROM phone_devices WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(db("revoke device"))?;
        if res.rows_affected() > 0 {
            signal_revocation();
        }
        Ok(res.rows_affected() > 0)
    }

    /// Revoke every device of `user_id` (and cancel outstanding pairings).
    pub async fn revoke_all(&self, user_id: &Id) -> Result<u64> {
        sqlx::query(
            "UPDATE phone_pairings SET status = 'expired' \
             WHERE user_id = ? AND status IN ('waiting','claimed','approved')",
        )
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(db("expire pairings"))?;
        let res = sqlx::query("DELETE FROM phone_devices WHERE user_id = ?")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(db("revoke all devices"))?;
        signal_revocation();
        Ok(res.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::AuthRepo;
    use otto_core::peer::{PeerKind, RequestPeer};
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    async fn mem_pool() -> DbPool {
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
        pool.into()
    }

    async fn seed_user(pool: &DbPool, username: &str) -> Id {
        let id = new_id();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
             VALUES (?, ?, 'hash', 'T', 1, ?)",
        )
        .bind(&id)
        .bind(username)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await
        .unwrap();
        id
    }

    /// Pair end to end; returns the raw device token and the device id.
    async fn pair(repo: &DevicesRepo, user: &Id) -> (String, Id) {
        let (id, code, _) = repo.create_pairing(user, Some("threads/T1")).await.unwrap();
        let c = repo
            .claim(&code, "iPhone · Safari", Some("UA"), "100.1.2.3")
            .await
            .unwrap();
        assert_eq!(c.pairing_id, id);
        repo.decide(user, &id, true).await.unwrap();
        match repo.poll(&id, &c.poll_secret).await.unwrap() {
            PollOutcome::Approved {
                token,
                next,
                device,
            } => {
                assert_eq!(next.as_deref(), Some("threads/T1"));
                (token, device.id)
            }
            other => panic!("not approved: {other:?}"),
        }
    }

    #[tokio::test]
    async fn code_is_single_use_hashed_and_token_shown_once() {
        let pool = mem_pool().await;
        let repo = DevicesRepo::new(pool.clone());
        let user = seed_user(&pool, "u").await;
        let (id, code, _) = repo.create_pairing(&user, None).await.unwrap();
        // Hashed at rest: the raw code is nowhere in the row.
        let stored: String =
            sqlx::query_scalar("SELECT code_hash FROM phone_pairings WHERE id = ?")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_ne!(stored, code);
        assert_eq!(stored, token_hash(&code));
        let c = repo.claim(&code, "Phone", None, "100.1.2.3").await.unwrap();
        // Second claim of the same code fails.
        assert!(matches!(
            repo.claim(&code, "Phone", None, "100.9.9.9").await,
            Err(Error::NotFound(_))
        ));
        // Poll before approval: pending; wrong secret: not found.
        assert!(matches!(
            repo.poll(&id, &c.poll_secret).await.unwrap(),
            PollOutcome::Pending
        ));
        assert!(repo.poll(&id, "nope").await.is_err());
        repo.decide(&user, &id, true).await.unwrap();
        let PollOutcome::Approved { token, .. } = repo.poll(&id, &c.poll_secret).await.unwrap()
        else {
            panic!("expected approved");
        };
        // The token is minted once: a replayed poll gets nothing.
        assert!(matches!(
            repo.poll(&id, &c.poll_secret).await.unwrap(),
            PollOutcome::Expired
        ));
        let hash: String = sqlx::query_scalar("SELECT token_hash FROM phone_devices")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(hash, token_hash(&token));
    }

    #[tokio::test]
    async fn expired_code_and_one_outstanding_per_user() {
        let pool = mem_pool().await;
        let repo = DevicesRepo::new(pool.clone());
        let user = seed_user(&pool, "u").await;
        let (_, first, _) = repo.create_pairing(&user, None).await.unwrap();
        let (_, second, _) = repo.create_pairing(&user, None).await.unwrap();
        // The first code was expired by the second pairing.
        assert!(repo.claim(&first, "P", None, "100.1.1.1").await.is_err());
        // Age the second past its TTL.
        sqlx::query("UPDATE phone_pairings SET expires_at = ?")
            .bind((Utc::now() - Duration::seconds(1)).to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();
        assert!(repo.claim(&second, "P", None, "100.1.1.1").await.is_err());
    }

    #[tokio::test]
    async fn denied_and_late_decisions() {
        let pool = mem_pool().await;
        let repo = DevicesRepo::new(pool.clone());
        let user = seed_user(&pool, "u").await;
        let other = seed_user(&pool, "o").await;
        let (id, code, _) = repo.create_pairing(&user, None).await.unwrap();
        // Unclaimed: cannot be approved.
        assert!(matches!(
            repo.decide(&user, &id, true).await,
            Err(Error::Conflict(_))
        ));
        let c = repo.claim(&code, "P", None, "100.1.1.1").await.unwrap();
        // Another user cannot decide it.
        assert!(matches!(
            repo.decide(&other, &id, true).await,
            Err(Error::NotFound(_))
        ));
        let v = repo.decide(&user, &id, false).await.unwrap();
        assert_eq!(v.status, PairingStatus::Denied);
        assert!(matches!(
            repo.poll(&id, &c.poll_secret).await.unwrap(),
            PollOutcome::Denied
        ));
        assert!(matches!(
            repo.decide(&user, &id, true).await,
            Err(Error::Conflict(_))
        ));
    }

    #[tokio::test]
    async fn device_token_authenticates_until_idle_expiry_or_revoke() {
        let pool = mem_pool().await;
        let repo = DevicesRepo::new(pool.clone());
        let auth = AuthRepo::new(pool.clone());
        let user = seed_user(&pool, "u").await;
        let (token, device_id) = pair(&repo, &user).await;
        let ctx = auth.authenticate(&token).await.unwrap();
        assert_eq!(ctx.device_id.as_deref(), Some(device_id.as_str()));
        assert_eq!(ctx.real_user.id, user);
        // Idle expiry passed → refused.
        sqlx::query("UPDATE phone_devices SET idle_expires_at = ?")
            .bind((Utc::now() - Duration::seconds(1)).to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();
        assert!(auth.authenticate(&token).await.is_err());
        // Revoke kills it for good.
        let (token2, id2) = pair(&repo, &user).await;
        assert!(auth.authenticate(&token2).await.is_ok());
        assert!(repo.revoke(&user, &id2).await.unwrap());
        assert!(auth.authenticate(&token2).await.is_err());
        // Password reset / revoke-all-for-user drops devices too.
        let (token3, _) = pair(&repo, &user).await;
        auth.revoke_all_for_user(&user).await.unwrap();
        assert!(auth.authenticate(&token3).await.is_err());
    }

    #[tokio::test]
    async fn device_token_refused_from_non_tailnet_peer() {
        let pool = mem_pool().await;
        let repo = DevicesRepo::new(pool.clone());
        let auth = AuthRepo::new(pool.clone());
        let user = seed_user(&pool, "u").await;
        let (token, _) = pair(&repo, &user).await;
        let lan = RequestPeer {
            ip: "192.168.1.5".parse().unwrap(),
            kind: PeerKind::Other,
        };
        let r = otto_core::auth::with_request_peer(lan, auth.authenticate(&token)).await;
        assert!(
            r.is_err(),
            "a LAN / tunnel peer must not use a device token"
        );
        let ts = RequestPeer {
            ip: "100.70.0.2".parse().unwrap(),
            kind: PeerKind::Tailnet,
        };
        let r = otto_core::auth::with_request_peer(ts, auth.authenticate(&token)).await;
        assert!(r.is_ok());
        let ip: Option<String> = sqlx::query_scalar("SELECT last_ip FROM phone_devices")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(ip.as_deref(), Some("100.70.0.2"));
    }

    #[test]
    fn device_names_are_bounded() {
        assert_eq!(
            clean_device_name("  My\u{7}Phone  ").as_deref(),
            Some("MyPhone")
        );
        assert!(clean_device_name("   ").is_none());
        assert_eq!(
            clean_device_name(&"x".repeat(200)).unwrap().len(),
            DEVICE_NAME_MAX
        );
    }
}
