//! Where an MCP server is available (migration 0232): its home workspace,
//! the workspaces it was shared with, or every workspace. One server row
//! stays the one place to edit it — one secret blob, one OAuth token set, one
//! health state. [`available_in`] is the ONE predicate behind every "which
//! MCP servers does workspace W have" read: the registry list and gateway
//! tools (`McpRegistryRepo::list_for_ws`), the native `.mcp.json` set
//! (`list_native_enabled`) and the reverse
//! [`McpRegistryRepo::workspaces_with`]. The native-config CRUD surface
//! (`McpServersRepo`, `/workspaces/{id}/mcp-servers`) is deliberately the
//! home's only: a server's settings live in its home workspace.

use otto_core::{Error, Id, Result};

use crate::convert::{dberr, fmt};
use crate::{McpRegistryRepo, McpServerDetail};

/// SQL: is the `mcp_servers s` row available in workspace `ws` (an SQL
/// expression such as `?1` or `w.id`)? Its home, a link, or
/// `all_workspaces` — read now, so a workspace created later has it. A name
/// clash (`.mcp.json` and the gateway's `mcp__<name>__<tool>` key servers by
/// name) goes to the workspace's own server, then to the oldest shared one;
/// the others are not available there.
pub fn available_in(ws: &str) -> String {
    let linked = |t: &str| {
        format!(
            "({t}.all_workspaces = 1 OR EXISTS (SELECT 1 FROM mcp_server_workspaces l \
             WHERE l.server_id = {t}.id AND l.workspace_id = {ws}))"
        )
    };
    format!(
        "((s.workspace_id = {ws} OR {}) AND NOT EXISTS (SELECT 1 FROM mcp_servers o \
         WHERE o.name = s.name AND o.id <> s.id AND s.workspace_id <> {ws} \
         AND (o.workspace_id = {ws} OR ({} AND (o.created_at, o.id) < (s.created_at, s.id)))))",
        linked("s"),
        linked("o")
    )
}

impl McpServerDetail {
    /// Whether a workspace reaches this server: home, shared or all. (A
    /// server shadowed by a same-named one there is still reachable by id —
    /// it IS shared with that workspace — just not listed.)
    pub fn is_available_in(&self, ws: &str) -> bool {
        self.workspace_id == ws
            || self.all_workspaces
            || self.shared_workspace_ids.iter().any(|w| w == ws)
    }
}

impl McpRegistryRepo {
    /// Replace where `id` is available: `all` workspaces, else its home plus
    /// `shared` (the home itself is implied, so it is skipped). One
    /// transaction: a reader never sees half a scope. An unknown workspace is
    /// a 400 and nothing changes.
    pub async fn set_scope(&self, id: &Id, all: bool, shared: &[Id]) -> Result<McpServerDetail> {
        let home = self.get(id).await?.workspace_id;
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
        let mut shared: Vec<&Id> = shared.iter().filter(|w| **w != home).collect();
        shared.sort();
        shared.dedup();
        for ws in shared {
            let added = sqlx::query(
                "INSERT INTO mcp_server_workspaces (server_id, workspace_id)
                 SELECT ?, id FROM workspaces WHERE id = ?",
            )
            .bind(id)
            .bind(ws)
            .execute(&mut *tx)
            .await
            .map_err(dberr("mcp scope"))?;
            if added.rows_affected() == 0 {
                return Err(Error::Invalid(format!("unknown workspace {ws}")));
            }
        }
        tx.commit().await.map_err(dberr("mcp scope"))?;
        self.get(id).await
    }

    /// The workspaces server `id` is available in (the reverse of
    /// [`available_in`]), oldest first: where its native launcher entries
    /// may live.
    pub async fn workspaces_with(&self, id: &Id) -> Result<Vec<Id>> {
        let sql = format!(
            "SELECT w.id FROM workspaces w JOIN mcp_servers s ON s.id = ? WHERE {} ORDER BY w.created_at",
            available_in("w.id")
        );
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .bind(id)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("mcp server workspaces"))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{McpServersRepo, NewServerRow};

