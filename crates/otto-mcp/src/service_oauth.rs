//! OAuth sign-in state of governed remote servers (`crate::oauth` does the
//! protocol): tokens in the Keychain, attached by the governed client as
//! `Authorization: Bearer …` (agents never see them), refreshed near expiry
//! or on a 401 (one refresh, one retry; serialized per server so a rotated
//! refresh token is never spent twice), and the in-progress sign-ins.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use otto_core::{Error, Result};
use otto_state::McpServerDetail;

use super::McpService;
use crate::oauth::{self, Tokens};

/// The status word a server needing a sign-in records as its health error
/// (the UI shows "Needs sign-in" and a Sign in action for it).
pub const NEEDS_SIGN_IN: &str = "Needs sign-in";

/// One server's sign-in, as `GET /mcp/servers/{id}/oauth` reports it.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct OAuthStatus {
    pub signed_in: bool,
    pub pending: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_host: Option<String>,
}

/// A server's last sign-in: its status, and the task still waiting for the
/// browser (aborted by a new start, a sign-out or a delete).
#[derive(Default)]
struct Flow {
    status: OAuthStatus,
    task: Option<tokio::task::AbortHandle>,
}

fn flows() -> std::sync::MutexGuard<'static, HashMap<String, Flow>> {
    static FLOWS: OnceLock<Mutex<HashMap<String, Flow>>> = OnceLock::new();
    FLOWS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn set_status(id: &str, status: OAuthStatus) {
    flows().entry(id.to_string()).or_default().status = status;
}

