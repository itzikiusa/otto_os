//! MCP profiles (`mcp_profiles`, migration 0191): named tool sets an agent
//! session or an external connection runs the Otto MCP server under. Pure
//! persistence plus the two invariants the table carries — exactly one
//! default, and the locked built-in (`full`) is never edited or deleted. Group
//! membership, validation and the filter itself live in `otto_mcp::profiles`.

use chrono::Utc;
use otto_core::{new_id, Error, Id, Result};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::convert::{dberr, dberr_unique, fmt};
use crate::DbPool;

/// The locked built-in every-tool profile (the initial default).
pub const FULL_PROFILE_ID: &str = "full";

/// One profile (wire + domain shape; mirrored in
/// `ui/src/lib/api/types/mcp.ts` as `McpProfile`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpProfile {
    pub id: Id,
    pub name: String,
    pub description: String,
    /// Every tool, minus `exclude_tools` (the Full profile).
    pub all_tools: bool,
    /// Tool-group ids (`otto_mcp::profiles::GROUPS`).
    pub groups: Vec<String>,
    /// Canonical tool names added on top of the groups.
    pub include_tools: Vec<String>,
    /// Canonical tool names removed from the groups.
    pub exclude_tools: Vec<String>,
    /// A seeded preset.
    pub builtin: bool,
    /// Read-only and undeletable (Full).
    pub locked: bool,
    pub is_default: bool,
    pub created_by: Option<Id>,
    pub created_at: String,
    pub updated_at: String,
}

/// The editable part of a profile (already validated by the caller).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpProfileFields {
    pub name: String,
    pub description: String,
    pub all_tools: bool,
    pub groups: Vec<String>,
    pub include_tools: Vec<String>,
    pub exclude_tools: Vec<String>,
}

fn list_of(r: &sqlx::sqlite::SqliteRow, col: &str) -> Vec<String> {
    serde_json::from_str(&r.get::<String, _>(col)).unwrap_or_default()
}

