//! Shared plumbing for governed self-calls (r3-06-04).
//!
//! A governed `otto_*` MCP tool call runs as an HTTP call back into this
//! daemon, as the effective user, so the target route's own RBAC decides.
//! Each call used to mint an API token (INSERT), build a fresh
//! `reqwest::Client` (rustls config + a new connection pool), open a new
//! loopback TCP connection, miss the auth cache on the brand-new token
//! (SELECT), and revoke the token on the way out (DELETE) — on top of the two
//! fail-closed audit writes. With several agents calling tools that was 4–5
//! SQLite writes per call competing for the single writer.
//!
//! Now:
//! - one process-wide [`client`] keeps loopback connections alive across
//!   calls (callers apply their own per-call timeout);
//! - one token per (database, daemon, user, purpose) is minted and reused for
//!   [`TOKEN_REUSE`], so steady-state calls hit the auth cache and write
//!   nothing for the credential. A token is revoked once it has aged out AND
//!   no call still holds it ([`Lease`] tracks that), so a rotation never
//!   pulls a credential from under an in-flight call;
//! - tokens this cache minted that a previous daemon process never got to
//!   revoke (a crash, a restart) are deleted the first time the cache is used
//!   against a database.
//!
//! RBAC is unchanged: the token is still an ordinary API token of the same
//! effective user, presented to the same routes.
//!
//! The credential CLASS follows the caller (S8-305): a self-call made for an
//! agent session is minted bound to that session (`session_scope`, so the
//! route sees `managed_session_id` and `ui_bridge::is_human` is false) — the
//! credential-class gate, root withholding and every agent-credential rule
//! apply to the agent behind a governed tool exactly as to its own token. Only
//! a person's own call (or a person-only tool a person just approved, see
//! `mcp_outward::SelfCallAs`) replays as a person-classed token.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use otto_core::{Error, Id};
use otto_rbac::AuthRepo;
use otto_state::DbPool;

/// How long one self-call token is handed out before a fresh one is minted.
/// Short: a user whose tokens are revoked (password reset, "revoke all")
/// sees at most this long of failing self-calls before a fresh mint — and
/// the old credential is revoked as soon as its last call returns.
pub(crate) const TOKEN_REUSE: Duration = Duration::from_secs(60);

/// Labels of the tokens this cache mints (one per purpose). Leftovers under
/// these labels are only ever this cache's own.
pub(crate) const LABEL_EXEC: &str = "mcp-otto-exec";
pub(crate) const LABEL_REFS: &str = "mcp-otto-refs";
const LABELS: [&str; 2] = [LABEL_EXEC, LABEL_REFS];

/// The shared self-call HTTP client. No client-wide timeout: each caller
/// bounds its own call (`RequestBuilder::timeout` or `tokio::time::timeout`).
pub(crate) fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .pool_idle_timeout(Duration::from_secs(60))
            .build()
            .expect("build self-call http client")
    })
}

/// (database id, daemon base URL, user, purpose, bound agent session).
type Key = (u64, String, Id, &'static str, Option<Id>);

struct Entry {
    token: String,
    /// The credential's `auth_sessions.id` — a session-bound one is revoked
    /// with its session, so the fast path re-checks it still exists.
    row_id: String,
    minted: Instant,
    in_flight: usize,
}

#[derive(Default)]
struct Tokens {
    live: HashMap<Key, Entry>,
    /// Aged-out tokens still held by in-flight calls: token → holders.
    retiring: HashMap<String, (usize, DbPool)>,
    /// Databases whose leftover tokens were already swept.
    swept: HashSet<u64>,
}

fn tokens() -> &'static Mutex<Tokens> {
    static TOKENS: OnceLock<Mutex<Tokens>> = OnceLock::new();
    TOKENS.get_or_init(Default::default)
}

/// A held self-call token. Dropping it releases the hold; an aged-out token
/// is revoked when its last holder lets go.
pub(crate) struct Lease {
    token: String,
    key: Key,
    pool: DbPool,
}

impl Lease {
    pub(crate) fn token(&self) -> &str {
        &self.token
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let revoke = {
            let mut t = tokens().lock().unwrap_or_else(|e| e.into_inner());
            match t.live.get_mut(&self.key) {
                Some(e) if e.token == self.token => {
                    e.in_flight = e.in_flight.saturating_sub(1);
                    false
                }
                _ => match t.retiring.get_mut(&self.token) {
                    Some((n, _)) => {
                        *n = n.saturating_sub(1);
                        if *n == 0 {
                            t.retiring.remove(&self.token);
                            true
                        } else {
                            false
                        }
                    }
                    None => false,
                },
            }
        };
        if revoke {
            spawn_revoke(self.pool.clone(), self.token.clone());
        }
    }
}

fn spawn_revoke(pool: DbPool, token: String) {
    if let Ok(rt) = tokio::runtime::Handle::try_current() {
        rt.spawn(async move {
            let _ = AuthRepo::new(pool).revoke(&token).await;
        });
    }
}

