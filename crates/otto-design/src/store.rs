//! SQLite persistence for the design graph (`design_*` tables, see the
//! `design_graph` migration) plus the runtime FTS5 search index. Rows hold
//! metadata only — content lives in the blob store. No foreign keys: child
//! rows are removed explicitly here (the canvas/product convention).

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, SecondsFormat, Utc};
use otto_core::{new_id, Error, Id, Result};
use serde_json::Value;
use sqlx::{Row, SqlitePool};

use crate::graph::Adjacency;
use crate::retention::VersionInfo;
use crate::types::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn dberr(op: &'static str) -> impl Fn(sqlx::Error) -> Error {
    move |e| match e {
        sqlx::Error::RowNotFound => Error::NotFound(op.to_string()),
        other => Error::Internal(format!("{op}: {other}")),
    }
}

fn is_unique(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(db) if db.is_unique_violation())
}

/// Storage timestamp: RFC 3339, nanoseconds, `Z` — fixed width, so the TEXT
/// columns sort chronologically.
pub fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

fn now() -> String {
    stamp(Utc::now())
}

fn ts(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| Error::Internal(format!("bad timestamp '{s}': {e}")))
}

/// Lenient JSON column read: a corrupt value degrades to `fallback`.
fn json_or(s: &str, fallback: Value) -> Value {
    serde_json::from_str(s).unwrap_or(fallback)
}