fn row_to_profile(r: &sqlx::sqlite::SqliteRow) -> McpProfile {
    McpProfile {
        id: r.get("id"),
        name: r.get("name"),
        description: r.get("description"),
        all_tools: r.get::<i64, _>("all_tools") != 0,
        groups: list_of(r, "groups_json"),
        include_tools: list_of(r, "include_tools_json"),
        exclude_tools: list_of(r, "exclude_tools_json"),
        builtin: r.get::<i64, _>("builtin") != 0,
        locked: r.get::<i64, _>("locked") != 0,
        is_default: r.get::<i64, _>("is_default") != 0,
        created_by: r.get("created_by"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

fn json_list(v: &[String]) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "[]".into())
}

const DUPLICATE_NAME: &str = "an MCP profile with this name already exists";

#[derive(Clone)]
pub struct McpProfileRepo {
    pool: DbPool,
}

impl McpProfileRepo {
    pub fn new(pool: impl Into<DbPool>) -> Self {
        Self { pool: pool.into() }
    }

    /// Every profile: built-ins first (Full leading), then the user's own by
    /// name.
    pub async fn list(&self) -> Result<Vec<McpProfile>> {
        let rows = sqlx::query(
            "SELECT * FROM mcp_profiles
             ORDER BY locked DESC, builtin DESC, name COLLATE NOCASE, id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("mcp profiles"))?;
        Ok(rows.iter().map(row_to_profile).collect())
    }

    pub async fn get(&self, id: &str) -> Result<McpProfile> {
        let r = sqlx::query("SELECT * FROM mcp_profiles WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("mcp profile"))?;
        Ok(row_to_profile(&r))
    }

    /// The default profile. Falls back to Full if the row is somehow missing
    /// (the index allows zero defaults; the repo never leaves it so).
    pub async fn default_profile(&self) -> Result<McpProfile> {
        let r = sqlx::query(
            "SELECT * FROM mcp_profiles WHERE is_default = 1 OR id = ?
             ORDER BY is_default DESC LIMIT 1",
        )
        .bind(FULL_PROFILE_ID)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("default mcp profile"))?;
        Ok(row_to_profile(&r))
    }

    /// The profiles named by `ids` that exist, in the given order (unknown
    /// ids — e.g. a deleted profile a session still names — are skipped).
    /// `"default"` names the current default. Nothing left ⇒ the default.
    pub async fn resolve(&self, ids: &[String]) -> Result<Vec<McpProfile>> {
        let mut out: Vec<McpProfile> = Vec::new();
        for id in ids {
            let found = if id == "default" {
                Some(self.default_profile().await?)
            } else {
                match self.get(id).await {
                    Ok(p) => Some(p),
                    Err(Error::NotFound(_)) => None,
                    Err(e) => return Err(e),
                }
            };
            if let Some(p) = found {
                if !out.iter().any(|o| o.id == p.id) {
                    out.push(p);
                }
            }
        }
        if out.is_empty() {
            out.push(self.default_profile().await?);
        }
        Ok(out)
    }

    pub async fn create(
        &self,
        f: &McpProfileFields,
        created_by: Option<&str>,
    ) -> Result<McpProfile> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO mcp_profiles
               (id, name, description, all_tools, groups_json, include_tools_json,
                exclude_tools_json, builtin, locked, is_default, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, 0, 0, 0, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&f.name)
        .bind(&f.description)
        .bind(f.all_tools as i64)
        .bind(json_list(&f.groups))
        .bind(json_list(&f.include_tools))
        .bind(json_list(&f.exclude_tools))
        .bind(created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr_unique("create mcp profile", DUPLICATE_NAME))?;
        self.get(&id).await
    }

    /// Replace a profile's editable fields. The locked profile is refused.
    pub async fn update(&self, id: &str, f: &McpProfileFields) -> Result<McpProfile> {
        let cur = self.get(id).await?;
        if cur.locked {
            return Err(Error::Invalid(format!(
                "the {} profile is built in and can't be edited — duplicate it instead",
                cur.name
            )));
        }
        sqlx::query(
            "UPDATE mcp_profiles
             SET name = ?, description = ?, all_tools = ?, groups_json = ?,
                 include_tools_json = ?, exclude_tools_json = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&f.name)
        .bind(&f.description)
        .bind(f.all_tools as i64)
        .bind(json_list(&f.groups))
        .bind(json_list(&f.include_tools))
        .bind(json_list(&f.exclude_tools))
        .bind(fmt(Utc::now()))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr_unique("update mcp profile", DUPLICATE_NAME))?;
        self.get(id).await
    }

    /// Delete a profile. The locked profile is refused; deleting the default
    /// hands the default back to Full in the same transaction, so there is
    /// always exactly one.
    pub async fn delete(&self, id: &str) -> Result<McpProfile> {
        let cur = self.get(id).await?;
        if cur.locked {
            return Err(Error::Invalid(format!(
                "the {} profile is built in and can't be deleted",
                cur.name
            )));
        }
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("delete mcp profile"))?;
        sqlx::query("DELETE FROM mcp_profiles WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("delete mcp profile"))?;
        if cur.is_default {
            sqlx::query("UPDATE mcp_profiles SET is_default = 1 WHERE id = ?")
                .bind(FULL_PROFILE_ID)
                .execute(&mut *tx)
                .await
                .map_err(dberr("restore default mcp profile"))?;
        }
        tx.commit().await.map_err(dberr("delete mcp profile"))?;
        Ok(cur)
    }

    /// Make `id` the one default (clear, then set — one transaction).
    pub async fn set_default(&self, id: &str) -> Result<McpProfile> {
        self.get(id).await?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("set default mcp profile"))?;
        sqlx::query("UPDATE mcp_profiles SET is_default = 0 WHERE is_default = 1")
            .execute(&mut *tx)
            .await
            .map_err(dberr("set default mcp profile"))?;
        sqlx::query("UPDATE mcp_profiles SET is_default = 1, updated_at = ? WHERE id = ?")
            .bind(fmt(Utc::now()))
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("set default mcp profile"))?;
        tx.commit()
            .await
            .map_err(dberr("set default mcp profile"))?;
        self.get(id).await
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

    fn fields(name: &str, groups: &[&str]) -> McpProfileFields {
        McpProfileFields {
            name: name.into(),
            groups: groups.iter().map(|g| g.to_string()).collect(),
            ..Default::default()
        }
    }

    async fn defaults(repo: &McpProfileRepo) -> Vec<String> {
        repo.list()
            .await
            .unwrap()
            .into_iter()
            .filter(|p| p.is_default)
            .map(|p| p.id)
            .collect()
    }

    #[tokio::test]
    async fn migration_seeds_the_presets_with_full_as_the_locked_default() {
        let repo = McpProfileRepo::new(mem_pool().await);
        let all = repo.list().await.unwrap();
        let ids: Vec<&str> = all.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids[0], FULL_PROFILE_ID, "Full leads the list");
        for id in ["full", "database", "issues", "git", "cloud"] {
            assert!(ids.contains(&id), "missing preset {id}");
        }
        let full = repo.get(FULL_PROFILE_ID).await.unwrap();
        assert!(full.all_tools && full.locked && full.builtin && full.is_default);
        assert_eq!(
            repo.get("issues").await.unwrap().groups,
            ["issues", "confluence"]
        );
        assert_eq!(defaults(&repo).await, [FULL_PROFILE_ID]);
    }

    #[tokio::test]
    async fn crud_round_trips_and_names_are_unique() {
        let repo = McpProfileRepo::new(mem_pool().await);
        let mut f = fields("Data only", &["database"]);
        f.exclude_tools = vec!["db_query".into()];
        let p = repo.create(&f, None).await.unwrap();
        assert_eq!(p.groups, ["database"]);
        assert_eq!(p.exclude_tools, ["db_query"]);
        assert!(!p.builtin && !p.locked && !p.is_default);

        let dup = repo.create(&fields("data ONLY", &[]), None).await;
        assert!(matches!(dup, Err(Error::Conflict(_))), "{dup:?}");

        f.include_tools = vec!["list_workspaces".into()];
        let p2 = repo.update(&p.id, &f).await.unwrap();
        assert_eq!(p2.include_tools, ["list_workspaces"]);

        repo.delete(&p.id).await.unwrap();
        assert!(matches!(repo.get(&p.id).await, Err(Error::NotFound(_))));
    }

    #[tokio::test]
    async fn exactly_one_default_and_full_is_undeletable_and_read_only() {
        let repo = McpProfileRepo::new(mem_pool().await);
        repo.set_default("database").await.unwrap();
        assert_eq!(defaults(&repo).await, ["database"]);
        assert_eq!(repo.default_profile().await.unwrap().id, "database");

        // Deleting the default hands it back to Full — never zero, never two.
        repo.delete("database").await.unwrap();
        assert_eq!(defaults(&repo).await, [FULL_PROFILE_ID]);

        assert!(matches!(
            repo.delete(FULL_PROFILE_ID).await,
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            repo.update(FULL_PROFILE_ID, &fields("Everything", &[]))
                .await,
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            repo.set_default("nope").await,
            Err(Error::NotFound(_))
        ));
        assert_eq!(defaults(&repo).await, [FULL_PROFILE_ID]);

        // The partial unique index refuses a second default written around
        // the repo.
        let raw = sqlx::query("UPDATE mcp_profiles SET is_default = 1 WHERE id = 'git'")
            .execute(&repo.pool)
            .await;
        assert!(raw.is_err());
    }

    #[tokio::test]
    async fn resolve_skips_unknown_ids_and_falls_back_to_the_default() {
        let repo = McpProfileRepo::new(mem_pool().await);
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let got = repo
            .resolve(&ids(&["database", "gone", "issues", "database"]))
            .await
            .unwrap();
        let names: Vec<&str> = got.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(names, ["database", "issues"]);
        assert_eq!(
            repo.resolve(&ids(&["gone"])).await.unwrap()[0].id,
            FULL_PROFILE_ID
        );
        assert_eq!(repo.resolve(&[]).await.unwrap()[0].id, FULL_PROFILE_ID);
        repo.set_default("cloud").await.unwrap();
        assert_eq!(
            repo.resolve(&ids(&["default"])).await.unwrap()[0].id,
            "cloud"
        );
    }
}
