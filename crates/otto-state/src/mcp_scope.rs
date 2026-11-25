//! Where an MCP server is available (migration 0232): its home workspace,
//! the workspaces it was shared with, or every workspace. One server row
//! stays the one place to edit it — one secret blob, one OAuth token set, one
//! health state. [`available_in`] is the ONE predicate behind every "which
//! MCP servers does workspace W have / may W use server S" read: the registry
//! list and gateway tools (`McpRegistryRepo::list_for_ws`),
//! [`McpRegistryRepo::is_available`] (gateway / UI invoke, allowlists) and
//! [`McpRegistryRepo::reach_workspaces`] (who may use it from elsewhere).
//!
//! Only a GOVERNED server is shared: a native one is written into agent
//! config files, so it stays in its home (`list_native_enabled`, the
//! native-config route). Scratch — every user's implicit workspace — and an
//! archived workspace are never reached through sharing.

use otto_core::domain::SCRATCH_WORKSPACE_ID;
use otto_core::{Error, Id, Result};

use crate::convert::{dberr, fmt};
use crate::{McpRegistryRepo, McpServerDetail};

/// SQL: is the `mcp_servers s` row available in workspace `ws` (an SQL
/// expression such as `?1` or `m.workspace_id`)? Its home; or, for a
/// governed server, a live (not archived, not Scratch) workspace it was
/// shared with, or any for `all_workspaces` — read now, so a workspace
/// created later has it. A name clash (`mcp__<name>__<tool>` keys servers by
/// name) goes to the workspace's own server, then to the oldest shared one;
/// the others are not available there.
pub fn available_in(ws: &str) -> String {
    let shared = |t: &str| {
        format!(
            "({t}.managed = 1 AND {ws} <> '{SCRATCH_WORKSPACE_ID}' \
             AND {ws} IN (SELECT id FROM workspaces WHERE archived = 0) \
             AND ({t}.all_workspaces = 1 OR EXISTS (SELECT 1 FROM mcp_server_workspaces l \
             WHERE l.server_id = {t}.id AND l.workspace_id = {ws})))"
        )
    };
    format!(
        "((s.workspace_id = {ws} OR {}) AND NOT EXISTS (SELECT 1 FROM mcp_servers o \
         WHERE o.name = s.name AND o.id <> s.id AND s.workspace_id <> {ws} \
         AND (o.workspace_id = {ws} OR ({} AND (o.created_at, o.id) < (s.created_at, s.id)))))",
        shared("s"),
        shared("o")
    )
}