/// Take a self-call token for `user_id` (minting one when none is fresh).
/// `session` binds it to that agent session (a non-human credential, see the
/// module doc); `None` is a person-classed token, for a person's own calls.
pub(crate) async fn lease(
    pool: &DbPool,
    base_url: &str,
    user_id: &Id,
    label: &'static str,
    session: Option<&Id>,
) -> Result<Lease, Error> {
    sweep_leftovers(pool).await;
    let key: Key = (
        pool.id(),
        base_url.to_string(),
        user_id.clone(),
        label,
        session.cloned(),
    );
    // A session-bound credential dies with its session (removal, respawn's
    // `revoke_session_tokens`): drop a cached one that no longer exists so
    // the fast path never hands out a revoked token.
    if session.is_some() {
        let cached = {
            let t = tokens().lock().unwrap_or_else(|e| e.into_inner());
            t.live
                .get(&key)
                .filter(|e| e.minted.elapsed() < TOKEN_REUSE)
                .map(|e| e.row_id.clone())
        };
        if let Some(row_id) = cached {
            let alive: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM auth_sessions WHERE id = ? AND revoked = 0)",
            )
            .bind(&row_id)
            .fetch_one(pool)
            .await
            .unwrap_or(false);
            if !alive {
                let mut t = tokens().lock().unwrap_or_else(|e| e.into_inner());
                if t.live.get(&key).is_some_and(|e| e.row_id == row_id) {
                    if let Some(old) = t.live.remove(&key) {
                        if old.in_flight > 0 {
                            t.retiring.insert(old.token, (old.in_flight, pool.clone()));
                        }
                    }
                }
            }
        }
    }
    // Fast path: a fresh token.
    {
        let mut t = tokens().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = t.live.get_mut(&key) {
            if e.minted.elapsed() < TOKEN_REUSE {
                e.in_flight += 1;
                return Ok(Lease {
                    token: e.token.clone(),
                    key,
                    pool: pool.clone(),
                });
            }
        }
    }
    // Mint outside the lock; two racing callers may both mint — the loser's
    // token is retired right away below.
    let repo = AuthRepo::new(pool.clone());
    let (token, info) = match session {
        Some(sid) => {
            repo.issue_session_api_token_labeled(user_id, sid, label)
                .await?
        }
        None => repo.issue_api_token(user_id, Some(label)).await?,
    };
    let mut revoke_now = Vec::new();
    {
        let mut t = tokens().lock().unwrap_or_else(|e| e.into_inner());
        let fresh = Entry {
            token: token.clone(),
            row_id: info.id,
            minted: Instant::now(),
            in_flight: 1,
        };
        if let Some(old) = t.live.insert(key.clone(), fresh) {
            if old.in_flight == 0 {
                revoke_now.push(old.token);
            } else {
                t.retiring.insert(old.token, (old.in_flight, pool.clone()));
            }
        }
    }
    for old in revoke_now {
        let _ = AuthRepo::new(pool.clone()).revoke(&old).await;
    }
    Ok(Lease {
        token,
        key,
        pool: pool.clone(),
    })
}

