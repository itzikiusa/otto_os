//! Who may RESUME an assist session (architecture-5-02). Shared artifact
//! visibility (a canvas scene, a Design Hall artifact, a story mockup) does
//! not grant authority to resume its owner's agent session: the turn runner
//! checks the session's authoritative DB row before it resumes `existing`.
//! Lived in otto-server (`agent_session::require_owned_resume`, over the raw
//! pool); the runner owns it now, so an engine crate needs no host forwarder
//! to run a turn safely. Missing old sessions keep the runner's fresh-session
//! fallback.

use std::future::Future;

use otto_core::domain::Session;
use otto_core::{Error, Id, Result};

/// Reads the authoritative row of a session the caller asks to resume.
pub trait ResumeAuthority: Send + Sync {
    /// Session `id`'s persisted row (`Error::NotFound` when there is none).
    fn session_row(&self, id: &Id) -> impl Future<Output = Result<Session>> + Send;
}

/// The session manager's lookup reads the same `sessions` row (its live map
/// is never consulted for ownership).
impl ResumeAuthority for otto_sessions::SessionManager {
    fn session_row(&self, id: &Id) -> impl Future<Output = Result<Session>> + Send {
        self.get(id)
    }
}

impl ResumeAuthority for otto_state::SessionsRepo {
    fn session_row(&self, id: &Id) -> impl Future<Output = Result<Session>> + Send {
        self.get(id)
    }
}

/// `Ok` when `existing` is absent, unknown, or owned by `user` in
/// `workspace`; `Error::Forbidden` (403) for another user's or another
/// workspace's session, live or persisted.
pub async fn require_owned_resume<A: ResumeAuthority>(
    authority: &A,
    workspace: &str,
    user: &str,
    existing: Option<&Id>,
) -> Result<()> {
    let Some(id) = existing else {
        return Ok(());
    };
    match authority.session_row(id).await {
        Ok(session) if session.workspace_id == workspace && session.created_by == user => Ok(()),
        Ok(_) => Err(Error::Forbidden(
            "cannot resume an assistant session owned by another user or workspace".into(),
        )),
        Err(Error::NotFound(_)) => Ok(()),
        Err(error) => Err(error),
    }
}

/// Moved from otto-server's `tests/unit/assist_resume_ownership.rs`, now
/// against both authorities (the repo and the session manager).
#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::domain::SessionStatus;
    use otto_state::{NewSession, SessionsRepo, UsersRepo, WorkspacesRepo};

    struct Fixture {
        repo: SessionsRepo,
        manager: otto_sessions::SessionManager,
        ws: Id,
        alice: Id,
        bob: Id,
        /// A running and an exited session, both alice's.
        owned: [Id; 2],
        _dir: tempfile::TempDir,
    }

    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let pool = otto_state::open(&dir.path().join("state.db"))
            .await
            .unwrap();
        let users = UsersRepo::new(pool.clone());
        let alice = users.create("alice", "x", "A", false).await.unwrap().id;
        let bob = users.create("bob", "x", "B", false).await.unwrap().id;
        let ws = WorkspacesRepo::new(pool.clone())
            .create("w", dir.path().to_str().unwrap(), &alice)
            .await
            .unwrap()
            .id;
        let repo = SessionsRepo::new(pool);
        let mut owned = Vec::new();
        for status in [SessionStatus::Running, SessionStatus::Exited] {
            let s = repo
                .create(NewSession {
                    workspace_id: ws.clone(),
                    kind: otto_core::domain::SessionKind::Agent,
                    provider: "claude".into(),
                    title: "Owned assist".into(),
                    cwd: "/tmp".into(),
                    provider_session_id: None,
                    connection_id: None,
                    created_by: alice.clone(),
                    meta: serde_json::Value::Null,
                })
                .await
                .unwrap();
            repo.update_status(&s.id, status).await.unwrap();
            owned.push(s.id);
        }
        let (events, _rx) = tokio::sync::broadcast::channel(4);
        let manager = otto_sessions::SessionManager::new(
            repo.clone(),
            events,
            otto_sessions::ProviderRegistry::new(None),
        );
        Fixture {
            repo,
            manager,
            ws,
            alice,
            bob,
            owned: [owned[0].clone(), owned[1].clone()],
            _dir: dir,
        }
    }

    async fn check(f: &Fixture, ws: &str, user: &str, id: Option<&Id>) -> [Result<()>; 2] {
        [
            require_owned_resume(&f.repo, ws, user, id).await,
            require_owned_resume(&f.manager, ws, user, id).await,
        ]
    }

    #[tokio::test]
    async fn owner_can_resume_live_and_persisted_sessions() {
        let f = fixture().await;
        for id in &f.owned {
            for r in check(&f, &f.ws, &f.alice, Some(id)).await {
                r.unwrap();
            }
        }
    }

    #[tokio::test]
    async fn absent_binding_retains_fresh_session_fallback() {
        let f = fixture().await;
        for r in check(&f, &f.ws, &f.alice, None).await {
            r.unwrap();
        }
        for r in check(&f, &f.ws, &f.alice, Some(&"missing".into())).await {
            r.unwrap();
        }
    }

    #[tokio::test]
    async fn foreign_user_cannot_resume_live_or_persisted_session() {
        let f = fixture().await;
        for id in &f.owned {
            for r in check(&f, &f.ws, &f.bob, Some(id)).await {
                assert!(matches!(r, Err(Error::Forbidden(_))), "{r:?}");
            }
        }
    }

    #[tokio::test]
    async fn foreign_workspace_cannot_resume_live_or_persisted_session() {
        let f = fixture().await;
        for id in &f.owned {
            for r in check(&f, "another-ws", &f.alice, Some(id)).await {
                assert!(matches!(r, Err(Error::Forbidden(_))), "{r:?}");
            }
        }
    }
}
