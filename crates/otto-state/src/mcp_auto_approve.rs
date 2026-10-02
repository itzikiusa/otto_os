//! MCP auto-approve rules (`mcp_auto_approve_rules`, migration 0148): the
//! explicit, opt-in policy under which a MUTATING `otto.*` tool call skips the
//! per-call human approval. Pure persistence — validation (known mutating tool
//! / category, the irreversible guardrail, scope ids) and the match itself live
//! in the daemon (`otto_server::mcp_auto_approve`, `otto_mcp::auto_approve`).

use chrono::Utc;
use otto_core::{new_id, Id, Result};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::convert::{dberr, dberr_unique, fmt};
use crate::DbPool;

/// One auto-approve rule (wire + domain shape; mirrored in `ui/src/lib/api/types.ts`
/// as `McpAutoApproveRule`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpAutoApproveRule {
    pub id: Id,
    pub name: String,
    pub enabled: bool,
    /// `global` | `workspace` | `session`.
    pub scope: String,
    /// Set iff `scope == "workspace"`.
    pub workspace_id: Option<Id>,
    /// Set iff `scope == "session"` (the agent session the rule is pinned to).
    pub session_id: Option<Id>,
    /// `tool` | `category`.
    pub target_kind: String,
    /// The bare tool name (`create_pr`) or the catalog category label (`Git`).
    pub target: String,
    /// The second explicit toggle: a per-tool rule only covers an irreversible
    /// tool when this is true. Always false on a category rule.
    pub allow_irreversible: bool,
    pub note: Option<String>,
    pub created_by: Id,
    pub created_at: String,
    pub updated_at: String,
}

/// Input for [`McpAutoApproveRepo::create`] (already validated by the caller).
#[derive(Debug, Clone)]
pub struct NewAutoApproveRule {
    pub name: String,
    pub enabled: bool,
    pub scope: String,
    pub workspace_id: Option<Id>,
    pub session_id: Option<Id>,
    pub target_kind: String,
    pub target: String,
    pub allow_irreversible: bool,
    pub note: Option<String>,
    pub created_by: Id,
}

/// A partial update. `None` leaves a field unchanged.
#[derive(Debug, Clone, Default)]
pub struct AutoApproveRulePatch {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub allow_irreversible: Option<bool>,
    pub note: Option<String>,
}