/// Once per database: delete self-call tokens a previous daemon process
/// minted and never revoked. Nothing in THIS process holds one yet (the
/// sweep runs before this process's first mint against the database).
async fn sweep_leftovers(pool: &DbPool) {
    {
        let mut t = tokens().lock().unwrap_or_else(|e| e.into_inner());
        if !t.swept.insert(pool.id()) {
            return;
        }
    }
    let _ = sqlx::query("DELETE FROM auth_sessions WHERE kind = 'api' AND label IN (?, ?)")
        .bind(LABELS[0])
        .bind(LABELS[1])
        .execute(pool)
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> (DbPool, Id) {
        let pool = otto_state::db::test_pool().await;
        let user = otto_state::UsersRepo::new(pool.clone())
            .create("selfcall", "hash", "Self Call", false)
            .await
            .unwrap();
        (pool, user.id)
    }

    async fn api_tokens(pool: &DbPool, user: &Id) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM auth_sessions WHERE user_id = ? AND kind = 'api'")
            .bind(user)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// Steady state: many calls, ONE credential row — and it authenticates.
    #[tokio::test]
    async fn reuses_one_token_per_user_and_purpose() {
        let (pool, user) = db().await;
        let base = "http://127.0.0.1:1";
        let a = lease(&pool, base, &user, LABEL_EXEC, None).await.unwrap();
        let b = lease(&pool, base, &user, LABEL_EXEC, None).await.unwrap();
        assert_eq!(a.token(), b.token());
        drop((a, b));
        for _ in 0..20 {
            let l = lease(&pool, base, &user, LABEL_EXEC, None).await.unwrap();
            drop(l);
        }
        assert_eq!(api_tokens(&pool, &user).await, 1);
        let l = lease(&pool, base, &user, LABEL_EXEC, None).await.unwrap();
        let ctx = AuthRepo::new(pool.clone())
            .authenticate(l.token())
            .await
            .unwrap();
        assert_eq!(ctx.effective_user.id, user);
        // A different purpose gets its own credential.
        let r = lease(&pool, base, &user, LABEL_REFS, None).await.unwrap();
        assert_ne!(r.token(), l.token());
    }

    /// An aged-out token is replaced, and revoked only once its last
    /// in-flight holder is done.
    #[tokio::test]
    async fn rotation_never_revokes_a_token_in_use() {
        let (pool, user) = db().await;
        let base = "http://127.0.0.1:2";
        let held = lease(&pool, base, &user, LABEL_EXEC, None).await.unwrap();
        let old = held.token().to_string();
        // Age it out.
        {
            let mut t = tokens().lock().unwrap();
            let key: Key = (pool.id(), base.to_string(), user.clone(), LABEL_EXEC, None);
            t.live.get_mut(&key).unwrap().minted = Instant::now() - TOKEN_REUSE * 2;
        }
        let next = lease(&pool, base, &user, LABEL_EXEC, None).await.unwrap();
        assert_ne!(next.token(), old);
        // The old one still authenticates while `held` is in flight…
        assert!(AuthRepo::new(pool.clone()).authenticate(&old).await.is_ok());
        drop(held);
        // …and is revoked (spawned) once released.
        for _ in 0..50 {
            if AuthRepo::new(pool.clone())
                .authenticate(&old)
                .await
                .is_err()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(AuthRepo::new(pool.clone())
            .authenticate(&old)
            .await
            .is_err());
        assert!(AuthRepo::new(pool.clone())
            .authenticate(next.token())
            .await
            .is_ok());
    }

    /// A crashed daemon's leftover self-call tokens are swept on first use;
    /// the user's other API tokens are untouched.
    #[tokio::test]
    async fn sweeps_leftover_tokens_once_per_database() {
        let (pool, user) = db().await;
        let repo = AuthRepo::new(pool.clone());
        let (stale, _) = repo.issue_api_token(&user, Some(LABEL_EXEC)).await.unwrap();
        let (mine, _) = repo
            .issue_api_token(&user, Some("my laptop"))
            .await
            .unwrap();
        let l = lease(&pool, "http://127.0.0.1:3", &user, LABEL_EXEC, None)
            .await
            .unwrap();
        assert!(repo.authenticate(&stale).await.is_err(), "leftover swept");
        assert!(
            repo.authenticate(&mine).await.is_ok(),
            "user's own token kept"
        );
        assert!(repo.authenticate(l.token()).await.is_ok());
    }

    /// S8-305: a self-call made for an agent session carries that session's
    /// binding (non-human), is cached per session, and is never handed out
    /// again once the session's credentials were revoked.
    #[tokio::test]
    async fn session_bound_tokens_are_agent_credentials() {
        let (pool, user) = db().await;
        let now = chrono::Utc::now().to_rfc3339();
        let ws = otto_core::new_id();
        let sid = otto_core::new_id();
        sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, 'w', '/tmp', ?)")
            .bind(&ws)
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO sessions (id, workspace_id, kind, provider, title, status, cwd, created_by, created_at, last_active_at)
             VALUES (?, ?, 'agent', 'claude', 't', 'idle', '/tmp', ?, ?, ?)",
        )
        .bind(&sid)
        .bind(&ws)
        .bind(&user)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();
        let base = "http://127.0.0.1:4";
        let repo = AuthRepo::new(pool.clone());
        let bound = lease(&pool, base, &user, LABEL_EXEC, Some(&sid))
            .await
            .unwrap();
        let ctx = repo.authenticate(bound.token()).await.unwrap();
        assert_eq!(ctx.managed_session_id.as_ref(), Some(&sid));
        assert!(!crate::ui_bridge::is_human(&ctx), "agent-classed");
        // A person's own self-call stays person-classed, and separate.
        let person = lease(&pool, base, &user, LABEL_EXEC, None).await.unwrap();
        assert_ne!(person.token(), bound.token());
        let pctx = repo.authenticate(person.token()).await.unwrap();
        assert!(crate::ui_bridge::is_human(&pctx));
        // Reused while the session lives…
        let again = lease(&pool, base, &user, LABEL_EXEC, Some(&sid))
            .await
            .unwrap();
        assert_eq!(again.token(), bound.token());
        let old = bound.token().to_string();
        drop((bound, again));
        // …and replaced once the session's credentials are revoked.
        repo.revoke_session_tokens(&user, &sid).await.unwrap();
        let fresh = lease(&pool, base, &user, LABEL_EXEC, Some(&sid))
            .await
            .unwrap();
        assert_ne!(fresh.token(), old);
        assert!(repo.authenticate(fresh.token()).await.is_ok());
        // A session the user does not own cannot be bound.
        let other = otto_state::UsersRepo::new(pool.clone())
            .create("other", "hash", "Other", false)
            .await
            .unwrap();
        assert!(lease(&pool, base, &other.id, LABEL_EXEC, Some(&sid))
            .await
            .is_err());
    }
}
