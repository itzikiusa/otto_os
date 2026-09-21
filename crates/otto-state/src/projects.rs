//! Common workspace projects. The existing swarm_projects row is the canonical
//! identity; nullable swarm_id attaches optional execution machinery to it.
use crate::convert::{dberr, fmt, ts};
use chrono::{DateTime, Utc};
use otto_core::{domain::Session, new_id, Error, Id, Result};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: Id,
    pub workspace_id: Id,
    pub swarm_id: Option<Id>,
    pub name: String,
    pub description: String,
    pub repo_path: Option<String>,
    pub goal_md: String,
    pub instructions_md: String,
    pub references: Vec<String>,
    pub memory_md: String,
    pub decisions_md: String,
    pub artifacts: Vec<String>,
    pub status: String,
    pub context_version: i64,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Complete editable common surface. Swarm execution fields are never written.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct ProjectInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub repo_path: Option<String>,
    #[serde(default)]
    pub goal_md: String,
    #[serde(default)]
    pub instructions_md: String,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub memory_md: String,
    #[serde(default)]
    pub decisions_md: String,
    #[serde(default)]
    pub artifacts: Vec<String>,
    #[serde(default = "active")]
    pub status: String,
}
fn active() -> String {
    "active".into()
}
impl ProjectInput {
    fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() || self.name.len() > 200 {
            return Err(Error::Invalid("project name must be 1–200 bytes".into()));
        }
        if !self.status.is_empty() && !["active", "archived"].contains(&self.status.as_str()) {
            return Err(Error::Invalid(
                "project status must be active or archived".into(),
            ));
        }
        if [
            &self.description,
            &self.goal_md,
            &self.instructions_md,
            &self.memory_md,
            &self.decisions_md,
        ]
        .iter()
        .any(|s| s.len() > 32_000)
            || [&self.references, &self.artifacts]
                .iter()
                .any(|items| items.len() > 100 || items.iter().any(|s| s.len() > 2000))
        {
            return Err(Error::Invalid(
                "project text exceeds 32 KB or reference list exceeds 100 entries of 2 KB".into(),
            ));
        }
        let text_bytes = self.name.len()
            + self.description.len()
            + self.goal_md.len()
            + self.instructions_md.len()
            + self.memory_md.len()
            + self.decisions_md.len()
            + self.repo_path.as_deref().unwrap_or("").len()
            + self
                .references
                .iter()
                .chain(self.artifacts.iter())
                .map(String::len)
                .sum::<usize>();
        if text_bytes > 64 * 1024
            || self
                .repo_path
                .as_ref()
                .is_some_and(|path| path.len() > 2000)
        {
            return Err(Error::Invalid("project context must fit within 64 KB; use references for longer documents (repository path max 2 KB)".into()));
        }
        Ok(())
    }
}
impl Project {
    /// Curated text only: references do not trigger filesystem/network reads.
    pub fn context_markdown(&self) -> String {
        let mut out = format!(
            "# Project: {}\n\nProject ID: {}\nContext version: {}\n",
            self.name, self.id, self.context_version
        );
        for (label, text) in [
            ("Description", &self.description),
            ("Goal", &self.goal_md),
            ("Instructions", &self.instructions_md),
            ("Project memory", &self.memory_md),
            ("Decisions", &self.decisions_md),
        ] {
            if !text.trim().is_empty() {
                out.push_str(&format!("\n## {label}\n\n{text}\n"));
            }
        }
        if let Some(path) = &self.repo_path {
            out.push_str(&format!("\nRepository reference: {path}\n"));
        }
        for (label, items) in [
            ("References", &self.references),
            ("Artifacts", &self.artifacts),
        ] {
            if !items.is_empty() {
                out.push_str(&format!("\n## {label}\n"));
                for item in items {
                    out.push_str(&format!("\n- {item}"));
                }
                out.push('\n');
            }
        }
        // Legacy Swarm project text predates the edit limits. Keep launch
        // arguments bounded without changing those persisted user documents.
        if out.len() > 64 * 1024 {
            let mut end = 64 * 1024 - 100;
            while !out.is_char_boundary(end) {
                end -= 1;
            }
            out.truncate(end);
            out.push_str(
                "\n\n[Project context shortened; open the project for the full saved content.]\n",
            );
        }
        out
    }
}
fn row(r: &sqlx::sqlite::SqliteRow) -> Result<Project> {
    let strings = |name| {
        serde_json::from_str::<Vec<String>>(&r.get::<String, _>(name))
            .map_err(|e| Error::Internal(format!("project list: {e}")))
    };
    Ok(Project {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        swarm_id: r.get("swarm_id"),
        name: r.get("name"),
        description: r.get("description"),
        repo_path: r.get("repo_path"),
        goal_md: r.get::<Option<String>, _>("goal_md").unwrap_or_default(),
        instructions_md: r.get("instructions_md"),
        references: strings("references_json")?,
        memory_md: r.get("memory_md"),
        decisions_md: r.get("decisions_md"),
        artifacts: strings("artifacts_json")?,
        status: r.get("status"),
        context_version: r.get("context_version"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}
pub struct ProjectSessionPage {
    pub items: Vec<Session>,
    /// Original DB text avoids changing timestamp spelling at page boundaries.
    pub next: Option<(String, Id)>,
}

#[derive(Clone)]
pub struct ProjectsRepo {
    pool: SqlitePool,
}
impl ProjectsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
    pub async fn list(&self, workspace_id: &Id) -> Result<Vec<Project>> {
        let rows = sqlx::query(
            "SELECT * FROM swarm_projects WHERE workspace_id = ? ORDER BY updated_at DESC, id",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list projects"))?;
        rows.iter().map(row).collect()
    }
    pub async fn get(&self, id: &Id) -> Result<Project> {
        row(&sqlx::query("SELECT * FROM swarm_projects WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("project"))?)
    }
    pub async fn create(
        &self,
        workspace_id: &Id,
        user_id: &Id,
        input: ProjectInput,
    ) -> Result<Project> {
        input.validate()?;
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query("INSERT INTO swarm_projects (id, workspace_id, swarm_id, name, description, repo_path, goal_md, instructions_md, references_json, memory_md, decisions_md, artifacts_json, status, created_by, created_at, updated_at) VALUES (?, ?, NULL, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&id).bind(workspace_id).bind(input.name.trim()).bind(input.description).bind(input.repo_path).bind(input.goal_md).bind(input.instructions_md)
            .bind(serde_json::to_string(&input.references).unwrap()).bind(input.memory_md).bind(input.decisions_md).bind(serde_json::to_string(&input.artifacts).unwrap())
            .bind(if input.status.is_empty() { active() } else { input.status }).bind(user_id).bind(&now).bind(&now)
            .execute(&self.pool).await.map_err(dberr("create project"))?;
        self.get(&id).await
    }
    pub async fn update(
        &self,
        id: &Id,
        input: ProjectInput,
        expected_version: i64,
    ) -> Result<Project> {
        input.validate()?;
        let result = sqlx::query("UPDATE swarm_projects SET name=?, description=?, repo_path=?, goal_md=?, instructions_md=?, references_json=?, memory_md=?, decisions_md=?, artifacts_json=?, status=?, context_version=context_version+1, updated_at=? WHERE id=? AND context_version=?")
            .bind(input.name.trim()).bind(input.description).bind(input.repo_path).bind(input.goal_md).bind(input.instructions_md)
            .bind(serde_json::to_string(&input.references).unwrap()).bind(input.memory_md).bind(input.decisions_md).bind(serde_json::to_string(&input.artifacts).unwrap())
            .bind(if input.status.is_empty() { active() } else { input.status }).bind(fmt(Utc::now())).bind(id).bind(expected_version)
            .execute(&self.pool).await.map_err(dberr("update project"))?;
        if result.rows_affected() == 0 {
            return Err(Error::Conflict(
                "project changed; reload before saving".into(),
            ));
        }
        self.get(id).await
    }
    pub async fn sessions(&self, id: &Id, workspace_id: &Id, limit: i64) -> Result<Vec<Session>> {
        Ok(self
            .session_page(id, workspace_id, None, None, limit)
            .await?
            .items)
    }

    /// Owner filtering precedes LIMIT. Keyset paging lets the HTTP layer skip
    /// resource-denied candidates without hiding older accessible membership.
    pub async fn session_page(
        &self,
        id: &Id,
        workspace_id: &Id,
        owner: Option<&str>,
        before: Option<(&str, &str)>,
        limit: i64,
    ) -> Result<ProjectSessionPage> {
        let mut query = sqlx::QueryBuilder::new("SELECT * FROM sessions WHERE workspace_id=");
        query
            .push_bind(workspace_id)
            .push(" AND json_extract(meta_json, '$.project_id')=")
            .push_bind(id);
        if let Some(owner) = owner {
            query.push(" AND created_by=").push_bind(owner);
        }
        if let Some((at, before_id)) = before {
            query
                .push(" AND (last_active_at < ")
                .push_bind(at)
                .push(" OR (last_active_at = ")
                .push_bind(at)
                .push(" AND id < ")
                .push_bind(before_id)
                .push("))");
        }
        query
            .push(" ORDER BY last_active_at DESC, id DESC LIMIT ")
            .push_bind(limit.clamp(1, 500));
        let rows = query
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("project sessions"))?;
        let next = rows
            .last()
            .map(|row| (row.get("last_active_at"), row.get("id")));
        Ok(ProjectSessionPage {
            items: rows
                .iter()
                .map(crate::sessions::row_to_session)
                .collect::<Result<_>>()?,
            next,
        })
    }
    pub async fn context(
        &self,
        workspace_id: &Id,
        meta: &serde_json::Value,
    ) -> Result<Option<String>> {
        let id = match meta.get("project_id") {
            None | Some(serde_json::Value::Null) => return Ok(None),
            Some(serde_json::Value::String(id)) if !id.trim().is_empty() => id,
            _ => {
                return Err(Error::Invalid(
                    "project_id must be a nonempty project ID or null".into(),
                ))
            }
        };
        let project = self.get(id).await?;
        if &project.workspace_id != workspace_id {
            return Err(Error::Forbidden(
                "project belongs to a different workspace".into(),
            ));
        }
        Ok(Some(project.context_markdown()))
    }
}