impl McpRegistryRepo {
    /// Whether workspace `ws` reaches server `id` ([`available_in`]).
    pub async fn is_available(&self, id: &Id, ws: &str) -> Result<bool> {
        let sql = format!(
            "SELECT EXISTS (SELECT 1 FROM mcp_servers s WHERE s.id = ?1 AND {})",
            available_in("?2")
        );
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .bind(id)
            .bind(ws)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("mcp server scope"))
    }

    /// The workspaces OTHER than its home through which `user_id` (by
    /// membership) reaches server `id`: where they may use a shared server.
    pub async fn reach_workspaces(&self, id: &Id, user_id: &Id) -> Result<Vec<Id>> {
        let sql = format!(
            "SELECT m.workspace_id FROM workspace_members m JOIN mcp_servers s ON s.id = ?1 \
             WHERE m.user_id = ?2 AND m.workspace_id <> s.workspace_id AND {}",
            available_in("m.workspace_id")
        );
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .bind(id)
            .bind(user_id)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("mcp server scope"))
    }

    /// Refuse a scope [`Self::set_scope`] would refuse, without writing:
    /// every workspace must exist and none may be Scratch.
    pub async fn check_scope(&self, shared: &[Id]) -> Result<()> {
        for ws in shared {
            let known: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM workspaces WHERE id = ?)")
                    .bind(ws)
                    .fetch_one(&self.pool)
                    .await
                    .map_err(dberr("mcp scope"))?;
            if !known || ws == SCRATCH_WORKSPACE_ID {
                return Err(Error::Invalid(format!("unknown workspace {ws}")));
            }
        }
        Ok(())
    }

    /// Replace where `id` is available: `all` workspaces (the chosen ones
    /// are cleared), else its home plus `shared` (the home itself is implied,
    /// so it is skipped). One transaction: a reader never sees half a scope.
    /// An unknown workspace or Scratch is a 400 and nothing changes. (A link
    /// to an archived workspace is kept: it counts again once restored.)
    pub async fn set_scope(&self, id: &Id, all: bool, shared: &[Id]) -> Result<McpServerDetail> {
        let home = self.get(id).await?.workspace_id;
        let mut shared: Vec<&Id> = shared.iter().filter(|w| !all && **w != home).collect();
        shared.sort();
        shared.dedup();
        let shared: Vec<Id> = shared.into_iter().cloned().collect();
        self.check_scope(&shared).await?;
        let mut tx = self.pool.begin().await.map_err(dberr("mcp scope"))?;
        sqlx::query("UPDATE mcp_servers SET all_workspaces = ?, updated_at = ? WHERE id = ?")
            .bind(i64::from(all))
            .bind(fmt(chrono::Utc::now()))
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("mcp scope"))?;
        sqlx::query("DELETE FROM mcp_server_workspaces WHERE server_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("mcp scope"))?;
        for ws in &shared {
            sqlx::query(
                "INSERT INTO mcp_server_workspaces (server_id, workspace_id) VALUES (?, ?)",
            )
            .bind(id)
            .bind(ws)
            .execute(&mut *tx)
            .await
            .map_err(dberr("mcp scope"))?;
        }
        tx.commit().await.map_err(dberr("mcp scope"))?;
        self.get(id).await
    }
}

impl McpRegistryRepo {
    /// Whether `user` may open server `id`'s pages for `operation`: a member
    /// of its home, or — to USE it, not to configure it or manage its access —
    /// of a workspace it is shared with. A missing server is NotFound.
    pub async fn page_member(
        &self,
        id: &Id,
        user: &otto_core::domain::User,
        operation: &str,
    ) -> Result<bool> {
        let home = self.get(id).await?.workspace_id;
        if crate::WorkspacesRepo::new(self.pool.clone())
            .role_of(user, &home)
            .await?
            .is_some()
        {
            return Ok(true);
        }
        Ok(!matches!(operation, "configure" | "manage_access")
            && !self.reach_workspaces(id, &user.id).await?.is_empty())
    }
}

