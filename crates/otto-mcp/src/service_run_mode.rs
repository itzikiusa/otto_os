//! Run mode: how a registered server reaches agents. NATIVE: the agent CLI
//! launches it from `.mcp.json` and its tools keep their own names; nothing is
//! governed. GOVERNED: Otto's gateway runs it, under policy, approval and
//! audit (`mcp__<server>__<tool>`). The `managed` column IS the mode, so one
//! row is never both (docs/features/mcp-control-plane.md, "Native and
//! governed servers").
//!
//! A native server is written into a file every session in its workspace
//! reads, so per-user access rules can't follow it: `list_native_enabled`
//! refuses an Enforced one, and the gateway / invoke gates skip an unmanaged
//! Legacy one. The switch therefore moves the access policy with the flag.
//! The rules are KEPT on the Legacy policy, so switching back restores them.
//! Secrets need no move: both modes read the same Keychain blob `mcp-{id}`
//! through the row's `secret_ref`; no value is copied anywhere.

use super::*;
use otto_core::access::{AccessActor, AccessMode, ResourceKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunMode {
    Native,
    Governed,
}

impl RunMode {
    pub fn of(server: &McpServerDetail) -> Self {
        if server.managed {
            RunMode::Governed
        } else {
            RunMode::Native
        }
    }
}

impl McpService {
    /// Switch a server between native and governed. The caller has checked
    /// the person may (the owner: either way the server's command runs on
    /// this Mac). Ordered so that between the two writes the server is
    /// governed only, never in `.mcp.json` and the gateway at once.
    pub async fn set_run_mode(
        &self,
        server_id: &str,
        mode: RunMode,
        actor: &AccessActor,
    ) -> Result<McpServerDetail> {
        let id = server_id.to_string();
        let server = self.registry().get(&id).await?;
        let access = otto_state::ResourceAccessRepo::new(self.pool.clone());
        let policy = access.get_policy(ResourceKind::McpServer, &id).await?;
        match mode {
            RunMode::Native => {
                if server.transport != "stdio" {
                    return Err(Error::Invalid(
                        "only a stdio server can run natively: the agent CLI launches its command"
                            .into(),
                    ));
                }
                if policy.mode == AccessMode::Enforced {
                    let mut legacy = policy.clone();
                    legacy.mode = AccessMode::Legacy;
                    access.put_policy(&legacy, policy.revision, actor).await?;
                }
                self.evict_client(&id);
                self.registry().set_managed(&id, false).await
            }
            RunMode::Governed => {
                let out = self.registry().set_managed(&id, true).await?;
                // A server that never had rules stays Legacy: that is how it
                // already ran through the gateway.
                if policy.mode == AccessMode::Legacy && !policy.rules.is_empty() {
                    let mut enforced = policy.clone();
                    enforced.mode = AccessMode::Enforced;
                    access.put_policy(&enforced, policy.revision, actor).await?;
                }
                Ok(out)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MemSecrets(Mutex<HashMap<String, String>>);
    impl SecretStore for MemSecrets {
        fn put(&self, k: &str, v: &str) -> Result<()> {
            self.0.lock().unwrap().insert(k.into(), v.into());
            Ok(())
        }
        fn get(&self, k: &str) -> Result<Option<String>> {
            Ok(self.0.lock().unwrap().get(k).cloned())
        }
        fn delete(&self, k: &str) -> Result<()> {
            self.0.lock().unwrap().remove(k);
            Ok(())
        }
    }

    fn actor() -> AccessActor {
        AccessActor {
            real_user_id: "root".into(),
            effective_user_id: None,
        }
    }

    async fn setup(transport: &str) -> (McpService, otto_state::DbPool, McpServerDetail) {
        let pool: otto_state::DbPool = {
            let p = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .unwrap();
            sqlx::migrate!("../otto-state/migrations")
                .run(&p)
                .await
                .unwrap();
            p.into()
        };
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES ('root', 'root', 'x', 'root', 1, ?)")
            .bind(&now).execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, created_at) VALUES ('w', 'w', '/tmp', ?)",
        )
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();
        let svc = McpService::new(
            pool.clone(),
            Arc::new(MemSecrets(Mutex::new(HashMap::new()))),
        );
        let server = svc
            .registry()
            .create(otto_state::NewServerRow {
                workspace_id: "w".into(),
                name: "linear".into(),
                transport: transport.into(),
                command: if transport == "stdio" {
                    "npx".into()
                } else {
                    String::new()
                },
                args: vec!["-y".into(), "@linear/mcp".into()],
                env: Default::default(),
                url: if transport == "http" {
                    Some("https://example.com/mcp".into())
                } else {
                    None
                },
                description: None,
                headers: Default::default(),
                secret_ref: None,
                secret_env_keys: vec![],
                secret_header_keys: vec![],
                injection_risk: "low".into(),
                default_tool_access: "allow".into(),
                enabled: true,
                created_by: "root".into(),
            })
            .await
            .unwrap();
        // The owner rule a create installs (`initialize_owner_policy`).
        otto_state::ResourceAccessRepo::new(pool.clone())
            .initialize_owner_policy(
                ResourceKind::McpServer,
                &server.id,
                &"root".to_string(),
                &otto_core::access::operations_for(ResourceKind::McpServer)
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>(),
                &[],
                &actor(),
            )
            .await
            .unwrap();
        (svc, pool, server)
    }

    async fn policy_of(pool: &otto_state::DbPool, id: &str) -> otto_core::access::AccessPolicy {
        otto_state::ResourceAccessRepo::new(pool.clone())
            .get_policy(ResourceKind::McpServer, &id.to_string())
            .await
            .unwrap()
    }

    async fn native_names(svc: &McpService) -> Vec<String> {
        svc.registry()
            .list_native_enabled(&"w".to_string())
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect()
    }

    /// One row, one mode: the merged list (`list_for_ws`) carries the server
    /// once, labelled by `managed`, and `.mcp.json` takes it only as native.
    #[tokio::test]
    async fn one_list_shows_each_server_once_with_its_mode() {
        let (svc, _pool, server) = setup("stdio").await;
        let ws = "w".to_string();
        let list = svc.registry().list_for_ws(&ws).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(RunMode::of(&list[0]), RunMode::Governed);
        assert!(native_names(&svc).await.is_empty());
        svc.set_run_mode(&server.id, RunMode::Native, &actor())
            .await
            .unwrap();
        let list = svc.registry().list_for_ws(&ws).await.unwrap();
        assert_eq!(list.len(), 1, "switching never duplicates the row");
        assert_eq!(RunMode::of(&list[0]), RunMode::Native);
        assert_eq!(serde_json::to_value(RunMode::Native).unwrap(), "native");
    }

    /// Governed → native: unmanaged, written to `.mcp.json`, out of the
    /// gateway's reach (unmanaged + Legacy), secrets left as references.
    #[tokio::test]
    async fn to_native_writes_mcp_json_and_leaves_the_gateway() {
        let (svc, pool, server) = setup("stdio").await;
        let sref = McpService::secret_ref(&server.id);
        otto_core::secrets::put_async(&svc.secrets, &sref, r#"{"env":{"TOKEN":"tk"}}"#)
            .await
            .unwrap();
        svc.registry()
            .set_secret_meta(&server.id, Some(&sref), &["TOKEN".to_string()], &[])
            .await
            .unwrap();
        let out = svc
            .set_run_mode(&server.id, RunMode::Native, &actor())
            .await
            .unwrap();
        assert!(!out.managed);
        let policy = policy_of(&pool, &server.id).await;
        assert_eq!(policy.mode, AccessMode::Legacy);
        assert_eq!(
            policy.rules.len(),
            1,
            "the owner rule is kept for the way back"
        );
        assert_eq!(native_names(&svc).await, ["linear"]);
        // The secret stays a reference: key names on the row, value only in
        // the store, nowhere in SQLite.
        assert_eq!(out.secret_env_keys, ["TOKEN"]);
        assert!(out.has_secret);
        let dump: Vec<String> = sqlx::query_scalar(
            "SELECT env_json || secret_env_keys || IFNULL(secret_ref, '') FROM mcp_servers",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert!(dump.iter().all(|r| !r.contains("\"tk\"")), "{dump:?}");
        // Switching again is a no-op, not a second row or a second policy bump.
        let rev = policy.revision;
        svc.set_run_mode(&server.id, RunMode::Native, &actor())
            .await
            .unwrap();
        assert_eq!(policy_of(&pool, &server.id).await.revision, rev);
        assert_eq!(native_names(&svc).await, ["linear"]);
    }

    /// Native → governed reverses it: managed again, gone from `.mcp.json`,
    /// and the per-user rules enforced again.
    #[tokio::test]
    async fn to_governed_reverses_it() {
        let (svc, pool, server) = setup("stdio").await;
        svc.set_run_mode(&server.id, RunMode::Native, &actor())
            .await
            .unwrap();
        let out = svc
            .set_run_mode(&server.id, RunMode::Governed, &actor())
            .await
            .unwrap();
        assert!(out.managed);
        assert!(native_names(&svc).await.is_empty());
        let policy = policy_of(&pool, &server.id).await;
        assert_eq!(policy.mode, AccessMode::Enforced);
        assert_eq!(policy.rules.len(), 1);
    }

    /// A server that never had rules (it predates access policies) stays
    /// Legacy when governed: that is how it already ran in the gateway.
    #[tokio::test]
    async fn a_ruleless_legacy_server_stays_legacy_when_governed() {
        let (svc, pool, server) = setup("stdio").await;
        sqlx::query("DELETE FROM resource_access_policies WHERE resource_id = ?")
            .bind(&server.id)
            .execute(&pool)
            .await
            .unwrap();
        svc.registry().set_managed(&server.id, false).await.unwrap();
        assert_eq!(native_names(&svc).await, ["linear"]);
        svc.set_run_mode(&server.id, RunMode::Governed, &actor())
            .await
            .unwrap();
        assert!(native_names(&svc).await.is_empty());
        assert_eq!(policy_of(&pool, &server.id).await.mode, AccessMode::Legacy);
    }

    /// An http server has no command for an agent CLI to launch.
    #[tokio::test]
    async fn an_http_server_cannot_run_natively() {
        let (svc, pool, server) = setup("http").await;
        let err = svc
            .set_run_mode(&server.id, RunMode::Native, &actor())
            .await
            .expect_err("http can't be native");
        assert!(matches!(err, Error::Invalid(_)), "{err:?}");
        assert!(svc.registry().get(&server.id).await.unwrap().managed);
        assert_eq!(
            policy_of(&pool, &server.id).await.mode,
            AccessMode::Enforced
        );
    }
}