fn row_to_rule(r: &sqlx::sqlite::SqliteRow) -> McpAutoApproveRule {
    McpAutoApproveRule {
        id: r.get("id"),
        name: r.get("name"),
        enabled: r.get::<i64, _>("enabled") != 0,
        scope: r.get("scope"),
        workspace_id: r.get("workspace_id"),
        session_id: r.get("session_id"),
        target_kind: r.get("target_kind"),
        target: r.get("target"),
        allow_irreversible: r.get::<i64, _>("allow_irreversible") != 0,
        note: r.get("note"),
        created_by: r.get("created_by"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

#[derive(Clone)]
pub struct McpAutoApproveRepo {
    pool: DbPool,
}

impl McpAutoApproveRepo {
    pub fn new(pool: impl Into<DbPool>) -> Self {
        Self { pool: pool.into() }
    }

    /// Every rule, global first, then workspace, then session; oldest first
    /// within a scope (the display order of the control-plane list).
    pub async fn list(&self) -> Result<Vec<McpAutoApproveRule>> {
        let rows = sqlx::query(
            "SELECT * FROM mcp_auto_approve_rules
             ORDER BY CASE scope WHEN 'global' THEN 0 WHEN 'workspace' THEN 1 ELSE 2 END,
                      created_at, id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("auto-approve rules"))?;
        Ok(rows.iter().map(row_to_rule).collect())
    }

    /// The enabled rules that could apply to one call: global ones plus those
    /// pinned to `workspace_id` / `session_id` (when given). The daemon picks
    /// the most specific match among them.
    pub async fn list_applicable(
        &self,
        workspace_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<Vec<McpAutoApproveRule>> {
        let rows = sqlx::query(
            "SELECT * FROM mcp_auto_approve_rules
             WHERE enabled = 1
               AND (scope = 'global'
                    OR (scope = 'workspace' AND workspace_id = ?)
                    OR (scope = 'session' AND session_id = ?))
             ORDER BY created_at, id",
        )
        .bind(workspace_id.unwrap_or(""))
        .bind(session_id.unwrap_or(""))
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("auto-approve rules"))?;
        Ok(rows.iter().map(row_to_rule).collect())
    }

    pub async fn get(&self, id: &str) -> Result<McpAutoApproveRule> {
        let r = sqlx::query("SELECT * FROM mcp_auto_approve_rules WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("auto-approve rule"))?;
        Ok(row_to_rule(&r))
    }

    pub async fn create(&self, n: NewAutoApproveRule) -> Result<McpAutoApproveRule> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO mcp_auto_approve_rules
               (id, name, enabled, scope, workspace_id, session_id, target_kind, target,
                allow_irreversible, note, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&n.name)
        .bind(n.enabled as i64)
        .bind(&n.scope)
        .bind(&n.workspace_id)
        .bind(&n.session_id)
        .bind(&n.target_kind)
        .bind(&n.target)
        .bind(n.allow_irreversible as i64)
        .bind(&n.note)
        .bind(&n.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr_unique(
            "create auto-approve rule",
            "an auto-approve rule for this target already exists in that scope",
        ))?;
        self.get(&id).await
    }

    pub async fn update(&self, id: &str, p: &AutoApproveRulePatch) -> Result<McpAutoApproveRule> {
        let cur = self.get(id).await?;
        sqlx::query(
            "UPDATE mcp_auto_approve_rules
             SET name = ?, enabled = ?, allow_irreversible = ?, note = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(p.name.as_ref().unwrap_or(&cur.name))
        .bind(p.enabled.unwrap_or(cur.enabled) as i64)
        .bind(p.allow_irreversible.unwrap_or(cur.allow_irreversible) as i64)
        .bind(p.note.as_ref().or(cur.note.as_ref()))
        .bind(fmt(Utc::now()))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update auto-approve rule"))?;
        self.get(id).await
    }

    /// Delete one rule. `NotFound` when it does not exist.
    pub async fn delete(&self, id: &str) -> Result<()> {
        let res = sqlx::query("DELETE FROM mcp_auto_approve_rules WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete auto-approve rule"))?;
        if res.rows_affected() == 0 {
            return Err(otto_core::Error::NotFound("auto-approve rule".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool.into()
    }

    async fn seed(pool: &DbPool) -> (Id, Id) {
        let user = new_id();
        let ws = new_id();
        let now = fmt(Utc::now());
        sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES (?, 'u', 'x', 'U', 1, ?)")
            .bind(&user).bind(&now).execute(pool).await.unwrap();
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, 'w', '/tmp', ?)",
        )
        .bind(&ws)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        (ws, user)
    }

    fn new_rule(scope: &str, ws: Option<&Id>, target: &str, user: &Id) -> NewAutoApproveRule {
        NewAutoApproveRule {
            name: format!("{scope} {target}"),
            enabled: true,
            scope: scope.into(),
            workspace_id: ws.cloned(),
            session_id: None,
            target_kind: "tool".into(),
            target: target.into(),
            allow_irreversible: false,
            note: None,
            created_by: user.clone(),
        }
    }

    #[tokio::test]
    async fn crud_and_applicable_scoping() {
        let pool = mem_pool().await;
        let (ws, user) = seed(&pool).await;
        let repo = McpAutoApproveRepo::new(pool.clone());
        let g = repo
            .create(new_rule("global", None, "create_pr", &user))
            .await
            .unwrap();
        let w = repo
            .create(new_rule("workspace", Some(&ws), "comment_pr", &user))
            .await
            .unwrap();
        assert_eq!(repo.list().await.unwrap().len(), 2);

        // Another workspace sees only the global rule.
        let other = repo.list_applicable(Some("elsewhere"), None).await.unwrap();
        assert_eq!(
            other.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
            vec![g.id.clone()]
        );
        let here = repo.list_applicable(Some(&ws), None).await.unwrap();
        assert_eq!(here.len(), 2);

        // Disabled rules never apply.
        let w2 = repo
            .update(
                &w.id,
                &AutoApproveRulePatch {
                    enabled: Some(false),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(!w2.enabled);
        assert_eq!(
            repo.list_applicable(Some(&ws), None).await.unwrap().len(),
            1
        );

        // Duplicate (same scope + place + target) → Conflict.
        let dup = repo
            .create(new_rule("global", None, "create_pr", &user))
            .await;
        assert!(matches!(dup, Err(otto_core::Error::Conflict(_))));

        repo.delete(&g.id).await.unwrap();
        assert!(matches!(
            repo.delete(&g.id).await,
            Err(otto_core::Error::NotFound(_))
        ));
    }

    /// The 0148 carry-over of the legacy `mcp_approval_exempt_tools` setting:
    /// re-run its INSERT against a seeded setting (the migration itself already
    /// ran on an empty DB) — each tool becomes one global per-tool rule, an
    /// `otto.` prefix is stripped, duplicates and junk are skipped, and the
    /// explicit legacy choice keeps covering an irreversible tool.
    #[tokio::test]
    async fn migration_imports_the_legacy_exempt_list() {
        let pool = mem_pool().await;
        let (_ws, _user) = seed(&pool).await;
        sqlx::query(
            "INSERT INTO settings (key, value_json) VALUES ('mcp_approval_exempt_tools', ?)",
        )
        .bind(r#"["comment_pr","otto.merge_pr","comment_pr",42,""]"#)
        .execute(&pool)
        .await
        .unwrap();
        let sql = include_str!("../migrations/0148_mcp_auto_approve_rules.sql");
        let insert = &sql[sql.find("INSERT OR IGNORE").expect("import block")..];
        sqlx::query(insert).execute(&pool).await.unwrap();
        // Idempotent: a second run inserts nothing new (unique index + OR IGNORE).
        sqlx::query(insert).execute(&pool).await.unwrap();
        let rules = McpAutoApproveRepo::new(pool.clone()).list().await.unwrap();
        let mut targets: Vec<(String, String, bool)> = rules
            .iter()
            .map(|r| (r.scope.clone(), r.target.clone(), r.allow_irreversible))
            .collect();
        targets.sort();
        assert_eq!(
            targets,
            vec![
                ("global".into(), "comment_pr".into(), true),
                ("global".into(), "merge_pr".into(), true),
            ]
        );
        assert!(rules.iter().all(|r| r.target_kind == "tool" && r.enabled));
    }

    #[tokio::test]
    async fn workspace_delete_cascades_its_rules() {
        let pool = mem_pool().await;
        let (ws, user) = seed(&pool).await;
        let repo = McpAutoApproveRepo::new(pool.clone());
        repo.create(new_rule("workspace", Some(&ws), "create_pr", &user))
            .await
            .unwrap();
        sqlx::query("DELETE FROM workspaces WHERE id = ?")
            .bind(&ws)
            .execute(&pool)
            .await
            .unwrap();
        assert!(repo.list().await.unwrap().is_empty());
    }
}