fn empty_to_none(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

fn placeholders(n: usize) -> String {
    vec!["?"; n].join(",")
}

/// A dynamically-bound query argument.
enum Arg {
    S(String),
    I(i64),
}

// ---------------------------------------------------------------------------
// Row mappers
// ---------------------------------------------------------------------------

fn row_project(r: &sqlx::sqlite::SqliteRow) -> Result<DesignProject> {
    Ok(DesignProject {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        name: r.get("name"),
        description: r.get("description"),
        epic_story_id: r.get("epic_story_id"),
        swarm_project_id: r.get("swarm_project_id"),
        brand_kit_id: r.get("brand_kit_id"),
        cover_artifact_id: r.get("cover_artifact_id"),
        archived: r.get::<i64, _>("archived") != 0,
        meta: json_or(
            &r.get::<String, _>("meta_json"),
            Value::Object(Default::default()),
        ),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
        artifact_count: r.try_get::<i64, _>("artifact_count").unwrap_or(0),
    })
}

fn row_artifact(r: &sqlx::sqlite::SqliteRow) -> Result<DesignArtifact> {
    let tags: Vec<String> =
        serde_json::from_str(&r.get::<String, _>("tags_json")).unwrap_or_default();
    Ok(DesignArtifact {
        id: r.get("id"),
        project_id: r.get("project_id"),
        workspace_id: r.get("workspace_id"),
        studio: r.get("studio"),
        format: r.get("format"),
        mime: r.get("mime"),
        title: r.get("title"),
        status: r.get("status"),
        head_version_id: r.get("head_version_id"),
        head_seq: r.try_get::<Option<i64>, _>("head_seq").unwrap_or(None),
        approved_version_id: r.get("approved_version_id"),
        tags,
        thumb_blob: r.get("thumb_blob"),
        meta: json_or(
            &r.get::<String, _>("meta_json"),
            Value::Object(Default::default()),
        ),
        source_kind: r.get("source_kind"),
        source_id: r.get("source_id"),
        created_by: r.get("created_by"),
        created_by_kind: r.get("created_by_kind"),
        created_session_id: r.get("created_session_id"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
        created_by_name: opt_col(r, "created_by_name"),
        last_editor_id: opt_col(r, "last_editor_id"),
        last_editor_kind: opt_col(r, "last_editor_kind"),
        last_editor_name: opt_col(r, "last_editor_name"),
        story_ids: split_ids(opt_col(r, "story_ids_joined")),
    })
}

/// A `group_concat(…, char(10))` id list → sorted, de-duplicated ids.
fn split_ids(joined: Option<String>) -> Vec<Id> {
    let mut ids: Vec<Id> = joined
        .unwrap_or_default()
        .split('\n')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// A nullable TEXT column that only some selects join in (absent → `None`).
fn opt_col(r: &sqlx::sqlite::SqliteRow, col: &str) -> Option<String> {
    r.try_get::<Option<String>, _>(col).ok().flatten()
}

fn row_version(r: &sqlx::sqlite::SqliteRow) -> Result<DesignVersion> {
    Ok(DesignVersion {
        id: r.get("id"),
        artifact_id: r.get("artifact_id"),
        seq: r.get("seq"),
        parent_version_id: r.get("parent_version_id"),
        branch: r.get("branch"),
        blob_sha256: r.get("blob_sha256"),
        size_bytes: r.get("size_bytes"),
        kind: r.get("kind"),
        author_kind: r.get("author_kind"),
        author_id: r.get("author_id"),
        session_id: r.get("session_id"),
        message: r.get("message"),
        provenance: json_or(
            &r.get::<String, _>("provenance_json"),
            Value::Object(Default::default()),
        ),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        author_name: opt_col(r, "author_name"),
    })
}

fn row_link(r: &sqlx::sqlite::SqliteRow) -> Result<DesignLink> {
    Ok(DesignLink {
        id: r.get("id"),
        src_artifact_id: r.get("src_artifact_id"),
        src_version_id: r.get("src_version_id"),
        src_node: empty_to_none(r.get("src_node")),
        dst_kind: r.get("dst_kind"),
        dst_id: r.get("dst_id"),
        dst_node: empty_to_none(r.get("dst_node")),
        rel: r.get("rel"),
        policy: r.get("policy"),
        pinned_version_id: r.get("pinned_version_id"),
        origin: r.get("origin"),
        broken: r.get::<i64, _>("broken") != 0,
        meta: json_or(
            &r.get::<String, _>("meta_json"),
            Value::Object(Default::default()),
        ),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

fn row_signal(r: &sqlx::sqlite::SqliteRow) -> Result<DesignSignal> {
    Ok(DesignSignal {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        artifact_id: r.get("artifact_id"),
        version_id: r.get("version_id"),
        kind: r.get("kind"),
        actor_kind: r.get("actor_kind"),
        actor_id: r.get("actor_id"),
        session_id: r.get("session_id"),
        payload: json_or(
            &r.get::<String, _>("payload_json"),
            Value::Object(Default::default()),
        ),
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

fn row_publish(r: &sqlx::sqlite::SqliteRow) -> Result<DesignPublish> {
    Ok(DesignPublish {
        id: r.get("id"),
        artifact_id: r.get("artifact_id"),
        version_id: r.get("version_id"),
        target: r.get("target"),
        url: r.get("url"),
        pinned_set: json_or(&r.get::<String, _>("pinned_set_json"), Value::Array(vec![])),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

/// SQL for a user's display name (`display_name`, else `username`); `$col`
/// is the column holding the user id. Unknown ids (system authors) → NULL.
macro_rules! user_name_of {
    ($col:literal) => {
        concat!(
            "(SELECT COALESCE(NULLIF(u.display_name, ''), u.username) FROM users u WHERE u.id = ",
            $col,
            ")"
        )
    };
}

/// Read-time enrichment of an artifact row (`a` + its head version `v`):
/// people, the head's author and the linked story ids. Every artifact
/// select carries these columns; `row_artifact` reads them leniently.
macro_rules! art_enrich_cols {
    () => {
        concat!(
            user_name_of!("a.created_by"),
            " AS created_by_name, v.author_id AS last_editor_id, \
             v.author_kind AS last_editor_kind, ",
            user_name_of!("v.author_id"),
            " AS last_editor_name, \
             (SELECT group_concat(l.dst_id, char(10)) FROM design_links l \
              WHERE l.src_artifact_id = a.id AND l.dst_kind = 'story') AS story_ids_joined"
        )
    };
}

const ART_SELECT: &str = concat!(
    "SELECT a.*, v.seq AS head_seq, ",
    art_enrich_cols!(),
    " FROM design_artifacts a LEFT JOIN design_versions v ON v.id = a.head_version_id"
);

/// Every version select: the row plus its author's display name.
const VER_SELECT: &str = concat!(
    "SELECT design_versions.*, ",
    user_name_of!("design_versions.author_id"),
    " AS author_name FROM design_versions"
);

const PROJECT_SELECT: &str = "SELECT p.*, (SELECT COUNT(*) FROM design_artifacts a \
     WHERE a.project_id = p.id AND a.status != 'archived') AS artifact_count \
     FROM design_projects p";

/// Row cap of one bulk links read (`GET /design/links`).
pub const MAX_BULK_LINKS: usize = 10_000;

/// Status order for "shipped first" listings.
const STATUS_ORDER: &str = "CASE a.status WHEN 'shipped' THEN 0 WHEN 'approved' THEN 1 \
     WHEN 'review' THEN 2 WHEN 'draft' THEN 3 ELSE 4 END";

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

pub struct NewProject {
    pub workspace_id: Id,
    pub name: String,
    pub description: String,
    pub epic_story_id: Option<Id>,
    pub swarm_project_id: Option<Id>,
    pub brand_kit_id: Option<Id>,
    pub meta: Value,
    pub created_by: Id,
}

pub struct NewArtifactRow {
    pub id: Id,
    pub project_id: Option<Id>,
    pub workspace_id: Id,
    pub studio: String,
    pub format: String,
    pub mime: String,
    pub title: String,
    pub status: String,
    pub tags: Vec<String>,
    pub meta: Value,
    pub source_kind: Option<String>,
    pub source_id: Option<Id>,
    pub created_by: Id,
    pub created_by_kind: String,
    pub created_session_id: Option<Id>,
    /// `None` = now (imports pass the source row's timestamp).
    pub created_at: Option<DateTime<Utc>>,
}

pub struct NewVersion {
    pub artifact_id: Id,
    pub blob_sha256: String,
    pub size_bytes: i64,
    pub kind: String,
    pub branch: String,
    pub author_kind: String,
    pub author_id: String,
    pub session_id: Option<Id>,
    pub message: String,
    pub provenance: Value,
}

#[derive(Debug, Clone)]
pub struct NewLink {
    pub src_artifact_id: Id,
    pub src_version_id: Option<Id>,
    /// `''` when the link hangs off the document as a whole.
    pub src_node: String,
    pub dst_kind: String,
    pub dst_id: String,
    pub dst_node: String,
    pub rel: String,
    pub policy: String,
    pub pinned_version_id: Option<Id>,
    pub origin: String,
    pub broken: bool,
    pub meta: Value,
    pub created_by: String,
}

pub struct NewSignal {
    pub workspace_id: Id,
    pub artifact_id: Id,
    pub version_id: Option<Id>,
    pub kind: String,
    pub actor_kind: String,
    pub actor_id: String,
    pub session_id: Option<Id>,
    pub payload: Value,
}

pub struct NewPublish {
    pub artifact_id: Id,
    pub version_id: Id,
    pub target: String,
    pub url: Option<String>,
    pub pinned_set: Value,
    pub created_by: Id,
}

/// Filters shared by `GET /design/artifacts` and `GET /design/search`.
#[derive(Debug, Clone, Default)]
pub struct ArtifactFilter {
    /// `Some(list)` restricts to these workspaces (non-root callers); `None` =
    /// every workspace (root).
    pub workspaces: Option<Vec<Id>>,
    pub project_id: Option<Id>,
    pub studio: Option<String>,
    pub format: Option<String>,
    pub status: Option<String>,
    pub story_id: Option<Id>,
    /// With `story_id`: also match artifacts linked to the story's direct
    /// children (an epic's Design tab).
    pub include_children: bool,
    pub author_kind: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub include_archived: bool,
    pub limit: i64,
    pub offset: i64,
    /// Keyset page (`list_artifacts` only): rows strictly after this
    /// `(updated_at stamp, id)` in newest-first order; `offset` is ignored.
    pub cursor: Option<(String, Id)>,
}

impl ArtifactFilter {
    /// Append the filter's WHERE clauses (each prefixed `AND`) + args.
    fn push_where(&self, sql: &mut String, args: &mut Vec<Arg>) {
        if let Some(ws) = &self.workspaces {
            sql.push_str(&format!(
                " AND a.workspace_id IN ({})",
                placeholders(ws.len())
            ));
            args.extend(ws.iter().map(|w| Arg::S(w.clone())));
        }
        if let Some(p) = &self.project_id {
            sql.push_str(" AND a.project_id = ?");
            args.push(Arg::S(p.clone()));
        }
        if let Some(s) = &self.studio {
            sql.push_str(" AND a.studio = ?");
            args.push(Arg::S(s.clone()));
        }
        if let Some(f) = &self.format {
            sql.push_str(" AND a.format = ?");
            args.push(Arg::S(f.clone()));
        }
        match &self.status {
            Some(s) => {
                sql.push_str(" AND a.status = ?");
                args.push(Arg::S(s.clone()));
            }
            None if !self.include_archived => sql.push_str(" AND a.status != 'archived'"),
            None => {}
        }
        if let Some(story) = &self.story_id {
            if self.include_children {
                sql.push_str(
                    " AND EXISTS (SELECT 1 FROM design_links l WHERE l.src_artifact_id = a.id \
                     AND l.dst_kind = 'story' AND (l.dst_id = ? OR l.dst_id IN \
                     (SELECT s.id FROM product_stories s WHERE s.parent_id = ?)))",
                );
                args.push(Arg::S(story.clone()));
                args.push(Arg::S(story.clone()));
            } else {
                sql.push_str(
                    " AND EXISTS (SELECT 1 FROM design_links l WHERE l.src_artifact_id = a.id \
                     AND l.dst_kind = 'story' AND l.dst_id = ?)",
                );
                args.push(Arg::S(story.clone()));
            }
        }
        if let Some(k) = &self.author_kind {
            sql.push_str(" AND a.created_by_kind = ?");
            args.push(Arg::S(k.clone()));
        }
        if let Some(t) = &self.since {
            sql.push_str(" AND a.updated_at >= ?");
            args.push(Arg::S(t.clone()));
        }
        if let Some(t) = &self.until {
            sql.push_str(" AND a.updated_at <= ?");
            args.push(Arg::S(t.clone()));
        }
    }

    /// The page size a listing actually uses (default 100, cap 500).
    pub fn effective_limit(&self) -> i64 {
        self.page().0
    }

    fn page(&self) -> (i64, i64) {
        let limit = if self.limit > 0 {
            self.limit.min(500)
        } else {
            100
        };
        (limit, self.offset.max(0))
    }
}

/// Tokenize free text into a safe FTS5 MATCH expression: each ≥2-char
/// alphanumeric term is double-quoted (so `:`/`-`/`*`/`(` can't be a syntax
/// error) and implicitly AND-ed; the LAST term is a prefix match so the
/// References drawer can search as you type. `None` when there are no terms.
pub fn fts_match(query: &str) -> Option<String> {
    let terms: Vec<String> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.chars().count() >= 2)
        .take(16)
        .map(|s| format!("\"{s}\""))
        .collect();
    if terms.is_empty() {
        return None;
    }
    let n = terms.len();
    Some(
        terms
            .into_iter()
            .enumerate()
            .map(|(i, t)| if i + 1 == n { format!("{t}*") } else { t })
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// The keyset cursor of a listed artifact — `<updated_at>|<id>`. Clients may
/// build the same string from a row's JSON (`updated_at` + `|` + `id`).
pub fn artifact_cursor(a: &DesignArtifact) -> String {
    format!("{}|{}", stamp(a.updated_at), a.id)
}

/// Parse an `artifact_cursor` (any RFC 3339 spelling of the time) into the
/// storage stamp + id the listing compares against.
pub fn parse_cursor(s: &str) -> Result<(String, Id)> {
    let bad = || Error::Invalid(format!("bad cursor {s:?} (expected <updated_at>|<id>)"));
    let (at, id) = s.trim().rsplit_once('|').ok_or_else(bad)?;
    if id.is_empty() || id.len() > 128 {
        return Err(bad());
    }
    let at = DateTime::parse_from_rfc3339(at).map_err(|_| bad())?;
    Ok((stamp(at.with_timezone(&Utc)), id.to_string()))
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    // -- projects -------------------------------------------------------------

    pub async fn create_project(&self, p: NewProject) -> Result<DesignProject> {
        let id = new_id();
        let now = now();
        sqlx::query(
            "INSERT INTO design_projects
             (id, workspace_id, name, description, epic_story_id, swarm_project_id,
              brand_kit_id, cover_artifact_id, archived, meta_json, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, NULL, 0, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&p.workspace_id)
        .bind(&p.name)
        .bind(&p.description)
        .bind(&p.epic_story_id)
        .bind(&p.swarm_project_id)
        .bind(&p.brand_kit_id)
        .bind(p.meta.to_string())
        .bind(&p.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("design.project.create"))?;
        self.get_project(&id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("design project {id}")))
    }

    pub async fn get_project(&self, id: &str) -> Result<Option<DesignProject>> {
        let row = sqlx::query(&format!("{PROJECT_SELECT} WHERE p.id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("design.project.get"))?;
        row.as_ref().map(row_project).transpose()
    }

    pub async fn list_projects(
        &self,
        workspaces: Option<&[Id]>,
        include_archived: bool,
    ) -> Result<Vec<DesignProject>> {
        let mut sql = format!("{PROJECT_SELECT} WHERE 1 = 1");
        let mut args = Vec::new();
        if let Some(ws) = workspaces {
            if ws.is_empty() {
                return Ok(vec![]);
            }
            sql.push_str(&format!(
                " AND p.workspace_id IN ({})",
                placeholders(ws.len())
            ));
            args.extend(ws.iter().map(|w| Arg::S(w.clone())));
        }
        if !include_archived {
            sql.push_str(" AND p.archived = 0");
        }
        sql.push_str(" ORDER BY p.updated_at DESC");
        let mut q = sqlx::query(&sql);
        for a in &args {
            q = match a {
                Arg::S(s) => q.bind(s.as_str()),
                Arg::I(i) => q.bind(*i),
            };
        }
        let rows = q
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("design.project.list"))?;
        rows.iter().map(row_project).collect()
    }

    /// Write every mutable column of `p` (the caller merged the patch).
    pub async fn write_project(&self, p: &DesignProject) -> Result<DesignProject> {
        let res = sqlx::query(
            "UPDATE design_projects SET name = ?, description = ?, epic_story_id = ?,
                    swarm_project_id = ?, brand_kit_id = ?, cover_artifact_id = ?,
                    archived = ?, meta_json = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&p.name)
        .bind(&p.description)
        .bind(&p.epic_story_id)
        .bind(&p.swarm_project_id)
        .bind(&p.brand_kit_id)
        .bind(&p.cover_artifact_id)
        .bind(i64::from(p.archived))
        .bind(p.meta.to_string())
        .bind(now())
        .bind(&p.id)
        .execute(&self.pool)
        .await
        .map_err(dberr("design.project.update"))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("design project {}", p.id)));
        }
        self.get_project(&p.id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("design project {}", p.id)))
    }

    /// Hard-delete an EMPTY project (its artifacts are never deleted with it —
    /// a project that still files any artifact is a 409).
    pub async fn delete_project(&self, id: &str) -> Result<()> {
        let n: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM design_artifacts WHERE project_id = ?")
                .bind(id)
                .fetch_one(&self.pool)
                .await
                .map_err(dberr("design.project.delete.count"))?;
        if n > 0 {
            return Err(Error::Conflict(format!(
                "design project {id} still files {n} artifact(s); move or archive them first"
            )));
        }
        let res = sqlx::query("DELETE FROM design_projects WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("design.project.delete"))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("design project {id}")));
        }
        Ok(())
    }

    /// Distinct workspaces that own any project / artifact (for the caller's
    /// visibility filter).
    pub async fn known_workspaces(&self) -> Result<Vec<Id>> {
        sqlx::query_scalar(
            "SELECT workspace_id FROM design_projects
             UNION SELECT workspace_id FROM design_artifacts",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("design.workspaces"))
    }

    // -- artifacts ------------------------------------------------------------

    /// Insert an artifact row (no version yet). A second row for the same
    /// legacy `source_kind`/`source_id` is a `Conflict` (the import's
    /// idempotency key).
    pub async fn insert_artifact(&self, a: &NewArtifactRow) -> Result<()> {
        let created = a.created_at.map(stamp).unwrap_or_else(now);
        sqlx::query(
            "INSERT INTO design_artifacts
             (id, project_id, workspace_id, studio, format, mime, title, status,
              head_version_id, approved_version_id, tags_json, thumb_blob, meta_json,
              source_kind, source_id, created_by, created_by_kind, created_session_id,
              created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, ?, NULL, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&a.id)
        .bind(&a.project_id)
        .bind(&a.workspace_id)
        .bind(&a.studio)
        .bind(&a.format)
        .bind(&a.mime)
        .bind(&a.title)
        .bind(&a.status)
        .bind(serde_json::to_string(&a.tags).unwrap_or_else(|_| "[]".into()))
        .bind(a.meta.to_string())
        .bind(&a.source_kind)
        .bind(&a.source_id)
        .bind(&a.created_by)
        .bind(&a.created_by_kind)
        .bind(&a.created_session_id)
        .bind(&created)
        .bind(&created)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            if is_unique(&e) {
                Error::Conflict(format!(
                    "a design artifact already mirrors {:?} {:?}",
                    a.source_kind, a.source_id
                ))
            } else {
                Error::Internal(format!("design.artifact.insert: {e}"))
            }
        })?;
        Ok(())
    }

    pub async fn get_artifact(&self, id: &str) -> Result<Option<DesignArtifact>> {
        let row = sqlx::query(&format!("{ART_SELECT} WHERE a.id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("design.artifact.get"))?;
        row.as_ref().map(row_artifact).transpose()
    }

    pub async fn require_artifact(&self, id: &str) -> Result<DesignArtifact> {
        self.get_artifact(id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("design artifact {id}")))
    }

    pub async fn find_by_source(&self, kind: &str, id: &str) -> Result<Option<DesignArtifact>> {
        let row = sqlx::query(&format!(
            "{ART_SELECT} WHERE a.source_kind = ? AND a.source_id = ?"
        ))
        .bind(kind)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("design.artifact.by_source"))?;
        row.as_ref().map(row_artifact).transpose()
    }

    /// Every imported artifact keyed by `(source_kind, source_id)` — one query
    /// so the startup import stays cheap on a big library.
    pub async fn source_index(&self) -> Result<HashMap<(String, String), DesignArtifact>> {
        let rows = sqlx::query(&format!("{ART_SELECT} WHERE a.source_kind IS NOT NULL"))
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("design.artifact.source_index"))?;
        let mut out = HashMap::new();
        for r in &rows {
            let a = row_artifact(r)?;
            if let (Some(k), Some(i)) = (a.source_kind.clone(), a.source_id.clone()) {
                out.insert((k, i), a);
            }
        }
        Ok(out)
    }

    pub async fn list_artifacts(&self, f: &ArtifactFilter) -> Result<Vec<DesignArtifact>> {
        if f.workspaces.as_ref().is_some_and(|w| w.is_empty()) {
            return Ok(vec![]);
        }
        let mut sql = format!("{ART_SELECT} WHERE 1 = 1");
        let mut args = Vec::new();
        f.push_where(&mut sql, &mut args);
        let (limit, mut offset) = f.page();
        if let Some((at, id)) = &f.cursor {
            sql.push_str(" AND (a.updated_at < ? OR (a.updated_at = ? AND a.id < ?))");
            args.push(Arg::S(at.clone()));
            args.push(Arg::S(at.clone()));
            args.push(Arg::S(id.clone()));
            offset = 0;
        }
        sql.push_str(" ORDER BY a.updated_at DESC, a.id DESC LIMIT ? OFFSET ?");
        args.push(Arg::I(limit));
        args.push(Arg::I(offset));
        let mut q = sqlx::query(&sql);
        for a in &args {
            q = match a {
                Arg::S(s) => q.bind(s.as_str()),
                Arg::I(i) => q.bind(*i),
            };
        }
        let rows = q
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("design.artifact.list"))?;
        rows.iter().map(row_artifact).collect()
    }

    /// Every artifact id (the opt-in prune walks the whole library).
    pub async fn all_artifact_ids(&self) -> Result<Vec<Id>> {
        sqlx::query_scalar("SELECT id FROM design_artifacts ORDER BY created_at")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("design.artifact.all_ids"))
    }

    /// Artifacts by id (unknown ids are skipped).
    pub async fn artifacts_by_ids(&self, ids: &[String]) -> Result<Vec<DesignArtifact>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let sql = format!("{ART_SELECT} WHERE a.id IN ({})", placeholders(ids.len()));
        let mut q = sqlx::query(&sql);
        for id in ids {
            q = q.bind(id.as_str());
        }
        let rows = q
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("design.artifact.by_ids"))?;
        rows.iter().map(row_artifact).collect()
    }

    /// Write the mutable metadata columns of `a` (the caller merged the patch).
    pub async fn write_artifact_meta(&self, a: &DesignArtifact) -> Result<DesignArtifact> {
        let res = sqlx::query(
            "UPDATE design_artifacts SET title = ?, project_id = ?, studio = ?, status = ?,
                    tags_json = ?, thumb_blob = ?, meta_json = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&a.title)
        .bind(&a.project_id)
        .bind(&a.studio)
        .bind(&a.status)
        .bind(serde_json::to_string(&a.tags).unwrap_or_else(|_| "[]".into()))
        .bind(&a.thumb_blob)
        .bind(a.meta.to_string())
        .bind(now())
        .bind(&a.id)
        .execute(&self.pool)
        .await
        .map_err(dberr("design.artifact.update"))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("design artifact {}", a.id)));
        }
        self.require_artifact(&a.id).await
    }

    /// Move `approved_version_id` (+ status). The version must belong to the
    /// artifact (checked by the caller).
    pub async fn set_approved(&self, id: &str, version_id: &str, status: &str) -> Result<()> {
        let res = sqlx::query(
            "UPDATE design_artifacts SET approved_version_id = ?, status = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(version_id)
        .bind(status)
        .bind(now())
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("design.artifact.approve"))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("design artifact {id}")));
        }
        Ok(())
    }

    /// HARD delete (workspace-admin only, explicit `?hard=true`): the row, its
    /// versions, its outgoing links and its search row. Incoming links are kept
    /// and flagged `broken` (consumers show a badge). Blobs are NOT touched —
    /// only the opt-in prune GC ever removes a blob.
    pub async fn delete_artifact(&self, id: &str) -> Result<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("design.artifact.delete.begin"))?;
        let res = sqlx::query("DELETE FROM design_artifacts WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("design.artifact.delete"))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("design artifact {id}")));
        }
        sqlx::query("DELETE FROM design_versions WHERE artifact_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("design.artifact.delete.versions"))?;
        sqlx::query("DELETE FROM design_links WHERE src_artifact_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("design.artifact.delete.links"))?;
        sqlx::query(
            "UPDATE design_links SET broken = 1 WHERE dst_kind = 'artifact' AND dst_id = ?",
        )
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(dberr("design.artifact.delete.backlinks"))?;
        tx.commit()
            .await
            .map_err(dberr("design.artifact.delete.commit"))?;
        self.fts_remove(id).await;
        Ok(())
    }

    // -- versions -------------------------------------------------------------

    /// Commit a version and move the head in ONE transaction. The guarded head
    /// UPDATE runs FIRST so the transaction takes SQLite's write lock before it
    /// reads anything (no deferred-upgrade `BUSY_SNAPSHOT` under concurrency).
    ///
    /// `base`: `None` = unconditional; `Some("")` = the caller expects no head
    /// yet; `Some(id)` = the head must still be `id`, else **409 Conflict**.
    pub async fn commit_version(&self, v: NewVersion, base: Option<&str>) -> Result<DesignVersion> {
        let id = new_id();
        let now_s = now();
        let (check, base_val) = match base {
            None => (0i64, String::new()),
            Some(b) => (1i64, b.to_string()),
        };
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("design.commit.begin"))?;
        let upd = sqlx::query(
            "UPDATE design_artifacts SET head_version_id = ?, updated_at = ?
             WHERE id = ? AND (? = 0 OR COALESCE(head_version_id, '') = ?)",
        )
        .bind(&id)
        .bind(&now_s)
        .bind(&v.artifact_id)
        .bind(check)
        .bind(&base_val)
        .execute(&mut *tx)
        .await
        .map_err(dberr("design.commit.head"))?;
        if upd.rows_affected() == 0 {
            let current: Option<Option<String>> =
                sqlx::query_scalar("SELECT head_version_id FROM design_artifacts WHERE id = ?")
                    .bind(&v.artifact_id)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(dberr("design.commit.probe"))?;
            return Err(match current {
                None => Error::NotFound(format!("design artifact {}", v.artifact_id)),
                Some(head) => Error::Conflict(format!(
                    "design artifact {} changed: head is {} but the save was based on {}; reload before saving",
                    v.artifact_id,
                    head.as_deref().unwrap_or("(none)"),
                    if base_val.is_empty() { "(none)" } else { base_val.as_str() }
                )),
            });
        }
        // The previous main-branch head (the new version is not inserted yet).
        let parent: Option<String> = sqlx::query_scalar(
            "SELECT id FROM design_versions WHERE artifact_id = ? AND branch = 'main'
             ORDER BY seq DESC LIMIT 1",
        )
        .bind(&v.artifact_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(dberr("design.commit.parent"))?;
        let seq: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM design_versions WHERE artifact_id = ?",
        )
        .bind(&v.artifact_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(dberr("design.commit.seq"))?;
        sqlx::query(
            "INSERT INTO design_versions
             (id, artifact_id, seq, parent_version_id, branch, blob_sha256, size_bytes, kind,
              author_kind, author_id, session_id, message, provenance_json, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&v.artifact_id)
        .bind(seq)
        .bind(&parent)
        .bind(&v.branch)
        .bind(&v.blob_sha256)
        .bind(v.size_bytes)
        .bind(&v.kind)
        .bind(&v.author_kind)
        .bind(&v.author_id)
        .bind(&v.session_id)
        .bind(&v.message)
        .bind(v.provenance.to_string())
        .bind(&now_s)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            if is_unique(&e) {
                Error::Conflict(format!(
                    "design artifact {} was saved concurrently; reload before saving",
                    v.artifact_id
                ))
            } else {
                Error::Internal(format!("design.commit.insert: {e}"))
            }
        })?;
        tx.commit().await.map_err(dberr("design.commit"))?;
        let author_name = self.user_display_name(&v.author_id).await;
        Ok(DesignVersion {
            id,
            artifact_id: v.artifact_id,
            seq,
            parent_version_id: parent,
            branch: v.branch,
            blob_sha256: v.blob_sha256,
            size_bytes: v.size_bytes,
            kind: v.kind,
            author_kind: v.author_kind,
            author_id: v.author_id,
            session_id: v.session_id,
            message: v.message,
            provenance: v.provenance,
            created_at: ts(&now_s)?,
            author_name,
        })
    }

    pub async fn get_version(&self, id: &str) -> Result<Option<DesignVersion>> {
        let row = sqlx::query(&format!("{VER_SELECT} WHERE id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("design.version.get"))?;
        row.as_ref().map(row_version).transpose()
    }

    pub async fn get_version_by_seq(
        &self,
        artifact_id: &str,
        seq: i64,
    ) -> Result<Option<DesignVersion>> {
        let row = sqlx::query(&format!("{VER_SELECT} WHERE artifact_id = ? AND seq = ?"))
            .bind(artifact_id)
            .bind(seq)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("design.version.by_seq"))?;
        row.as_ref().map(row_version).transpose()
    }

    /// Versions of an artifact, newest first.
    pub async fn list_versions(
        &self,
        artifact_id: &str,
        kind: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<DesignVersion>> {
        let limit = if limit > 0 { limit.min(1_000) } else { 200 };
        let rows = sqlx::query(&format!(
            "{VER_SELECT} WHERE artifact_id = ? AND (? IS NULL OR kind = ?)
             ORDER BY seq DESC LIMIT ? OFFSET ?"
        ))
        .bind(artifact_id)
        .bind(kind)
        .bind(kind)
        .bind(limit)
        .bind(offset.max(0))
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("design.version.list"))?;
        rows.iter().map(row_version).collect()
    }

    /// Insert a version on a SIDE branch (`variant/<run>/<k>`) WITHOUT moving
    /// the head — variants never touch main until one is accepted. `parent` is
    /// the main version the variant was generated from. The no-op artifact
    /// UPDATE runs first so the transaction takes SQLite's write lock before it
    /// reads `seq` (same reasoning as [`Self::commit_version`]); parallel
    /// variant commits therefore serialize instead of colliding on `seq`.
    pub async fn insert_side_version(
        &self,
        v: NewVersion,
        parent: Option<&str>,
    ) -> Result<DesignVersion> {
        if v.branch == "main" || v.branch.is_empty() {
            return Err(Error::Invalid(
                "a side version needs a non-main branch".into(),
            ));
        }
        let id = new_id();
        let now_s = now();
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("design.side.begin"))?;
        let upd = sqlx::query("UPDATE design_artifacts SET updated_at = updated_at WHERE id = ?")
            .bind(&v.artifact_id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("design.side.lock"))?;
        if upd.rows_affected() == 0 {
            return Err(Error::NotFound(format!(
                "design artifact {}",
                v.artifact_id
            )));
        }
        let seq: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM design_versions WHERE artifact_id = ?",
        )
        .bind(&v.artifact_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(dberr("design.side.seq"))?;
        let parent: Option<String> = parent.filter(|p| !p.is_empty()).map(str::to_string);
        sqlx::query(
            "INSERT INTO design_versions
             (id, artifact_id, seq, parent_version_id, branch, blob_sha256, size_bytes, kind,
              author_kind, author_id, session_id, message, provenance_json, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&v.artifact_id)
        .bind(seq)
        .bind(&parent)
        .bind(&v.branch)
        .bind(&v.blob_sha256)
        .bind(v.size_bytes)
        .bind(&v.kind)
        .bind(&v.author_kind)
        .bind(&v.author_id)
        .bind(&v.session_id)
        .bind(&v.message)
        .bind(v.provenance.to_string())
        .bind(&now_s)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            if is_unique(&e) {
                Error::Conflict(format!(
                    "design artifact {} was saved concurrently; retry",
                    v.artifact_id
                ))
            } else {
                Error::Internal(format!("design.side.insert: {e}"))
            }
        })?;
        tx.commit().await.map_err(dberr("design.side"))?;
        let author_name = self.user_display_name(&v.author_id).await;
        Ok(DesignVersion {
            id,
            artifact_id: v.artifact_id,
            seq,
            parent_version_id: parent,
            branch: v.branch,
            blob_sha256: v.blob_sha256,
            size_bytes: v.size_bytes,
            kind: v.kind,
            author_kind: v.author_kind,
            author_id: v.author_id,
            session_id: v.session_id,
            message: v.message,
            provenance: v.provenance,
            created_at: ts(&now_s)?,
            author_name,
        })
    }

    /// Versions of `artifact_id` whose branch starts with `prefix` (e.g.
    /// `variant/<run>/`), oldest first. Prefix-compared with `substr` so no
    /// LIKE escaping is needed.
    pub async fn versions_on_branch_prefix(
        &self,
        artifact_id: &str,
        prefix: &str,
    ) -> Result<Vec<DesignVersion>> {
        let rows = sqlx::query(&format!(
            "{VER_SELECT}
             WHERE artifact_id = ?1 AND substr(branch, 1, length(?2)) = ?2
             ORDER BY seq LIMIT 1000"
        ))
        .bind(artifact_id)
        .bind(prefix)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("design.version.branch"))?;
        rows.iter().map(row_version).collect()
    }

    pub async fn version_infos(&self, artifact_id: &str) -> Result<Vec<VersionInfo>> {
        let rows = sqlx::query(
            "SELECT id, seq, kind, created_at FROM design_versions WHERE artifact_id = ?",
        )
        .bind(artifact_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("design.version.infos"))?;
        rows.iter()
            .map(|r| {
                Ok(VersionInfo {
                    id: r.get("id"),
                    seq: r.get("seq"),
                    kind: r.get("kind"),
                    created_at: ts(&r.get::<String, _>("created_at"))?,
                })
            })
            .collect()
    }

    /// Versions of `a` that retention must never touch: head, approved, any
    /// version a link pins or was extracted from, published / publish-pinned
    /// versions, and versions a learning signal cites.
    pub async fn protected_versions(&self, a: &DesignArtifact) -> Result<HashSet<String>> {
        let mut out: HashSet<String> = HashSet::new();
        out.extend(a.head_version_id.clone());
        out.extend(a.approved_version_id.clone());
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT pinned_version_id FROM design_links
               WHERE pinned_version_id IS NOT NULL
                 AND pinned_version_id IN (SELECT id FROM design_versions WHERE artifact_id = ?1)
             UNION SELECT src_version_id FROM design_links
               WHERE src_artifact_id = ?1 AND src_version_id IS NOT NULL
             UNION SELECT version_id FROM design_publishes WHERE artifact_id = ?1
             UNION SELECT version_id FROM design_signals
               WHERE artifact_id = ?1 AND version_id IS NOT NULL",
        )
        .bind(&a.id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("design.version.protected"))?;
        out.extend(ids);
        // Versions pinned inside ANY publish's pinned set (embedded artifacts).
        let sets: Vec<String> = sqlx::query_scalar("SELECT pinned_set_json FROM design_publishes")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("design.version.protected.publishes"))?;
        for s in sets {
            if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&s) {
                for it in items {
                    if it.get("artifact_id").and_then(Value::as_str) == Some(a.id.as_str()) {
                        if let Some(v) = it.get("version_id").and_then(Value::as_str) {
                            out.insert(v.to_string());
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    /// Delete version rows (opt-in prune only). Returns their blob hashes.
    pub async fn delete_versions(&self, ids: &[String]) -> Result<Vec<String>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let ph = placeholders(ids.len());
        let sql = format!("SELECT DISTINCT blob_sha256 FROM design_versions WHERE id IN ({ph})");
        let mut q = sqlx::query_scalar::<_, String>(&sql);
        for id in ids {
            q = q.bind(id.as_str());
        }
        let blobs = q
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("design.version.prune.blobs"))?;
        let sql = format!("DELETE FROM design_versions WHERE id IN ({ph})");
        let mut q = sqlx::query(&sql);
        for id in ids {
            q = q.bind(id.as_str());
        }
        q.execute(&self.pool)
            .await
            .map_err(dberr("design.version.prune"))?;
        Ok(blobs)
    }

    /// Is `sha` still referenced by any version or thumbnail?
    pub async fn blob_in_use(&self, sha: &str) -> Result<bool> {
        let n: i64 = sqlx::query_scalar(
            "SELECT (SELECT COUNT(*) FROM design_versions WHERE blob_sha256 = ?1)
                  + (SELECT COUNT(*) FROM design_artifacts WHERE thumb_blob = ?1)",
        )
        .bind(sha)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("design.blob.in_use"))?;
        Ok(n > 0)
    }

    // -- links ----------------------------------------------------------------

    /// Replace every `extracted` link of `src` with `links` in one transaction.
    /// Returns whether the set of `(rel, dst_kind, dst_id, src_node, dst_node)`
    /// keys changed (drives the `design_link_updated` event).
    pub async fn replace_extracted(&self, src: &str, links: &[NewLink]) -> Result<bool> {
        type Key = (String, String, String, String, String);
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("design.links.replace.begin"))?;
        // DELETE first (takes the write lock before any read) and learn the old
        // key set from RETURNING.
        let old: Vec<Key> = sqlx::query_as(
            "DELETE FROM design_links WHERE src_artifact_id = ? AND origin = 'extracted'
             RETURNING rel, dst_kind, dst_id, src_node, dst_node",
        )
        .bind(src)
        .fetch_all(&mut *tx)
        .await
        .map_err(dberr("design.links.replace.delete"))?;
        for l in links {
            sqlx::query(
                "INSERT OR IGNORE INTO design_links
                 (id, src_artifact_id, src_version_id, src_node, dst_kind, dst_id, dst_node,
                  rel, policy, pinned_version_id, origin, broken, meta_json, created_by, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'extracted', ?, ?, ?, ?)",
            )
            .bind(new_id())
            .bind(&l.src_artifact_id)
            .bind(&l.src_version_id)
            .bind(&l.src_node)
            .bind(&l.dst_kind)
            .bind(&l.dst_id)
            .bind(&l.dst_node)
            .bind(&l.rel)
            .bind(&l.policy)
            .bind(&l.pinned_version_id)
            .bind(i64::from(l.broken))
            .bind(l.meta.to_string())
            .bind(&l.created_by)
            .bind(now())
            .execute(&mut *tx)
            .await
            .map_err(dberr("design.links.replace.insert"))?;
        }
        tx.commit()
            .await
            .map_err(dberr("design.links.replace.commit"))?;
        let old: HashSet<Key> = old.into_iter().collect();
        let new: HashSet<Key> = links
            .iter()
            .map(|l| {
                (
                    l.rel.clone(),
                    l.dst_kind.clone(),
                    l.dst_id.clone(),
                    l.src_node.clone(),
                    l.dst_node.clone(),
                )
            })
            .collect();
        Ok(old != new)
    }

    /// Insert an explicit link. A duplicate is a `Conflict`.
    pub async fn insert_link(&self, l: &NewLink) -> Result<DesignLink> {
        let id = new_id();
        sqlx::query(
            "INSERT INTO design_links
             (id, src_artifact_id, src_version_id, src_node, dst_kind, dst_id, dst_node,
              rel, policy, pinned_version_id, origin, broken, meta_json, created_by, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&l.src_artifact_id)
        .bind(&l.src_version_id)
        .bind(&l.src_node)
        .bind(&l.dst_kind)
        .bind(&l.dst_id)
        .bind(&l.dst_node)
        .bind(&l.rel)
        .bind(&l.policy)
        .bind(&l.pinned_version_id)
        .bind(&l.origin)
        .bind(i64::from(l.broken))
        .bind(l.meta.to_string())
        .bind(&l.created_by)
        .bind(now())
        .execute(&self.pool)
        .await
        .map_err(|e| {
            if is_unique(&e) {
                Error::Conflict("that link already exists".into())
            } else {
                Error::Internal(format!("design.link.insert: {e}"))
            }
        })?;
        self.get_link(&id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("design link {id}")))
    }

    pub async fn get_link(&self, id: &str) -> Result<Option<DesignLink>> {
        let row = sqlx::query("SELECT * FROM design_links WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("design.link.get"))?;
        row.as_ref().map(row_link).transpose()
    }

    pub async fn delete_link(&self, id: &str) -> Result<()> {
        let res = sqlx::query("DELETE FROM design_links WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("design.link.delete"))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("design link {id}")));
        }
        Ok(())
    }

    /// Links FROM `artifact_id` ("Uses").
    pub async fn links_out(&self, artifact_id: &str) -> Result<Vec<DesignLink>> {
        let rows = sqlx::query(
            "SELECT * FROM design_links WHERE src_artifact_id = ? ORDER BY rel, created_at",
        )
        .bind(artifact_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("design.links.out"))?;
        rows.iter().map(row_link).collect()
    }

    /// Links TO `artifact_id` from other artifacts ("Used in").
    pub async fn links_in(&self, artifact_id: &str) -> Result<Vec<DesignLink>> {
        let rows = sqlx::query(
            "SELECT * FROM design_links WHERE dst_kind = 'artifact' AND dst_id = ?
             ORDER BY rel, created_at",
        )
        .bind(artifact_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("design.links.in"))?;
        rows.iter().map(row_link).collect()
    }

    /// Links touching any of `ids` — FROM them (`out`) and/or TO them
    /// (`inn`) — each row once (a link between two of them is not doubled),
    /// grouped by source. Capped at [`MAX_BULK_LINKS`] rows.
    pub async fn links_touching(
        &self,
        ids: &[String],
        out: bool,
        inn: bool,
    ) -> Result<Vec<DesignLink>> {
        if ids.is_empty() || !(out || inn) {
            return Ok(vec![]);
        }
        let ph = placeholders(ids.len());
        let mut conds = Vec::new();
        if out {
            conds.push(format!("src_artifact_id IN ({ph})"));
        }
        if inn {
            conds.push(format!("(dst_kind = 'artifact' AND dst_id IN ({ph}))"));
        }
        let sql = format!(
            "SELECT * FROM design_links WHERE {} ORDER BY src_artifact_id, rel, created_at LIMIT {MAX_BULK_LINKS}",
            conds.join(" OR ")
        );
        let mut q = sqlx::query(&sql);
        for _ in 0..conds.len() {
            for id in ids {
                q = q.bind(id.as_str());
            }
        }
        let rows = q
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("design.links.bulk"))?;
        rows.iter().map(row_link).collect()
    }

    pub async fn link_counts(&self, artifact_id: &str) -> Result<(i64, i64)> {
        let row = sqlx::query(
            "SELECT (SELECT COUNT(*) FROM design_links WHERE src_artifact_id = ?1) AS n_out,
                    (SELECT COUNT(*) FROM design_links
                       WHERE dst_kind = 'artifact' AND dst_id = ?1) AS n_in",
        )
        .bind(artifact_id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("design.links.count"))?;
        Ok((row.get("n_out"), row.get("n_in")))
    }

    /// Bounded BFS over render edges from `start`, returning the adjacency of
    /// every node reached (≤ `max_nodes`).
    pub async fn render_adjacency(&self, start: &[String], max_nodes: usize) -> Result<Adjacency> {
        let mut adj = Adjacency::new();
        let mut seen: HashSet<String> = start.iter().cloned().collect();
        let mut frontier: Vec<String> = start.to_vec();
        let rels = placeholders(RENDER_RELS.len());
        while !frontier.is_empty() && seen.len() <= max_nodes {
            let sql = format!(
                "SELECT src_artifact_id, dst_id FROM design_links
                 WHERE dst_kind = 'artifact' AND rel IN ({rels}) AND src_artifact_id IN ({})",
                placeholders(frontier.len())
            );
            let mut q = sqlx::query(&sql);
            for r in RENDER_RELS {
                q = q.bind(*r);
            }
            for n in &frontier {
                q = q.bind(n.as_str());
            }
            let rows = q
                .fetch_all(&self.pool)
                .await
                .map_err(dberr("design.links.adjacency"))?;
            let mut next = Vec::new();
            for r in &rows {
                let s: String = r.get("src_artifact_id");
                let d: String = r.get("dst_id");
                adj.entry(s).or_default().push(d.clone());
                if seen.insert(d.clone()) {
                    next.push(d);
                }
            }
            frontier = next;
        }
        Ok(adj)
    }

    /// Product stories an artifact `implements`.
    pub async fn story_ids_for(&self, artifact_id: &str) -> Result<Vec<Id>> {
        sqlx::query_scalar(
            "SELECT dst_id FROM design_links WHERE src_artifact_id = ? AND dst_kind = 'story'
             ORDER BY created_at",
        )
        .bind(artifact_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("design.links.stories"))
    }

    /// How many OTHER artifacts embed / reference / derive from this one.
    pub async fn reference_count(&self, artifact_id: &str) -> Result<i64> {
        sqlx::query_scalar(
            "SELECT COUNT(DISTINCT src_artifact_id) FROM design_links
             WHERE dst_kind = 'artifact' AND dst_id = ?1 AND src_artifact_id != ?1",
        )
        .bind(artifact_id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("design.links.refcount"))
    }

    /// `"<KEY> <title>"` of a product story, for the search index.
    pub async fn story_label(&self, story_id: &str) -> Result<Option<String>> {
        let row = sqlx::query("SELECT source_key, title FROM product_stories WHERE id = ?")
            .bind(story_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("design.story.label"))?;
        Ok(row.map(|r| {
            format!(
                "{} {}",
                r.get::<String, _>("source_key"),
                r.get::<String, _>("title")
            )
        }))
    }

    /// A user's display name (`display_name`, else `username`); `None` for
    /// an unknown id (a system author) or a lookup error — names are cosmetic.
    pub async fn user_display_name(&self, user_id: &str) -> Option<String> {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT COALESCE(NULLIF(display_name, ''), username) FROM users WHERE id = ?",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()
        .flatten()
    }

    pub async fn story_workspace(&self, story_id: &str) -> Result<Option<Id>> {
        sqlx::query_scalar("SELECT workspace_id FROM product_stories WHERE id = ?")
            .bind(story_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("design.story.workspace"))
    }

    // -- signals / publishes --------------------------------------------------

    pub async fn insert_signal(&self, s: NewSignal) -> Result<DesignSignal> {
        let id = new_id();
        sqlx::query(
            "INSERT INTO design_signals
             (id, workspace_id, artifact_id, version_id, kind, actor_kind, actor_id,
              session_id, payload_json, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&s.workspace_id)
        .bind(&s.artifact_id)
        .bind(&s.version_id)
        .bind(&s.kind)
        .bind(&s.actor_kind)
        .bind(&s.actor_id)
        .bind(&s.session_id)
        .bind(s.payload.to_string())
        .bind(now())
        .execute(&self.pool)
        .await
        .map_err(dberr("design.signal.insert"))?;
        let row = sqlx::query("SELECT * FROM design_signals WHERE id = ?")
            .bind(&id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("design.signal.get"))?;
        row_signal(&row)
    }

    /// Signals, newest first. `workspaces` as in [`ArtifactFilter`].
    pub async fn list_signals(
        &self,
        workspaces: Option<&[Id]>,
        artifact_id: Option<&str>,
        kind: Option<&str>,
        since: Option<&str>,
        limit: i64,
    ) -> Result<Vec<DesignSignal>> {
        let mut sql = String::from("SELECT * FROM design_signals WHERE 1 = 1");
        let mut args = Vec::new();
        if let Some(ws) = workspaces {
            if ws.is_empty() {
                return Ok(vec![]);
            }
            sql.push_str(&format!(
                " AND workspace_id IN ({})",
                placeholders(ws.len())
            ));
            args.extend(ws.iter().map(|w| Arg::S(w.clone())));
        }
        if let Some(a) = artifact_id {
            sql.push_str(" AND artifact_id = ?");
            args.push(Arg::S(a.to_string()));
        }
        if let Some(k) = kind {
            sql.push_str(" AND kind = ?");
            args.push(Arg::S(k.to_string()));
        }
        if let Some(t) = since {
            sql.push_str(" AND created_at >= ?");
            args.push(Arg::S(t.to_string()));
        }
        sql.push_str(" ORDER BY created_at DESC LIMIT ?");
        args.push(Arg::I(if limit > 0 { limit.min(1_000) } else { 200 }));
        let mut q = sqlx::query(&sql);
        for a in &args {
            q = match a {
                Arg::S(s) => q.bind(s.as_str()),
                Arg::I(i) => q.bind(*i),
            };
        }
        let rows = q
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("design.signal.list"))?;
        rows.iter().map(row_signal).collect()
    }

    pub async fn insert_publish(&self, p: NewPublish) -> Result<DesignPublish> {
        let id = new_id();
        sqlx::query(
            "INSERT INTO design_publishes
             (id, artifact_id, version_id, target, url, pinned_set_json, created_by, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&p.artifact_id)
        .bind(&p.version_id)
        .bind(&p.target)
        .bind(&p.url)
        .bind(p.pinned_set.to_string())
        .bind(&p.created_by)
        .bind(now())
        .execute(&self.pool)
        .await
        .map_err(dberr("design.publish.insert"))?;
        let row = sqlx::query("SELECT * FROM design_publishes WHERE id = ?")
            .bind(&id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("design.publish.get"))?;
        row_publish(&row)
    }

    pub async fn list_publishes(&self, artifact_id: &str) -> Result<Vec<DesignPublish>> {
        let rows = sqlx::query(
            "SELECT * FROM design_publishes WHERE artifact_id = ? ORDER BY created_at DESC",
        )
        .bind(artifact_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("design.publish.list"))?;
        rows.iter().map(row_publish).collect()
    }

    // -- search ---------------------------------------------------------------

    /// Create the FTS5 index if this SQLite build supports it; `false` → the
    /// search falls back to LIKE. Idempotent. Created at runtime (not in the
    /// migration) so a build without FTS5 degrades instead of failing boot.
    pub async fn ensure_fts(&self) -> bool {
        sqlx::query(
            "CREATE VIRTUAL TABLE IF NOT EXISTS design_search_fts USING fts5(\
             artifact_id UNINDEXED, title, tags, body, story, project, \
             tokenize='porter unicode61')",
        )
        .execute(&self.pool)
        .await
        .is_ok()
    }

    async fn has_fts(&self) -> bool {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'design_search_fts'",
        )
        .fetch_one(&self.pool)
        .await
        .map(|n| n > 0)
        .unwrap_or(false)
    }

    /// (Re)index one artifact. Best-effort: a missing FTS table is a no-op.
    pub async fn fts_index(
        &self,
        artifact_id: &str,
        title: &str,
        tags: &str,
        body: &str,
        story: &str,
        project: &str,
    ) {
        self.fts_remove(artifact_id).await;
        let _ = sqlx::query(
            "INSERT INTO design_search_fts (artifact_id, title, tags, body, story, project)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(artifact_id)
        .bind(title)
        .bind(tags)
        .bind(body)
        .bind(story)
        .bind(project)
        .execute(&self.pool)
        .await;
    }

    pub async fn fts_remove(&self, artifact_id: &str) {
        let _ = sqlx::query("DELETE FROM design_search_fts WHERE artifact_id = ?")
            .bind(artifact_id)
            .execute(&self.pool)
            .await;
    }

    /// Search: FTS5 (bm25-ranked, snippets) when available, else a LIKE scan
    /// over titles + tags. Results are "shipped first", then by relevance.
    /// An empty/term-less query is a pure filter listing (newest first).
    pub async fn search(
        &self,
        query: &str,
        f: &ArtifactFilter,
    ) -> Result<Vec<(DesignArtifact, String, f64)>> {
        if f.workspaces.as_ref().is_some_and(|w| w.is_empty()) {
            return Ok(vec![]);
        }
        let (limit, offset) = f.page();
        let Some(mq) = fts_match(query) else {
            return Ok(self
                .list_artifacts(f)
                .await?
                .into_iter()
                .map(|a| (a, String::new(), 0.0))
                .collect());
        };
        let mut args = Vec::new();
        let sql = if self.has_fts().await {
            args.push(Arg::S(mq));
            let mut sql = String::from(concat!(
                "SELECT a.*, v.seq AS head_seq, ",
                art_enrich_cols!(),
                ",
                        snippet(design_search_fts, -1, '\u{2039}', '\u{203a}', '\u{2026}', 12) AS snip,
                        bm25(design_search_fts) AS rank
                 FROM design_search_fts
                 JOIN design_artifacts a ON a.id = design_search_fts.artifact_id
                 LEFT JOIN design_versions v ON v.id = a.head_version_id
                 WHERE design_search_fts MATCH ?"
            ));
            f.push_where(&mut sql, &mut args);
            sql.push_str(&format!(" ORDER BY {STATUS_ORDER}, rank LIMIT ? OFFSET ?"));
            sql
        } else {
            let needle = format!("%{}%", query.trim().replace('%', "\\%").replace('_', "\\_"));
            args.push(Arg::S(needle.clone()));
            args.push(Arg::S(needle));
            let mut sql = format!(
                "{ART_SELECT} WHERE (a.title LIKE ? ESCAPE '\\' OR a.tags_json LIKE ? ESCAPE '\\')"
            );
            f.push_where(&mut sql, &mut args);
            sql.push_str(&format!(
                " ORDER BY {STATUS_ORDER}, a.updated_at DESC LIMIT ? OFFSET ?"
            ));
            sql
        };
        args.push(Arg::I(limit));
        args.push(Arg::I(offset));
        let mut q = sqlx::query(&sql);
        for a in &args {
            q = match a {
                Arg::S(s) => q.bind(s.as_str()),
                Arg::I(i) => q.bind(*i),
            };
        }
        let rows = q
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("design.search"))?;
        rows.iter()
            .map(|r| {
                let a = row_artifact(r)?;
                let snip = r.try_get::<String, _>("snip").unwrap_or_default();
                let rank = r.try_get::<f64, _>("rank").unwrap_or(0.0);
                Ok((a, snip, -rank))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> Store {
        Store::new(otto_state::db::test_pool().await)
    }

    fn art(id: &str, ws: &str) -> NewArtifactRow {
        NewArtifactRow {
            id: id.into(),
            project_id: None,
            workspace_id: ws.into(),
            studio: "frames".into(),
            format: "html".into(),
            mime: "text/html".into(),
            title: format!("Artifact {id}"),
            status: "draft".into(),
            tags: vec!["hero".into()],
            meta: serde_json::json!({}),
            source_kind: None,
            source_id: None,
            created_by: "u1".into(),
            created_by_kind: "user".into(),
            created_session_id: None,
            created_at: None,
        }
    }

    fn ver(aid: &str, sha: &str) -> NewVersion {
        NewVersion {
            artifact_id: aid.into(),
            blob_sha256: sha.into(),
            size_bytes: 3,
            kind: "autosave".into(),
            branch: "main".into(),
            author_kind: "user".into(),
            author_id: "u1".into(),
            session_id: None,
            message: String::new(),
            provenance: serde_json::json!({}),
        }
    }

    fn link(src: &str, dst: &str, rel: &str, origin: &str) -> NewLink {
        NewLink {
            src_artifact_id: src.into(),
            src_version_id: None,
            src_node: String::new(),
            dst_kind: "artifact".into(),
            dst_id: dst.into(),
            dst_node: String::new(),
            rel: rel.into(),
            policy: "follow_approved".into(),
            pinned_version_id: None,
            origin: origin.into(),
            broken: false,
            meta: serde_json::json!({}),
            created_by: "u1".into(),
        }
    }

    #[tokio::test]
    async fn commit_moves_head_and_guards_the_base_version() {
        let s = store().await;
        s.insert_artifact(&art("A", "w1")).await.unwrap();
        let v1 = s.commit_version(ver("A", "s1"), Some("")).await.unwrap();
        assert_eq!(v1.seq, 1);
        assert_eq!(v1.parent_version_id, None);
        let v2 = s
            .commit_version(ver("A", "s2"), Some(&v1.id))
            .await
            .unwrap();
        assert_eq!(v2.seq, 2);
        assert_eq!(v2.parent_version_id.as_deref(), Some(v1.id.as_str()));
        let a = s.require_artifact("A").await.unwrap();
        assert_eq!(a.head_version_id.as_deref(), Some(v2.id.as_str()));
        assert_eq!(a.head_seq, Some(2));

        // A stale base is a 409 and leaves the head alone.
        let stale = s.commit_version(ver("A", "s3"), Some(&v1.id)).await;
        assert!(matches!(stale, Err(Error::Conflict(_))), "{stale:?}");
        assert_eq!(
            s.require_artifact("A").await.unwrap().head_version_id,
            Some(v2.id.clone())
        );
        // Unconditional saves always land.
        let v3 = s.commit_version(ver("A", "s3"), None).await.unwrap();
        assert_eq!(v3.seq, 3);
        // Missing artifact → NotFound.
        assert!(matches!(
            s.commit_version(ver("nope", "s1"), None).await,
            Err(Error::NotFound(_))
        ));
        let listed = s.list_versions("A", None, 0, 0).await.unwrap();
        assert_eq!(
            listed.iter().map(|v| v.seq).collect::<Vec<_>>(),
            vec![3, 2, 1]
        );
        assert_eq!(
            s.get_version_by_seq("A", 2).await.unwrap().unwrap().id,
            v2.id
        );
    }

    #[tokio::test]
    async fn source_key_is_unique_for_idempotent_import() {
        let s = store().await;
        let mut a = art("A", "w1");
        a.source_kind = Some("canvas_scene".into());
        a.source_id = Some("sc1".into());
        s.insert_artifact(&a).await.unwrap();
        let mut b = art("B", "w1");
        b.source_kind = Some("canvas_scene".into());
        b.source_id = Some("sc1".into());
        assert!(matches!(
            s.insert_artifact(&b).await,
            Err(Error::Conflict(_))
        ));
        let idx = s.source_index().await.unwrap();
        assert_eq!(idx.len(), 1);
        assert_eq!(
            s.find_by_source("canvas_scene", "sc1")
                .await
                .unwrap()
                .unwrap()
                .id,
            "A"
        );
    }

    #[tokio::test]
    async fn links_in_out_adjacency_and_hard_delete_marks_backlinks_broken() {
        let s = store().await;
        for id in ["A", "B", "C"] {
            s.insert_artifact(&art(id, "w1")).await.unwrap();
        }
        assert!(s
            .replace_extracted("A", &[link("A", "B", "embeds", "extracted")])
            .await
            .unwrap());
        s.insert_link(&link("B", "C", "embeds", "explicit"))
            .await
            .unwrap();
        assert!(matches!(
            s.insert_link(&link("B", "C", "embeds", "explicit")).await,
            Err(Error::Conflict(_))
        ));
        assert_eq!(s.links_out("A").await.unwrap().len(), 1);
        assert_eq!(s.links_in("C").await.unwrap().len(), 1);
        assert_eq!(s.link_counts("B").await.unwrap(), (1, 1));
        let adj = s.render_adjacency(&["A".to_string()], 100).await.unwrap();
        assert!(crate::graph::reaches(&adj, "A", "C", 100));

        // Re-extraction replaces (not appends).
        s.replace_extracted("A", &[link("A", "C", "references", "extracted")])
            .await
            .unwrap();
        let out = s.links_out("A").await.unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].dst_id, "C");

        s.delete_artifact("C").await.unwrap();
        let back = s.links_out("B").await.unwrap();
        assert!(back[0].broken, "incoming link flagged broken, not deleted");
    }

    #[tokio::test]
    async fn filters_workspaces_and_searches_with_fts() {
        let s = store().await;
        assert!(s.ensure_fts().await);
        s.insert_artifact(&art("A", "w1")).await.unwrap();
        s.insert_artifact(&art("B", "w2")).await.unwrap();
        s.fts_index(
            "A",
            "Rewards landing",
            "hero",
            "Win more with rewards",
            "LOY-142 Rewards",
            "Launch",
        )
        .await;
        s.fts_index("B", "Other", "", "nothing here", "", "").await;
        let f = ArtifactFilter {
            workspaces: Some(vec!["w1".into(), "w2".into()]),
            ..Default::default()
        };
        let hits = s.search("reward", &f).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0.id, "A");
        // Workspace filter applies to search.
        let only_w2 = ArtifactFilter {
            workspaces: Some(vec!["w2".into()]),
            ..Default::default()
        };
        assert!(s.search("reward", &only_w2).await.unwrap().is_empty());
        // No workspace access at all → nothing (and no SQL with `IN ()`).
        let none = ArtifactFilter {
            workspaces: Some(vec![]),
            ..Default::default()
        };
        assert!(s.list_artifacts(&none).await.unwrap().is_empty());
        // Filter-only listing.
        assert_eq!(s.search("", &f).await.unwrap().len(), 2);
        assert_eq!(s.known_workspaces().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn protected_versions_cover_head_approved_pins_publishes_and_signals() {
        let s = store().await;
        s.insert_artifact(&art("A", "w1")).await.unwrap();
        let v1 = s.commit_version(ver("A", "s1"), None).await.unwrap();
        let v2 = s.commit_version(ver("A", "s2"), None).await.unwrap();
        let v3 = s.commit_version(ver("A", "s3"), None).await.unwrap();
        let v4 = s.commit_version(ver("A", "s4"), None).await.unwrap();
        s.set_approved("A", &v1.id, "approved").await.unwrap();
        s.insert_publish(NewPublish {
            artifact_id: "A".into(),
            version_id: v2.id.clone(),
            target: "zip".into(),
            url: None,
            pinned_set: serde_json::json!([]),
            created_by: "u1".into(),
        })
        .await
        .unwrap();
        s.insert_signal(NewSignal {
            workspace_id: "w1".into(),
            artifact_id: "A".into(),
            version_id: Some(v3.id.clone()),
            kind: "variant_chosen".into(),
            actor_kind: "user".into(),
            actor_id: "u1".into(),
            session_id: None,
            payload: serde_json::json!({}),
        })
        .await
        .unwrap();
        let a = s.require_artifact("A").await.unwrap();
        let p = s.protected_versions(&a).await.unwrap();
        for v in [&v1, &v2, &v3, &v4] {
            assert!(p.contains(&v.id), "{} should be protected", v.seq);
        }
        assert!(s.blob_in_use("s1").await.unwrap());
        assert!(!s.blob_in_use("zz").await.unwrap());
    }

    #[test]
    fn fts_match_quotes_terms_and_prefixes_the_last() {
        assert_eq!(
            fts_match("hero card").as_deref(),
            Some("\"hero\" \"card\"*")
        );
        assert_eq!(fts_match("a: (x) -").as_deref(), None);
        assert_eq!(fts_match("LOY-142").as_deref(), Some("\"loy\" \"142\"*"));
    }

    /// Seed a `users` row (the enrichment joins resolve names from it).
    async fn user(s: &Store, id: &str, username: &str, display: &str) {
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, created_at)
             VALUES (?, ?, 'x', ?, '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(username)
        .bind(display)
        .execute(s.pool())
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn artifacts_and_versions_carry_resolved_people() {
        let s = store().await;
        user(&s, "u1", "ada", "Ada Lovelace").await;
        user(&s, "u2", "grace", "").await; // no display name → username
        s.insert_artifact(&art("A", "w1")).await.unwrap();
        // No version yet: creator resolved, no last editor.
        let a = s.require_artifact("A").await.unwrap();
        assert_eq!(a.created_by_name.as_deref(), Some("Ada Lovelace"));
        assert_eq!(a.last_editor_id, None);
        assert_eq!(a.last_editor_name, None);

        let v1 = s.commit_version(ver("A", "s1"), None).await.unwrap();
        assert_eq!(v1.author_name.as_deref(), Some("Ada Lovelace"));
        let mut by_grace = ver("A", "s2");
        by_grace.author_id = "u2".into();
        by_grace.author_kind = "agent".into();
        let v2 = s.commit_version(by_grace, None).await.unwrap();
        assert_eq!(v2.author_name.as_deref(), Some("grace"));

        // The artifact's last editor is the head's author, on every select.
        let a = s.require_artifact("A").await.unwrap();
        assert_eq!(a.last_editor_id.as_deref(), Some("u2"));
        assert_eq!(a.last_editor_kind.as_deref(), Some("agent"));
        assert_eq!(a.last_editor_name.as_deref(), Some("grace"));
        let listed = s.list_artifacts(&ArtifactFilter::default()).await.unwrap();
        assert_eq!(listed[0].last_editor_name.as_deref(), Some("grace"));
        assert_eq!(listed[0].created_by_name.as_deref(), Some("Ada Lovelace"));

        // Versions resolve their author on read; a system author stays null.
        let mut sys = ver("A", "s3");
        sys.author_id = "import".into();
        sys.author_kind = "system".into();
        s.commit_version(sys, None).await.unwrap();
        let vs = s.list_versions("A", None, 0, 0).await.unwrap();
        let names: Vec<Option<&str>> = vs.iter().map(|v| v.author_name.as_deref()).collect();
        assert_eq!(names, vec![None, Some("grace"), Some("Ada Lovelace")]);
        assert_eq!(
            s.get_version(&v1.id)
                .await
                .unwrap()
                .unwrap()
                .author_name
                .as_deref(),
            Some("Ada Lovelace")
        );
        assert_eq!(s.user_display_name("nobody").await, None);
    }

    #[tokio::test]
    async fn rows_carry_story_ids_and_page_by_cursor() {
        let s = store().await;
        for id in ["A", "B", "C", "D", "E"] {
            s.insert_artifact(&art(id, "w1")).await.unwrap();
        }
        let mut story = link("A", "S2", "implements", "explicit");
        story.dst_kind = "story".into();
        s.insert_link(&story).await.unwrap();
        story.dst_id = "S1".into();
        s.insert_link(&story).await.unwrap();
        let a = s.require_artifact("A").await.unwrap();
        assert_eq!(a.story_ids, vec!["S1".to_string(), "S2".to_string()]);
        assert!(s.require_artifact("B").await.unwrap().story_ids.is_empty());
        // The story filter and the per-row ids agree.
        let f = ArtifactFilter {
            story_id: Some("S1".into()),
            ..Default::default()
        };
        let by_story = s.list_artifacts(&f).await.unwrap();
        assert_eq!(by_story.len(), 1);
        assert_eq!(by_story[0].story_ids.len(), 2);

        // Keyset paging walks every row exactly once, newest first.
        let mut seen = Vec::new();
        let mut f = ArtifactFilter {
            limit: 2,
            ..Default::default()
        };
        loop {
            let page = s.list_artifacts(&f).await.unwrap();
            seen.extend(page.iter().map(|a| a.id.clone()));
            if (page.len() as i64) < f.effective_limit() {
                break;
            }
            // A client-built cursor (JSON spelling of updated_at) works too.
            let last = page.last().unwrap();
            let json_time = serde_json::to_value(last.updated_at).unwrap();
            let cursor = format!("{}|{}", json_time.as_str().unwrap(), last.id);
            assert_eq!(
                parse_cursor(&cursor).unwrap(),
                parse_cursor(&artifact_cursor(last)).unwrap()
            );
            f.cursor = Some(parse_cursor(&cursor).unwrap());
        }
        let mut sorted = seen.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted, vec!["A", "B", "C", "D", "E"], "{seen:?}");
        assert_eq!(seen.len(), 5, "{seen:?}");
        assert!(parse_cursor("nope").is_err());
        assert!(parse_cursor("2026-01-01T00:00:00Z|").is_err());
    }
}
