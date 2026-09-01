//! Site credentials for the in-app browser.
//!
//! The password NEVER lives here (or anywhere in SQLite) — [`BrowserCredential`]
//! has no password field at all, only a `keychain_ref` pointing at the actual
//! secret in the macOS Keychain (via `otto_keychain`, or the `OTTO_SECRETS=file`
//! test/dev fallback). That makes leaking the password through this type
//! structurally impossible: `serde_json::to_string(&cred)` can never contain
//! it, because there is nothing to serialize. Callers that need the password
//! (autofill, the `/reveal` route) fetch it separately through the injected
//! `SecretStore`, keyed by `keychain_ref`.
//!
//! `domain` is stored normalized (lowercased) by the caller (the route layer)
//! before it reaches this repo; [`match_domain`] implements the lookup half —
//! does a browsed host match a stored credential's domain — for the later
//! autofill feature (task 12).

use chrono::Utc;
use otto_core::{Id, Result};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::convert::{dberr, dberr_unique, fmt};

/// A stored site credential. Deliberately has **no password field** — see
/// module docs. Safe to `Serialize` and return from list/get routes.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BrowserCredential {
    pub id: Id,
    pub workspace_id: Id,
    /// eTLD+1, lowercased — see [`normalize_domain`]/[`match_domain`].
    pub domain: String,
    pub username: String,
    /// Opaque `SecretStore` key. Never the password itself.
    pub keychain_ref: String,
    pub allow_agent_use: bool,
    pub notes: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

pub struct NewBrowserCredential {
    /// Caller-supplied id (the route layer needs it up front to compute
    /// `keychain_ref = format!("browser-cred-{id}")` and write the secret to
    /// the keychain BEFORE this insert — see
    /// `crate::routes::browser::create_credential`).
    pub id: Id,
    pub workspace_id: Id,
    pub domain: String,
    pub username: String,
    pub keychain_ref: String,
    pub allow_agent_use: bool,
    pub notes: String,
}

/// Patch for `PATCH /browser/credentials/{id}`. `None` fields are left
/// unchanged. The password/`keychain_ref` are intentionally not patchable
/// here — the route layer rotates the keychain entry in place (same ref) when
/// the caller supplies a new password, so this repo never needs to touch it.
#[derive(Default)]
pub struct BrowserCredentialPatch {
    pub username: Option<String>,
    pub allow_agent_use: Option<bool>,
    pub notes: Option<String>,
}

const COLUMNS: &str = "id, workspace_id, domain, username, keychain_ref, allow_agent_use, notes, created_at, last_used_at";

#[derive(Clone)]
pub struct BrowserCredentialsRepo {
    pool: SqlitePool,
}

impl BrowserCredentialsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, new: NewBrowserCredential) -> Result<BrowserCredential> {
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO browser_credentials
                (id, workspace_id, domain, username, keychain_ref, allow_agent_use, notes, created_at, last_used_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL)",
        )
        .bind(&new.id)
        .bind(&new.workspace_id)
        .bind(&new.domain)
        .bind(&new.username)
        .bind(&new.keychain_ref)
        .bind(new.allow_agent_use)
        .bind(&new.notes)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr_unique(
            "create browser credential",
            "a credential for this domain and username already exists",
        ))?;
        self.get(&new.id)
            .await?
            .ok_or_else(|| otto_core::Error::Internal("browser credential vanished after insert".into()))
    }

    pub async fn list(&self, workspace_id: &str) -> Result<Vec<BrowserCredential>> {
        sqlx::query_as::<_, BrowserCredential>(&format!(
            "SELECT {COLUMNS} FROM browser_credentials WHERE workspace_id = ? ORDER BY domain, username"
        ))
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("browser credentials"))
    }

    pub async fn get(&self, id: &Id) -> Result<Option<BrowserCredential>> {
        sqlx::query_as::<_, BrowserCredential>(&format!(
            "SELECT {COLUMNS} FROM browser_credentials WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("browser credential"))
    }

    pub async fn update(&self, id: &Id, patch: BrowserCredentialPatch) -> Result<BrowserCredential> {
        let existing = self
            .get(id)
            .await?
            .ok_or_else(|| otto_core::Error::NotFound(format!("browser credential {id}")))?;
        let username = patch.username.unwrap_or(existing.username);
        let allow_agent_use = patch.allow_agent_use.unwrap_or(existing.allow_agent_use);
        let notes = patch.notes.unwrap_or(existing.notes);
        sqlx::query(
            "UPDATE browser_credentials SET username = ?, allow_agent_use = ?, notes = ? WHERE id = ?",
        )
        .bind(&username)
        .bind(allow_agent_use)
        .bind(&notes)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr_unique(
            "update browser credential",
            "a credential for this domain and username already exists",
        ))?;
        self.get(id)
            .await?
            .ok_or_else(|| otto_core::Error::NotFound(format!("browser credential {id}")))
    }

    pub async fn touch_last_used(&self, id: &Id) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query("UPDATE browser_credentials SET last_used_at = ? WHERE id = ?")
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("touch browser credential"))?;
        Ok(())
    }

    pub async fn delete(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM browser_credentials WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete browser credential"))?;
        Ok(())
    }
}