impl McpServerDetail {
    /// Shared beyond its home (all workspaces or chosen ones).
    pub fn is_shared(&self) -> bool {
        self.all_workspaces || !self.shared_workspace_ids.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{McpServersRepo, NewServerRow};

    async fn ws(pool: &crate::DbPool, name: &str) -> Id {
        let id = otto_core::new_id();
        add_ws(pool, &id, name).await;
        id
    }

    async fn add_ws(pool: &crate::DbPool, id: &str, name: &str) {
        sqlx::query("INSERT OR IGNORE INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, '/tmp', ?)")
            .bind(id)
            .bind(name)
            .bind(fmt(chrono::Utc::now()))
            .execute(pool)
            .await
            .unwrap();
    }

    async fn member(pool: &crate::DbPool, ws: &str, user: &str) {
        sqlx::query(
            "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES (?, ?, 'editor')",
        )
        .bind(ws)
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn setup() -> (crate::DbPool, McpRegistryRepo, Id, Id) {
        let pool = crate::db::test_pool().await;
        let now = fmt(chrono::Utc::now());
        sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES ('u', 'u', 'x', 'U', 0, ?)")
            .bind(&now).execute(&pool).await.unwrap();
        let (a, b) = (ws(&pool, "a").await, ws(&pool, "b").await);
        (pool.clone(), McpRegistryRepo::new(pool), a, b)
    }

    async fn server(repo: &McpRegistryRepo, home: &Id, name: &str) -> McpServerDetail {
        repo.create(NewServerRow {
            workspace_id: home.clone(),
            name: name.into(),
            transport: "stdio".into(),
            command: "echo".into(),
            args: vec![],
            env: BTreeMap::new(),
            url: None,
            description: None,
            headers: BTreeMap::new(),
            secret_ref: None,
            secret_env_keys: vec![],
            secret_header_keys: vec![],
            injection_risk: "low".into(),
            default_tool_access: "allow".into(),
            enabled: true,
            created_by: "u".into(),
        })
        .await
        .unwrap()
    }

    async fn names(repo: &McpRegistryRepo, ws: &Id) -> Vec<(String, Id)> {
        let list = repo.list_for_ws(ws).await.unwrap();
        list.into_iter().map(|s| (s.name, s.workspace_id)).collect()
    }

    /// Home only (every existing server) → shared with B → all, which a
    /// workspace created AFTERWARDS has too → back to "B": gone from C.
    #[tokio::test]
    async fn home_shared_all_and_a_later_workspace() {
        let (pool, repo, a, b) = setup().await;
        let s = server(&repo, &a, "gh").await;
        assert!(!s.is_shared());
        assert_eq!(names(&repo, &a).await.len(), 1);
        assert!(names(&repo, &b).await.is_empty());

        let s = repo
            .set_scope(&s.id, false, &[b.clone(), a.clone(), b.clone()])
            .await
            .unwrap();
        assert_eq!(
            s.shared_workspace_ids,
            vec![b.clone()],
            "home skipped, deduped"
        );
        assert_eq!(names(&repo, &b).await, vec![("gh".to_string(), a.clone())]);
        assert!(repo.is_available(&s.id, &b).await.unwrap());

        // "All" clears the chosen links: turning it off later can't revive them.
        let s = repo
            .set_scope(&s.id, true, std::slice::from_ref(&b))
            .await
            .unwrap();
        assert!(s.shared_workspace_ids.is_empty());
        let c = ws(&pool, "c").await;
        assert_eq!(
            names(&repo, &c).await.len(),
            1,
            "a workspace added later has it"
        );
        member(&pool, &c, "u").await;
        assert_eq!(
            repo.reach_workspaces(&s.id, &"u".into()).await.unwrap(),
            vec![c.clone()]
        );
        let s = repo.set_scope(&s.id, false, &[]).await.unwrap();
        assert!(!s.is_shared() && names(&repo, &b).await.is_empty());

        repo.set_scope(&s.id, false, std::slice::from_ref(&b))
            .await
            .unwrap();
        assert!(names(&repo, &c).await.is_empty());
        assert!(!repo.is_available(&s.id, &c).await.unwrap());
        assert!(repo
            .reach_workspaces(&s.id, &"u".into())
            .await
            .unwrap()
            .is_empty());
        // The native-config CRUD surface stays the home's.
        let legacy = McpServersRepo::new(pool.clone());
        assert!(legacy.list_for_ws(&b).await.unwrap().is_empty());
    }

    /// Only a governed server is shared: a native one is written into agent
    /// config files, so even a stored scope never takes it out of its home.
    #[tokio::test]
    async fn a_native_server_is_never_shared() {
        let (pool, repo, a, b) = setup().await;
        let s = server(&repo, &a, "gh").await;
        repo.set_scope(&s.id, true, &[]).await.unwrap();
        repo.set_managed(&s.id, false).await.unwrap();
        sqlx::query("DELETE FROM resource_access_policies WHERE resource_id = ?")
            .bind(&s.id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(names(&repo, &b).await.is_empty());
        assert!(!repo.is_available(&s.id, &b).await.unwrap());
        assert!(repo.list_native_enabled(&b).await.unwrap().is_empty());
        assert_eq!(repo.list_native_enabled(&a).await.unwrap().len(), 1);
    }

    /// Scratch (every user's implicit workspace) is never part of "all" and
    /// can't be picked; an archived workspace is not reached until restored.
    #[tokio::test]
    async fn scratch_and_archived_workspaces_are_not_reached() {
        let (pool, repo, a, b) = setup().await;
        add_ws(&pool, SCRATCH_WORKSPACE_ID, "Scratch").await;
        let s = server(&repo, &a, "gh").await;
        let err = repo
            .set_scope(&s.id, false, &[SCRATCH_WORKSPACE_ID.to_string()])
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{err:?}");
        repo.set_scope(&s.id, true, &[]).await.unwrap();
        assert!(names(&repo, &SCRATCH_WORKSPACE_ID.to_string())
            .await
            .is_empty());
        assert!(!repo
            .is_available(&s.id, SCRATCH_WORKSPACE_ID)
            .await
            .unwrap());
        let archive = |v: i64| {
            let (pool, b) = (pool.clone(), b.clone());
            async move {
                sqlx::query("UPDATE workspaces SET archived = ? WHERE id = ?")
                    .bind(v)
                    .bind(b)
                    .execute(&pool)
                    .await
                    .unwrap();
            }
        };
        archive(1).await;
        assert!(!repo.is_available(&s.id, &b).await.unwrap());
        archive(0).await;
        assert!(repo.is_available(&s.id, &b).await.unwrap());
    }

    /// An unknown workspace refuses the whole scope change.
    #[tokio::test]
    async fn unknown_workspace_changes_nothing() {
        let (_pool, repo, a, b) = setup().await;
        let s = server(&repo, &a, "gh").await;
        repo.set_scope(&s.id, false, std::slice::from_ref(&b))
            .await
            .unwrap();
        let err = repo
            .set_scope(&s.id, false, &["nope".into()])
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{err:?}");
        let s = repo.get(&s.id).await.unwrap();
        assert!(!s.all_workspaces);
        assert_eq!(s.shared_workspace_ids, vec![b]);
    }

    /// A workspace's own server wins a name clash; among shared ones the
    /// oldest — one `mcp__<name>__*` tool prefix.
    #[tokio::test]
    async fn own_server_wins_a_name_clash() {
        let (pool, repo, a, b) = setup().await;
        let shared = server(&repo, &a, "gh").await;
        repo.set_scope(&shared.id, true, &[]).await.unwrap();
        let c = ws(&pool, "c").await;
        let late = server(&repo, &c, "gh").await;
        repo.set_scope(&late.id, true, &[]).await.unwrap();
        assert_eq!(
            names(&repo, &b).await,
            vec![("gh".to_string(), a.clone())],
            "oldest shared"
        );
        assert!(!repo.is_available(&late.id, &b).await.unwrap());
        let own = server(&repo, &b, "gh").await;
        assert_eq!(
            names(&repo, &b).await,
            vec![("gh".to_string(), b.clone())],
            "own wins"
        );
        assert_eq!(names(&repo, &a).await, vec![("gh".to_string(), a.clone())]);
        assert!(!repo.is_available(&shared.id, &b).await.unwrap());
        assert!(repo.is_available(&own.id, &b).await.unwrap());
    }

    /// Deleting a workspace drops its links; deleting the HOME of a server
    /// other workspaces use is refused (it would cascade the server away)
    /// until the server is home-only again.
    #[tokio::test]
    async fn workspace_delete_rules() {
        let (pool, repo, a, b) = setup().await;
        let s = server(&repo, &a, "gh").await;
        repo.set_scope(&s.id, false, std::slice::from_ref(&b))
            .await
            .unwrap();
        let del = |id: Id| {
            let pool = pool.clone();
            async move {
                sqlx::query("DELETE FROM workspaces WHERE id = ?")
                    .bind(id)
                    .execute(&pool)
                    .await
            }
        };
        let err = del(a.clone()).await.unwrap_err().to_string();
        assert!(err.contains("shared with other workspaces"), "{err}");
        assert!(repo.get(&s.id).await.is_ok());
        del(b.clone()).await.unwrap();
        assert!(repo
            .get(&s.id)
            .await
            .unwrap()
            .shared_workspace_ids
            .is_empty());
        repo.set_scope(&s.id, true, &[]).await.unwrap();
        assert!(
            del(a.clone()).await.is_err(),
            "all workspaces counts as shared"
        );
        repo.set_scope(&s.id, false, &[]).await.unwrap();
        del(a).await.unwrap();
        assert!(repo.get(&s.id).await.is_err());
    }
}