/// Forget `id`'s flow, aborting a sign-in still waiting for the browser.
fn end_flow(id: &str) {
    if let Some(task) = flows().remove(id).and_then(|f| f.task) {
        task.abort();
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// A remote (http / sse) server: the only kind OAuth applies to.
pub(crate) fn is_remote(server: &McpServerDetail) -> bool {
    matches!(server.transport.as_str(), "http" | "sse")
}

impl McpService {
    pub(crate) async fn oauth_tokens(&self, id: &str) -> Option<Tokens> {
        let raw = otto_core::secrets::get_async(&self.secrets, &oauth::token_key(id))
            .await
            .ok()??;
        serde_json::from_str(&raw).ok()
    }

    async fn oauth_save(&self, id: &str, t: &Tokens) -> Result<()> {
        let raw = serde_json::to_string(t).map_err(|e| Error::Internal(e.to_string()))?;
        otto_core::secrets::put_async(&self.secrets, &oauth::token_key(id), &raw).await
    }

    /// `Authorization` for a signed-in remote server, refreshed when near
    /// expiry or `force`d (after a 401). `None`: not signed in, or the grant
    /// is gone (recorded as "Needs sign-in").
    pub(crate) async fn oauth_bearer(
        &self,
        server: &McpServerDetail,
        force: bool,
    ) -> Option<String> {
        if !is_remote(server) {
            return None;
        }
        let mut t = self.oauth_tokens(&server.id).await?;
        if !bearer_allowed(server.url.as_deref(), &t) {
            let _ = self
                .record_health(
                    server,
                    Err(format!(
                        "{NEEDS_SIGN_IN}: the server's URL changed since sign-in"
                    )),
                    None,
                )
                .await;
            return None;
        }
        if force || t.needs_refresh(now()) {
            let stale = t.access_token.clone();
            let _guard = self
                .secret_locks
                .lock(&format!("oauth:{}", server.id))
                .await;
            t = self.oauth_tokens(&server.id).await?;
            // Another caller may have refreshed while this one waited.
            if (force && t.access_token == stale) || t.needs_refresh(now()) {
                match oauth::refresh(&t, now()).await {
                    Ok(fresh) => {
                        let _ = self.oauth_save(&server.id, &fresh).await;
                        t = fresh;
                    }
                    Err(e) => {
                        let _ = self
                            .record_health(server, Err(format!("{NEEDS_SIGN_IN}: {e}")), None)
                            .await;
                        return None;
                    }
                }
            }
        }
        Some(format!("Bearer {}", t.access_token))
    }

    /// Start a sign-in: the authorization URL for the browser and the
    /// authorization host to confirm with the person. The flow finishes in
    /// the background when the browser comes back (≤ 5 min).
    pub async fn oauth_start(
        self: &Arc<Self>,
        server: &McpServerDetail,
        manual: Option<(String, Option<String>)>,
    ) -> Result<(String, String)> {
        if !is_remote(server) {
            return Err(Error::Invalid(
                "only a remote (http or sse) server signs in".into(),
            ));
        }
        let url = server.url.clone().unwrap_or_default();
        let (auth_url, pending) = oauth::start(&url, manual).await.map_err(Error::Upstream)?;
        let host = pending.base.auth_host.clone();
        // A new start replaces the pending one (its listener closes).
        end_flow(&server.id);
        let (svc, id) = (self.clone(), server.id.clone());
        let task = tokio::spawn(async move {
            let done = oauth::finish(pending, now).await;
            // Saved under the lock sign-out takes: a sign-out that won
            // already aborted this task.
            let _guard = svc.secret_locks.lock(&format!("oauth:{id}")).await;
            let status = match done {
                Ok(t) => match svc.oauth_save(&id, &t).await {
                    Ok(()) => {
                        let _ = svc.registry().set_health(&id, "unknown", None, None).await;
                        svc.drop_client(&id);
                        OAuthStatus {
                            signed_in: true,
                            auth_host: Some(t.auth_host),
                            ..Default::default()
                        }
                    }
                    Err(_) => OAuthStatus {
                        error: Some("the Keychain refused the tokens".into()),
                        ..Default::default()
                    },
                },
                Err(e) => OAuthStatus {
                    error: Some(e),
                    ..Default::default()
                },
            };
            set_status(&id, status);
        });
        flows().insert(
            server.id.clone(),
            Flow {
                status: OAuthStatus {
                    pending: true,
                    auth_host: Some(host.clone()),
                    ..Default::default()
                },
                task: Some(task.abort_handle()),
            },
        );
        Ok((auth_url, host))
    }

    pub async fn oauth_status(&self, id: &str) -> OAuthStatus {
        let mut s = flows()
            .get(id)
            .map(|f| f.status.clone())
            .unwrap_or_default();
        if let Some(t) = self.oauth_tokens(id).await {
            s.signed_in = true;
            s.auth_host.get_or_insert(t.auth_host);
        }
        s
    }

    /// Sign out: a pending sign-in is aborted, and the tokens and client
    /// registration leave the Keychain — under the lock a refresh holds, so
    /// neither can save them back afterwards.
    pub async fn oauth_sign_out(&self, id: &str) -> Result<()> {
        let _guard = self.secret_locks.lock(&format!("oauth:{id}")).await;
        end_flow(id);
        otto_core::secrets::delete_async(&self.secrets, &oauth::token_key(id)).await?;
        self.drop_client(id);
        Ok(())
    }

    /// A deleted server's Keychain entries: its credentials blob and its
    /// OAuth tokens (best effort; the row is gone either way).
    pub async fn forget_server_secrets(&self, id: &str) {
        let _ = otto_core::secrets::delete_async(&self.secrets, &Self::secret_ref(id)).await;
        let _ = self.oauth_sign_out(id).await;
    }

    /// Forget the pooled client (its headers carried the old token).
    pub(crate) fn drop_client(&self, id: &str) {
        if let Ok(mut m) = self.clients.lock() {
            m.remove(id);
        }
    }
}

/// Whether `t` may ride on requests to `server_url`: only to the URL it
/// was issued for (a re-pointed server needs a new sign-in), and never in
/// cleartext off this Mac.
fn bearer_allowed(server_url: Option<&str>, t: &Tokens) -> bool {
    server_url.is_some_and(|u| u == t.resource && otto_netguard::require_tls_or_loopback(u).is_ok())
}

/// A transport error that is the server asking for authorization.
pub(crate) fn is_unauthorized(error: &str) -> bool {
    error.contains("http 401")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(resource: &str) -> Tokens {
        Tokens {
            access_token: "at".into(),
            refresh_token: None,
            expires_at: None,
            token_endpoint: String::new(),
            client_id: String::new(),
            client_secret: None,
            resource: resource.into(),
            auth_host: String::new(),
        }
    }

    #[test]
    fn a_bearer_never_travels_in_cleartext_off_this_mac() {
        let ok = |u: &str| bearer_allowed(Some(u), &tokens(u));
        assert!(ok("https://mcp.example/mcp"));
        assert!(ok("http://127.0.0.1:9/mcp"));
        assert!(!ok("http://mcp.example/mcp"));
        assert!(!ok("http://10.1.2.3/mcp"));
        assert!(!bearer_allowed(None, &tokens("")));
        // Issued for another URL: the re-pointed server gets no bearer.
        assert!(!bearer_allowed(
            Some("https://new.example/mcp"),
            &tokens("https://old.example/mcp")
        ));
    }
}