    async fn ws(pool: &crate::DbPool, name: &str) -> Id {
        let id = otto_core::new_id();
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, '/tmp', ?)",
        )
        .bind(&id)
        .bind(name)
        .bind(fmt(chrono::Utc::now()))
        .execute(pool)
        .await
        .unwrap();
        id
    }

    async fn setup() -> (crate::DbPool, McpRegistryRepo, Id, Id, Id) {
        let pool = crate::db::test_pool().await;
        let now = fmt(chrono::Utc::now());
        sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES ('u', 'u', 'x', 'U', 0, ?)")
            .bind(&now).execute(&pool).await.unwrap();
        let (a, b) = (ws(&pool, "a").await, ws(&pool, "b").await);
        (pool.clone(), McpRegistryRepo::new(pool), a, b, "u".into())
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
    /// workspace created AFTERWARDS has too → back to "A, B": gone from C.
    /// The native `.mcp.json` set follows the same predicate.
    #[tokio::test]
    async fn home_shared_all_and_a_later_workspace() {
        let (pool, repo, a, b, _) = setup().await;
        let s = server(&repo, &a, "gh").await;
        assert!(!s.all_workspaces && s.shared_workspace_ids.is_empty());
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
        assert!(s.is_available_in(&b));

        repo.set_scope(&s.id, true, &[]).await.unwrap();
        let c = ws(&pool, "c").await;
        assert_eq!(
            names(&repo, &c).await.len(),
            1,
            "a workspace added later has it"
        );
        // Native set + the legacy workspace route read the same predicate.
        repo.set_managed(&s.id, false).await.unwrap();
        sqlx::query("DELETE FROM resource_access_policies WHERE resource_id = ?")
            .bind(&s.id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(repo.list_native_enabled(&c).await.unwrap().len(), 1);
        // The native-config CRUD surface stays the home's (settings live there).
        let legacy = McpServersRepo::new(pool.clone());
        assert!(legacy.list_for_ws(&c).await.unwrap().is_empty());
        assert_eq!(legacy.list_for_ws(&a).await.unwrap().len(), 1);
        assert_eq!(
            repo.workspaces_with(&s.id).await.unwrap(),
            vec![a.clone(), b.clone(), c.clone()]
        );

        let s = repo
            .set_scope(&s.id, false, std::slice::from_ref(&b))
            .await
            .unwrap();
        assert!(names(&repo, &c).await.is_empty());
        assert!(repo.list_native_enabled(&c).await.unwrap().is_empty());
        assert!(!s.is_available_in(&c));
        assert_eq!(repo.workspaces_with(&s.id).await.unwrap(), vec![a, b]);
    }

    /// An unknown workspace refuses the whole scope change.
    #[tokio::test]
    async fn unknown_workspace_changes_nothing() {
        let (_pool, repo, a, b, _) = setup().await;
        let s = server(&repo, &a, "gh").await;
        repo.set_scope(&s.id, false, std::slice::from_ref(&b))
            .await
            .unwrap();
        let err = repo
            .set_scope(&s.id, true, &["nope".into()])
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{err:?}");
        let s = repo.get(&s.id).await.unwrap();
        assert!(!s.all_workspaces);
        assert_eq!(s.shared_workspace_ids, vec![b]);
    }

    /// A workspace's own server wins a name clash; among shared ones the
    /// oldest — one `.mcp.json` key, one `mcp__<name>__*` tool prefix.
    #[tokio::test]
    async fn own_server_wins_a_name_clash() {
        let (pool, repo, a, b, _) = setup().await;
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
        let own = server(&repo, &b, "gh").await;
        assert_eq!(
            names(&repo, &b).await,
            vec![("gh".to_string(), b.clone())],
            "own wins"
        );
        assert_eq!(names(&repo, &a).await, vec![("gh".to_string(), a.clone())]);
        assert!(!repo.workspaces_with(&shared.id).await.unwrap().contains(&b));
        assert!(repo.workspaces_with(&own.id).await.unwrap().contains(&b));
    }

    /// Deleting a workspace drops its links; deleting the HOME of a server
    /// other workspaces use is refused (it would cascade the server away)
    /// until the server is home-only again.
    #[tokio::test]
    async fn workspace_delete_rules() {
        let (pool, repo, a, b, _) = setup().await;
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