/// Lowercase + trim a raw host/domain for storage. Not a full eTLD+1
/// reduction — callers that need "the registrable domain for `sub.example.co.uk`"
/// should reduce first (see module docs re: no bundled public-suffix list);
/// this just normalizes casing/whitespace so lookups are consistent.
pub fn normalize_domain(raw: &str) -> String {
    raw.trim().trim_end_matches('.').to_ascii_lowercase()
}

/// Conservative registrable-domain match between a browsed `host` and a
/// stored credential `domain`.
///
/// There is no bundled Public Suffix List in this workspace (the
/// `publicsuffix` crate present in `Cargo.lock` is a transitive dep of
/// `cookie_store` and ships no embedded list — using it correctly would mean
/// vendoring/fetching PSL data, out of scope here), so this is a documented
/// approximation rather than true eTLD+1 matching:
///
/// 1. Exact match (case-insensitive) always matches.
/// 2. Otherwise, `host` matches `domain` if `host` is a strict subdomain of
///    `domain` (`host` ends with `.` + `domain`) — e.g. `login.example.com`
///    matches a stored `example.com`, but `example.com` does NOT match a
///    stored `login.example.com`, and `evilexample.com` does NOT match
///    `example.com` (the `.` prefix check rules out the suffix-without-
///    separator false positive).
///
/// This deliberately does NOT strip an arbitrary single label off `host` to
/// derive a "domain" — that would wrongly treat unrelated two-label domains
/// under a shared public suffix (e.g. `co.uk`) as related. It only ever
/// widens `host` → `domain`, matching what's actually stored, never guesses.
pub fn match_domain(host: &str, domain: &str) -> bool {
    let host = normalize_domain(host);
    let domain = normalize_domain(domain);
    if domain.is_empty() {
        return false;
    }
    host == domain || host.ends_with(&format!(".{domain}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> SqlitePool {
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

    fn new_cred(ws: &str, domain: &str, user: &str, keychain_ref: &str) -> NewBrowserCredential {
        NewBrowserCredential {
            id: otto_core::new_id(),
            workspace_id: ws.into(),
            domain: domain.into(),
            username: user.into(),
            keychain_ref: keychain_ref.into(),
            allow_agent_use: false,
            notes: String::new(),
        }
    }

    #[tokio::test]
    async fn crud_roundtrip_and_never_serializes_a_secret() {
        let pool = test_pool().await;
        let repo = BrowserCredentialsRepo::new(pool.clone());
        let cred = repo
            .create(new_cred("ws1", "example.com", "alice", "browser-cred-1"))
            .await
            .unwrap();
        assert!(!cred.allow_agent_use, "allow_agent_use must default false");

        // The struct has no password field, so no accident of implementation
        // can leak the secret through Serialize — assert the wire shape
        // literally cannot contain a plausible password string.
        let json = serde_json::to_string(&cred).unwrap();
        assert!(!json.to_lowercase().contains("password"));

        let listed = repo.list("ws1").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].domain, "example.com");

        let updated = repo
            .update(
                &cred.id,
                BrowserCredentialPatch {
                    notes: Some("rotated quarterly".into()),
                    allow_agent_use: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.notes, "rotated quarterly");
        assert!(updated.allow_agent_use);

        repo.touch_last_used(&cred.id).await.unwrap();
        let got = repo.get(&cred.id).await.unwrap().expect("credential found");
        assert!(got.last_used_at.is_some());

        repo.delete(&cred.id).await.unwrap();
        assert!(repo.get(&cred.id).await.unwrap().is_none());
        assert!(repo.list("ws1").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn unique_constraint_on_workspace_domain_username() {
        let pool = test_pool().await;
        let repo = BrowserCredentialsRepo::new(pool.clone());
        repo.create(new_cred("ws1", "example.com", "alice", "browser-cred-1"))
            .await
            .unwrap();
        let dup = repo
            .create(new_cred("ws1", "example.com", "alice", "browser-cred-2"))
            .await;
        assert!(
            matches!(dup, Err(otto_core::Error::Conflict(_))),
            "duplicate (workspace, domain, username) must Conflict, got {dup:?}"
        );

        // Different workspace, or different username, is not a conflict.
        repo.create(new_cred("ws2", "example.com", "alice", "browser-cred-3"))
            .await
            .unwrap();
        repo.create(new_cred("ws1", "example.com", "bob", "browser-cred-4"))
            .await
            .unwrap();
    }

    #[test]
    fn match_domain_exact() {
        assert!(match_domain("example.com", "example.com"));
        assert!(match_domain("Example.COM", "example.com"));
    }

    #[test]
    fn match_domain_subdomain() {
        assert!(match_domain("login.example.com", "example.com"));
        assert!(match_domain("a.b.example.com", "example.com"));
    }

    #[test]
    fn match_domain_rejects_unrelated_and_reverse() {
        assert!(!match_domain("evilexample.com", "example.com"));
        assert!(!match_domain("example.com", "login.example.com"));
        assert!(!match_domain("notexample.com", "example.com"));
    }
}
