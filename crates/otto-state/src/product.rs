//! Product story analysis repository: stories, versions, analyses, questions,
//! notes, events, testcases, learnings.

use chrono::{DateTime, Utc};
use otto_core::{new_id, Id, Result};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, ts};

// ---------------------------------------------------------------------------
// Domain structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductStory {
    pub id: Id,
    pub workspace_id: Id,
    pub source_kind: String,
    pub account_id: Id,
    pub source_key: String,
    pub title: String,
    pub url: String,
    pub issue_type: Option<String>,
    pub stage: String,
    pub cwd: Option<String>,
    pub watch_enabled: bool,
    pub watch_cadence_min: i64,
    pub watch_cursor: Option<String>,
    pub confluence_tests_page_id: Option<String>,
    pub confluence_tests_url: Option<String>,
    pub tags: String,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductStoryVersion {
    pub id: Id,
    pub story_id: Id,
    pub version_no: i64,
    pub kind: String,
    pub title: String,
    pub body_md: String,
    pub raw_json: Option<String>,
    pub change_notes: Option<String>,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductAnalysis {
    pub id: Id,
    pub story_id: Id,
    pub source_version_id: Option<Id>,
    pub status: String,
    pub summary: String,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductAnalysisAgent {
    pub id: Id,
    pub analysis_id: Id,
    pub name: String,
    pub skill: String,
    pub provider: String,
    pub model: String,
    pub status: String,
    pub findings_json: Option<String>,
    pub error: Option<String>,
    /// SessionManager session id for this agent's live, openable terminal (set
    /// once the session is created, like a PR-review agent).
    pub session_id: Option<Id>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    /// How many times this agent has been auto-resumed after a daemon restart.
    /// Capped by the orphan reaper to avoid infinite resume loops.
    pub resume_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductQuestion {
    pub id: Id,
    pub story_id: Id,
    pub analysis_id: Option<Id>,
    pub text: String,
    pub rationale: String,
    pub category: String,
    pub status: String,
    pub answer: Option<String>,
    pub posted_ref: Option<String>,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductNote {
    pub id: Id,
    pub story_id: Id,
    pub section: Option<String>,
    pub body: String,
    pub author_id: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductEvent {
    pub id: Id,
    pub story_id: Id,
    pub section: String,
    pub kind: String,
    pub summary: String,
    pub actor_id: Option<Id>,
    pub meta_json: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductTestcaseRun {
    pub id: Id,
    pub story_id: Id,
    pub status: String,
    pub confluence_page_id: Option<String>,
    pub confluence_url: Option<String>,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductTestcase {
    pub id: Id,
    pub run_id: Id,
    pub story_id: Id,
    pub title: String,
    pub category: String,
    pub priority: String,
    pub steps_json: String,
    pub status: String,
    pub review_note: Option<String>,
    pub order_idx: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductLearning {
    pub id: Id,
    pub workspace_id: Id,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub tags: String,
    pub refs_json: String,
    pub source_story_id: Option<Id>,
    pub active: bool,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Input structs
// ---------------------------------------------------------------------------

pub struct NewStory {
    pub workspace_id: Id,
    pub source_kind: String,
    pub account_id: Id,
    pub source_key: String,
    pub title: String,
    pub url: String,
    pub issue_type: Option<String>,
    pub stage: String,
    pub cwd: Option<String>,
    pub created_by: Id,
}

#[derive(Default)]
pub struct StoryPatch {
    pub title: Option<String>,
    pub url: Option<String>,
    pub issue_type: Option<Option<String>>,
    pub stage: Option<String>,
    pub cwd: Option<Option<String>>,
    pub watch_enabled: Option<bool>,
    pub watch_cadence_min: Option<i64>,
    pub confluence_tests_page_id: Option<Option<String>>,
    pub confluence_tests_url: Option<Option<String>>,
    pub source_kind: Option<String>,
    pub account_id: Option<Id>,
    pub source_key: Option<String>,
    pub tags: Option<String>,
}

pub struct NewVersion {
    pub story_id: Id,
    pub kind: String,
    pub title: String,
    pub body_md: String,
    pub raw_json: Option<String>,
    pub change_notes: Option<String>,
    pub created_by: Id,
}

pub struct NewAnalysis {
    pub story_id: Id,
    pub source_version_id: Option<Id>,
    pub status: String,
    pub created_by: Id,
}

pub struct NewAnalysisAgent {
    pub analysis_id: Id,
    pub name: String,
    pub skill: String,
    pub provider: String,
    pub model: String,
    pub status: String,
    pub session_id: Option<Id>,
}

pub struct NewQuestion {
    pub story_id: Id,
    pub analysis_id: Option<Id>,
    pub text: String,
    pub rationale: String,
    pub category: String,
    pub created_by: Id,
}

pub struct QuestionPatch {
    pub text: Option<String>,
    pub rationale: Option<String>,
    pub category: Option<String>,
    pub status: Option<String>,
    pub answer: Option<Option<String>>,
    pub posted_ref: Option<Option<String>>,
}

pub struct NewNote {
    pub story_id: Id,
    pub section: Option<String>,
    pub body: String,
    pub author_id: Id,
}

pub struct NewEvent {
    pub story_id: Id,
    pub section: String,
    pub kind: String,
    pub summary: String,
    pub actor_id: Option<Id>,
    pub meta_json: Option<String>,
}

pub struct NewTestcase {
    pub run_id: Id,
    pub story_id: Id,
    pub title: String,
    pub category: String,
    pub priority: String,
    pub steps_json: String,
    pub order_idx: i64,
}

pub struct TestcasePatch {
    pub title: Option<String>,
    pub category: Option<String>,
    pub priority: Option<String>,
    pub steps_json: Option<String>,
    pub status: Option<String>,
    pub review_note: Option<Option<String>>,
    pub order_idx: Option<i64>,
}

pub struct NewLearning {
    pub workspace_id: Id,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub tags: String,
    pub refs_json: String,
    pub source_story_id: Option<Id>,
    pub created_by: Id,
}

pub struct LearningPatch {
    pub kind: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub tags: Option<String>,
    pub refs_json: Option<String>,
    pub active: Option<bool>,
}

// ---------------------------------------------------------------------------
// Transcript domain + input structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductTranscript {
    pub id: Id,
    pub story_id: Id,
    pub title: String,
    pub body: String,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
}

pub struct NewTranscript {
    pub story_id: Id,
    pub title: String,
    pub body: String,
    pub created_by: Id,
}

// ---------------------------------------------------------------------------
// Row mapping helpers
// ---------------------------------------------------------------------------

fn row_to_story(r: &sqlx::sqlite::SqliteRow) -> Result<ProductStory> {
    Ok(ProductStory {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        source_kind: r.get("source_kind"),
        account_id: r.get("account_id"),
        source_key: r.get("source_key"),
        title: r.get("title"),
        url: r.get("url"),
        issue_type: r.get("issue_type"),
        stage: r.get("stage"),
        cwd: r.get("cwd"),
        watch_enabled: r.get::<i64, _>("watch_enabled") != 0,
        watch_cadence_min: r.get("watch_cadence_min"),
        watch_cursor: r.get("watch_cursor"),
        confluence_tests_page_id: r.get("confluence_tests_page_id"),
        confluence_tests_url: r.get("confluence_tests_url"),
        tags: r.get::<Option<String>, _>("tags").unwrap_or_default(),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

fn row_to_version(r: &sqlx::sqlite::SqliteRow) -> Result<ProductStoryVersion> {
    Ok(ProductStoryVersion {
        id: r.get("id"),
        story_id: r.get("story_id"),
        version_no: r.get("version_no"),
        kind: r.get("kind"),
        title: r.get("title"),
        body_md: r.get("body_md"),
        raw_json: r.get("raw_json"),
        change_notes: r.get("change_notes"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

fn row_to_analysis(r: &sqlx::sqlite::SqliteRow) -> Result<ProductAnalysis> {
    let finished_at: Option<String> = r.get("finished_at");
    Ok(ProductAnalysis {
        id: r.get("id"),
        story_id: r.get("story_id"),
        source_version_id: r.get("source_version_id"),
        status: r.get("status"),
        summary: r.get("summary"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        finished_at: finished_at.as_deref().map(ts).transpose()?,
    })
}

fn row_to_agent(r: &sqlx::sqlite::SqliteRow) -> Result<ProductAnalysisAgent> {
    let started_at: Option<String> = r.get("started_at");
    let finished_at: Option<String> = r.get("finished_at");
    Ok(ProductAnalysisAgent {
        id: r.get("id"),
        analysis_id: r.get("analysis_id"),
        name: r.get("name"),
        skill: r.get("skill"),
        provider: r.get("provider"),
        model: r.get("model"),
        status: r.get("status"),
        findings_json: r.get("findings_json"),
        error: r.get("error"),
        session_id: r.get("session_id"),
        started_at: started_at.as_deref().map(ts).transpose()?,
        finished_at: finished_at.as_deref().map(ts).transpose()?,
        resume_count: r.get("resume_count"),
    })
}

fn row_to_question(r: &sqlx::sqlite::SqliteRow) -> Result<ProductQuestion> {
    Ok(ProductQuestion {
        id: r.get("id"),
        story_id: r.get("story_id"),
        analysis_id: r.get("analysis_id"),
        text: r.get("text"),
        rationale: r.get("rationale"),
        category: r.get("category"),
        status: r.get("status"),
        answer: r.get("answer"),
        posted_ref: r.get("posted_ref"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

fn row_to_note(r: &sqlx::sqlite::SqliteRow) -> Result<ProductNote> {
    Ok(ProductNote {
        id: r.get("id"),
        story_id: r.get("story_id"),
        section: r.get("section"),
        body: r.get("body"),
        author_id: r.get("author_id"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

fn row_to_event(r: &sqlx::sqlite::SqliteRow) -> Result<ProductEvent> {
    Ok(ProductEvent {
        id: r.get("id"),
        story_id: r.get("story_id"),
        section: r.get("section"),
        kind: r.get("kind"),
        summary: r.get("summary"),
        actor_id: r.get("actor_id"),
        meta_json: r.get("meta_json"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

fn row_to_tcrun(r: &sqlx::sqlite::SqliteRow) -> Result<ProductTestcaseRun> {
    Ok(ProductTestcaseRun {
        id: r.get("id"),
        story_id: r.get("story_id"),
        status: r.get("status"),
        confluence_page_id: r.get("confluence_page_id"),
        confluence_url: r.get("confluence_url"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

fn row_to_testcase(r: &sqlx::sqlite::SqliteRow) -> Result<ProductTestcase> {
    Ok(ProductTestcase {
        id: r.get("id"),
        run_id: r.get("run_id"),
        story_id: r.get("story_id"),
        title: r.get("title"),
        category: r.get("category"),
        priority: r.get("priority"),
        steps_json: r.get("steps_json"),
        status: r.get("status"),
        review_note: r.get("review_note"),
        order_idx: r.get("order_idx"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

fn row_to_learning(r: &sqlx::sqlite::SqliteRow) -> Result<ProductLearning> {
    Ok(ProductLearning {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        kind: r.get("kind"),
        title: r.get("title"),
        body: r.get("body"),
        tags: r.get("tags"),
        refs_json: r.get("refs_json"),
        source_story_id: r.get("source_story_id"),
        active: r.get::<i64, _>("active") != 0,
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

fn row_to_transcript(r: &sqlx::sqlite::SqliteRow) -> Result<ProductTranscript> {
    Ok(ProductTranscript {
        id: r.get("id"),
        story_id: r.get("story_id"),
        title: r.get("title"),
        body: r.get("body"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

// ---------------------------------------------------------------------------
// Repo
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ProductRepo {
    pool: SqlitePool,
}

impl ProductRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // -----------------------------------------------------------------------
    // Stories
    // -----------------------------------------------------------------------

    pub async fn create_story(&self, s: NewStory) -> Result<ProductStory> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO product_stories
             (id, workspace_id, source_kind, account_id, source_key, title, url, issue_type,
              stage, cwd, watch_enabled, watch_cadence_min, watch_cursor,
              confluence_tests_page_id, confluence_tests_url, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, 15, NULL, NULL, NULL, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&s.workspace_id)
        .bind(&s.source_kind)
        .bind(&s.account_id)
        .bind(&s.source_key)
        .bind(&s.title)
        .bind(&s.url)
        .bind(&s.issue_type)
        .bind(&s.stage)
        .bind(&s.cwd)
        .bind(&s.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create story"))?;
        self.get_story(&id).await
    }

    pub async fn get_story(&self, id: &Id) -> Result<ProductStory> {
        let row = sqlx::query("SELECT * FROM product_stories WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get story"))?;
        row_to_story(&row)
    }

    pub async fn list_stories(&self, ws: &Id) -> Result<Vec<ProductStory>> {
        let rows = sqlx::query(
            "SELECT * FROM product_stories WHERE workspace_id = ? ORDER BY created_at DESC",
        )
        .bind(ws)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list stories"))?;
        rows.iter().map(row_to_story).collect()
    }

    pub async fn update_story(&self, id: &Id, p: StoryPatch) -> Result<ProductStory> {
        let existing = self.get_story(id).await?;
        let title = p.title.as_deref().unwrap_or(&existing.title);
        let url = p.url.as_deref().unwrap_or(&existing.url);
        let issue_type = match p.issue_type {
            Some(v) => v,
            None => existing.issue_type.clone(),
        };
        let stage = p.stage.as_deref().unwrap_or(&existing.stage);
        let cwd = match p.cwd {
            Some(v) => v,
            None => existing.cwd.clone(),
        };
        let watch_enabled = p.watch_enabled.unwrap_or(existing.watch_enabled);
        let watch_cadence_min = p.watch_cadence_min.unwrap_or(existing.watch_cadence_min);
        let confluence_tests_page_id = match p.confluence_tests_page_id {
            Some(v) => v,
            None => existing.confluence_tests_page_id.clone(),
        };
        let confluence_tests_url = match p.confluence_tests_url {
            Some(v) => v,
            None => existing.confluence_tests_url.clone(),
        };
        let source_kind = p.source_kind.as_deref().unwrap_or(&existing.source_kind);
        let account_id = p.account_id.as_ref().unwrap_or(&existing.account_id);
        let source_key = p.source_key.as_deref().unwrap_or(&existing.source_key);
        let tags = p.tags.as_deref().unwrap_or(&existing.tags);
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE product_stories
             SET title = ?, url = ?, issue_type = ?, stage = ?, cwd = ?,
                 watch_enabled = ?, watch_cadence_min = ?,
                 confluence_tests_page_id = ?, confluence_tests_url = ?,
                 source_kind = ?, account_id = ?, source_key = ?,
                 tags = ?,
                 updated_at = ?
             WHERE id = ?",
        )
        .bind(title)
        .bind(url)
        .bind(&issue_type)
        .bind(stage)
        .bind(&cwd)
        .bind(i64::from(watch_enabled))
        .bind(watch_cadence_min)
        .bind(&confluence_tests_page_id)
        .bind(&confluence_tests_url)
        .bind(source_kind)
        .bind(account_id)
        .bind(source_key)
        .bind(tags)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update story"))?;
        self.get_story(id).await
    }

    pub async fn delete_story(&self, id: &Id) -> Result<()> {
        // Delete child rows first (no FK cascade in SQLite without PRAGMA)
        // testcases depend on testcase_runs; delete testcases by story_id first
        sqlx::query("DELETE FROM product_testcases WHERE story_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete story testcases"))?;
        sqlx::query("DELETE FROM product_testcase_runs WHERE story_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete story testcase_runs"))?;
        // analysis_agents depend on analyses; delete agents via analyses
        sqlx::query(
            "DELETE FROM product_analysis_agents
             WHERE analysis_id IN (SELECT id FROM product_analyses WHERE story_id = ?)",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("delete story analysis_agents"))?;
        sqlx::query("DELETE FROM product_analyses WHERE story_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete story analyses"))?;
        sqlx::query("DELETE FROM product_story_versions WHERE story_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete story versions"))?;
        sqlx::query("DELETE FROM product_questions WHERE story_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete story questions"))?;
        sqlx::query("DELETE FROM product_notes WHERE story_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete story notes"))?;
        sqlx::query("DELETE FROM product_events WHERE story_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete story events"))?;
        sqlx::query("DELETE FROM product_transcripts WHERE story_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete story transcripts"))?;
        sqlx::query("DELETE FROM product_stories WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete story"))?;
        Ok(())
    }

    pub async fn list_watching(&self) -> Result<Vec<ProductStory>> {
        let rows = sqlx::query(
            "SELECT * FROM product_stories WHERE watch_enabled = 1 ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list watching"))?;
        rows.iter().map(row_to_story).collect()
    }

    pub async fn set_watch_cursor(&self, id: &Id, cursor: &str) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE product_stories SET watch_cursor = ?, updated_at = ? WHERE id = ?",
        )
        .bind(cursor)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set watch cursor"))?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Versions
    // -----------------------------------------------------------------------

    pub async fn add_version(&self, v: NewVersion) -> Result<ProductStoryVersion> {
        let id = new_id();
        let now = fmt(Utc::now());
        // Compute next version_no
        let next_no: i64 = sqlx::query(
            "SELECT COALESCE(MAX(version_no), 0) + 1 FROM product_story_versions WHERE story_id = ?",
        )
        .bind(&v.story_id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("compute version_no"))?
        .get::<i64, _>(0);

        sqlx::query(
            "INSERT INTO product_story_versions
             (id, story_id, version_no, kind, title, body_md, raw_json, change_notes,
              created_by, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&v.story_id)
        .bind(next_no)
        .bind(&v.kind)
        .bind(&v.title)
        .bind(&v.body_md)
        .bind(&v.raw_json)
        .bind(&v.change_notes)
        .bind(&v.created_by)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("add version"))?;
        self.get_version(&id).await
    }

    /// List versions for a story; `body_md` is omitted (empty string) for brevity.
    pub async fn list_versions(&self, story: &Id) -> Result<Vec<ProductStoryVersion>> {
        let rows = sqlx::query(
            "SELECT id, story_id, version_no, kind, title, '' AS body_md, raw_json,
                    change_notes, created_by, created_at
             FROM product_story_versions WHERE story_id = ? ORDER BY version_no DESC",
        )
        .bind(story)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list versions"))?;
        rows.iter().map(row_to_version).collect()
    }

    pub async fn get_version(&self, id: &Id) -> Result<ProductStoryVersion> {
        let row = sqlx::query("SELECT * FROM product_story_versions WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get version"))?;
        row_to_version(&row)
    }

    pub async fn latest_source_version(
        &self,
        story: &Id,
    ) -> Result<Option<ProductStoryVersion>> {
        let row = sqlx::query(
            "SELECT * FROM product_story_versions
             WHERE story_id = ? AND kind = 'source'
             ORDER BY version_no DESC LIMIT 1",
        )
        .bind(story)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("latest source version"))?;
        row.as_ref().map(row_to_version).transpose()
    }

    /// Newest `kind='plan'` version for a story (full row, including `body_md`),
    /// or `None` when the story has no plan yet.
    pub async fn latest_plan_version(
        &self,
        story: &Id,
    ) -> Result<Option<ProductStoryVersion>> {
        let row = sqlx::query(
            "SELECT * FROM product_story_versions
             WHERE story_id = ? AND kind = 'plan'
             ORDER BY version_no DESC LIMIT 1",
        )
        .bind(story)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("latest plan version"))?;
        row.as_ref().map(row_to_version).transpose()
    }

    // -----------------------------------------------------------------------
    // Analyses
    // -----------------------------------------------------------------------

    pub async fn create_analysis(&self, a: NewAnalysis) -> Result<ProductAnalysis> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO product_analyses
             (id, story_id, source_version_id, status, summary, created_by, created_at, finished_at)
             VALUES (?, ?, ?, ?, '', ?, ?, NULL)",
        )
        .bind(&id)
        .bind(&a.story_id)
        .bind(&a.source_version_id)
        .bind(&a.status)
        .bind(&a.created_by)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create analysis"))?;
        self.get_analysis(&id).await
    }

    pub async fn set_analysis_status(
        &self,
        id: &Id,
        status: &str,
        summary: Option<&str>,
        finished: bool,
    ) -> Result<()> {
        let existing = self.get_analysis(id).await?;
        let summary = summary.unwrap_or(&existing.summary);
        let finished_at = if finished {
            Some(fmt(Utc::now()))
        } else {
            existing.finished_at.map(fmt)
        };
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE product_analyses SET status = ?, summary = ?, finished_at = ?,
             created_at = created_at WHERE id = ?",
        )
        .bind(status)
        .bind(summary)
        .bind(&finished_at)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set analysis status"))?;
        let _ = now;
        Ok(())
    }

    pub async fn list_analyses(&self, story: &Id) -> Result<Vec<ProductAnalysis>> {
        let rows = sqlx::query(
            "SELECT * FROM product_analyses WHERE story_id = ? ORDER BY created_at DESC",
        )
        .bind(story)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list analyses"))?;
        rows.iter().map(row_to_analysis).collect()
    }

    pub async fn get_analysis(&self, id: &Id) -> Result<ProductAnalysis> {
        let row = sqlx::query("SELECT * FROM product_analyses WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get analysis"))?;
        row_to_analysis(&row)
    }

    pub async fn add_analysis_agent(&self, ag: NewAnalysisAgent) -> Result<ProductAnalysisAgent> {
        let id = new_id();
        sqlx::query(
            "INSERT INTO product_analysis_agents
             (id, analysis_id, name, skill, provider, model, status,
              findings_json, error, session_id, started_at, finished_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, NULL, NULL, ?, NULL, NULL)",
        )
        .bind(&id)
        .bind(&ag.analysis_id)
        .bind(&ag.name)
        .bind(&ag.skill)
        .bind(&ag.provider)
        .bind(&ag.model)
        .bind(&ag.status)
        .bind(&ag.session_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("add analysis agent"))?;
        self.get_analysis_agent(&id).await
    }

    /// Record the live SessionManager session id for an analysis agent so the
    /// UI can Open its terminal (mirrors a PR-review agent's `session_id`).
    pub async fn set_agent_session(&self, id: &Id, session_id: &Id) -> Result<()> {
        sqlx::query("UPDATE product_analysis_agents SET session_id = ? WHERE id = ?")
            .bind(session_id)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set agent session"))?;
        Ok(())
    }

    pub async fn get_analysis_agent(&self, id: &Id) -> Result<ProductAnalysisAgent> {
        let row = sqlx::query("SELECT * FROM product_analysis_agents WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get analysis agent"))?;
        row_to_agent(&row)
    }

    pub async fn set_agent_status(
        &self,
        id: &Id,
        status: &str,
        findings: Option<&str>,
        error: Option<&str>,
        finished: bool,
    ) -> Result<()> {
        let existing = self.get_analysis_agent(id).await?;
        let findings_json = findings.map(str::to_string).or(existing.findings_json);
        let error_val = error.map(str::to_string).or(existing.error);
        let started_at = existing.started_at.map(fmt);
        let finished_at = if finished {
            Some(fmt(Utc::now()))
        } else {
            existing.finished_at.map(fmt)
        };
        sqlx::query(
            "UPDATE product_analysis_agents
             SET status = ?, findings_json = ?, error = ?, started_at = ?, finished_at = ?
             WHERE id = ?",
        )
        .bind(status)
        .bind(&findings_json)
        .bind(&error_val)
        .bind(&started_at)
        .bind(&finished_at)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set agent status"))?;
        Ok(())
    }

    pub async fn list_analysis_agents(
        &self,
        analysis: &Id,
    ) -> Result<Vec<ProductAnalysisAgent>> {
        let rows = sqlx::query(
            "SELECT * FROM product_analysis_agents WHERE analysis_id = ? ORDER BY rowid",
        )
        .bind(analysis)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list analysis agents"))?;
        rows.iter().map(row_to_agent).collect()
    }

    /// All agents still in a non-terminal state (`running`/`waiting`). Used by the
    /// startup orphan reaper — after a restart these have no surviving task.
    pub async fn list_unfinished_agents(&self) -> Result<Vec<ProductAnalysisAgent>> {
        let rows = sqlx::query(
            "SELECT * FROM product_analysis_agents \
             WHERE status IN ('running', 'waiting') ORDER BY rowid",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list unfinished agents"))?;
        rows.iter().map(row_to_agent).collect()
    }

    /// Increment an agent's auto-resume counter (orphan reaper loop guard).
    pub async fn bump_resume_count(&self, id: &Id) -> Result<()> {
        sqlx::query(
            "UPDATE product_analysis_agents SET resume_count = resume_count + 1 WHERE id = ?",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("bump resume count"))?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Questions
    // -----------------------------------------------------------------------

    pub async fn create_question(&self, q: NewQuestion) -> Result<ProductQuestion> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO product_questions
             (id, story_id, analysis_id, text, rationale, category, status,
              answer, posted_ref, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, 'open', NULL, NULL, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&q.story_id)
        .bind(&q.analysis_id)
        .bind(&q.text)
        .bind(&q.rationale)
        .bind(&q.category)
        .bind(&q.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create question"))?;
        self.get_question(&id).await
    }

    pub async fn list_questions(&self, story: &Id) -> Result<Vec<ProductQuestion>> {
        let rows = sqlx::query(
            "SELECT * FROM product_questions WHERE story_id = ? ORDER BY created_at",
        )
        .bind(story)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list questions"))?;
        rows.iter().map(row_to_question).collect()
    }

    pub async fn get_question(&self, id: &Id) -> Result<ProductQuestion> {
        let row = sqlx::query("SELECT * FROM product_questions WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get question"))?;
        row_to_question(&row)
    }

    pub async fn update_question(&self, id: &Id, p: QuestionPatch) -> Result<ProductQuestion> {
        let existing = self.get_question(id).await?;
        let text = p.text.as_deref().unwrap_or(&existing.text);
        let rationale = p.rationale.as_deref().unwrap_or(&existing.rationale);
        let category = p.category.as_deref().unwrap_or(&existing.category);
        let status = p.status.as_deref().unwrap_or(&existing.status);
        let answer = match p.answer {
            Some(v) => v,
            None => existing.answer.clone(),
        };
        let posted_ref = match p.posted_ref {
            Some(v) => v,
            None => existing.posted_ref.clone(),
        };
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE product_questions
             SET text = ?, rationale = ?, category = ?, status = ?,
                 answer = ?, posted_ref = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(text)
        .bind(rationale)
        .bind(category)
        .bind(status)
        .bind(&answer)
        .bind(&posted_ref)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update question"))?;
        self.get_question(id).await
    }

    pub async fn delete_question(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM product_questions WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete question"))?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Notes
    // -----------------------------------------------------------------------

    pub async fn create_note(&self, n: NewNote) -> Result<ProductNote> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO product_notes (id, story_id, section, body, author_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&n.story_id)
        .bind(&n.section)
        .bind(&n.body)
        .bind(&n.author_id)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create note"))?;
        self.get_note(&id).await
    }

    pub async fn list_notes(&self, story: &Id) -> Result<Vec<ProductNote>> {
        let rows = sqlx::query(
            "SELECT * FROM product_notes WHERE story_id = ? ORDER BY created_at",
        )
        .bind(story)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list notes"))?;
        rows.iter().map(row_to_note).collect()
    }

    pub async fn get_note(&self, id: &Id) -> Result<ProductNote> {
        let row = sqlx::query("SELECT * FROM product_notes WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get note"))?;
        row_to_note(&row)
    }

    pub async fn update_note(&self, id: &Id, body: &str) -> Result<ProductNote> {
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE product_notes SET body = ?, updated_at = ? WHERE id = ?",
        )
        .bind(body)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update note"))?;
        self.get_note(id).await
    }

    pub async fn delete_note(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM product_notes WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete note"))?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Events
    // -----------------------------------------------------------------------

    pub async fn add_event(&self, e: NewEvent) -> Result<ProductEvent> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO product_events
             (id, story_id, section, kind, summary, actor_id, meta_json, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&e.story_id)
        .bind(&e.section)
        .bind(&e.kind)
        .bind(&e.summary)
        .bind(&e.actor_id)
        .bind(&e.meta_json)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("add event"))?;
        let row = sqlx::query("SELECT * FROM product_events WHERE id = ?")
            .bind(&id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get event"))?;
        row_to_event(&row)
    }

    pub async fn list_events(
        &self,
        story: &Id,
        section: Option<&str>,
    ) -> Result<Vec<ProductEvent>> {
        let rows = if let Some(sec) = section {
            sqlx::query(
                "SELECT * FROM product_events
                 WHERE story_id = ? AND section = ? ORDER BY created_at",
            )
            .bind(story)
            .bind(sec)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list events by section"))?
        } else {
            sqlx::query(
                "SELECT * FROM product_events WHERE story_id = ? ORDER BY created_at",
            )
            .bind(story)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list events"))?
        };
        rows.iter().map(row_to_event).collect()
    }

    // -----------------------------------------------------------------------
    // Testcase Runs
    // -----------------------------------------------------------------------

    pub async fn create_testcase_run(
        &self,
        story: &Id,
        by: &Id,
    ) -> Result<ProductTestcaseRun> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO product_testcase_runs
             (id, story_id, status, confluence_page_id, confluence_url, created_by, created_at)
             VALUES (?, ?, 'draft', NULL, NULL, ?, ?)",
        )
        .bind(&id)
        .bind(story)
        .bind(by)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create testcase run"))?;
        self.get_testcase_run(&id).await
    }

    pub async fn list_testcase_runs(&self, story: &Id) -> Result<Vec<ProductTestcaseRun>> {
        let rows = sqlx::query(
            "SELECT * FROM product_testcase_runs WHERE story_id = ? ORDER BY created_at DESC",
        )
        .bind(story)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list testcase runs"))?;
        rows.iter().map(row_to_tcrun).collect()
    }

    pub async fn get_testcase_run(&self, id: &Id) -> Result<ProductTestcaseRun> {
        let row = sqlx::query("SELECT * FROM product_testcase_runs WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get testcase run"))?;
        row_to_tcrun(&row)
    }

    pub async fn set_testcase_run(
        &self,
        id: &Id,
        status: Option<&str>,
        page_id: Option<&str>,
        url: Option<&str>,
    ) -> Result<ProductTestcaseRun> {
        let existing = self.get_testcase_run(id).await?;
        let status = status.unwrap_or(&existing.status);
        let page_id = page_id
            .map(str::to_string)
            .or(existing.confluence_page_id.clone());
        let url = url.map(str::to_string).or(existing.confluence_url.clone());
        sqlx::query(
            "UPDATE product_testcase_runs
             SET status = ?, confluence_page_id = ?, confluence_url = ?
             WHERE id = ?",
        )
        .bind(status)
        .bind(&page_id)
        .bind(&url)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set testcase run"))?;
        self.get_testcase_run(id).await
    }

    // -----------------------------------------------------------------------
    // Testcases
    // -----------------------------------------------------------------------

    pub async fn add_testcase(&self, t: NewTestcase) -> Result<ProductTestcase> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO product_testcases
             (id, run_id, story_id, title, category, priority, steps_json, status,
              review_note, order_idx, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, 'draft', NULL, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&t.run_id)
        .bind(&t.story_id)
        .bind(&t.title)
        .bind(&t.category)
        .bind(&t.priority)
        .bind(&t.steps_json)
        .bind(t.order_idx)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("add testcase"))?;
        self.get_testcase(&id).await
    }

    pub async fn list_testcases(&self, run: &Id) -> Result<Vec<ProductTestcase>> {
        let rows = sqlx::query(
            "SELECT * FROM product_testcases WHERE run_id = ? ORDER BY order_idx, created_at",
        )
        .bind(run)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list testcases"))?;
        rows.iter().map(row_to_testcase).collect()
    }

    pub async fn get_testcase(&self, id: &Id) -> Result<ProductTestcase> {
        let row = sqlx::query("SELECT * FROM product_testcases WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get testcase"))?;
        row_to_testcase(&row)
    }

    pub async fn update_testcase(&self, id: &Id, p: TestcasePatch) -> Result<ProductTestcase> {
        let existing = self.get_testcase(id).await?;
        let title = p.title.as_deref().unwrap_or(&existing.title);
        let category = p.category.as_deref().unwrap_or(&existing.category);
        let priority = p.priority.as_deref().unwrap_or(&existing.priority);
        let steps_json = p.steps_json.as_deref().unwrap_or(&existing.steps_json);
        let status = p.status.as_deref().unwrap_or(&existing.status);
        let review_note = match p.review_note {
            Some(v) => v,
            None => existing.review_note.clone(),
        };
        let order_idx = p.order_idx.unwrap_or(existing.order_idx);
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE product_testcases
             SET title = ?, category = ?, priority = ?, steps_json = ?,
                 status = ?, review_note = ?, order_idx = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(title)
        .bind(category)
        .bind(priority)
        .bind(steps_json)
        .bind(status)
        .bind(&review_note)
        .bind(order_idx)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update testcase"))?;
        self.get_testcase(id).await
    }

    pub async fn approve_run_testcases(&self, run: &Id) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE product_testcases SET status = 'approved', updated_at = ?
             WHERE run_id = ? AND status = 'draft'",
        )
        .bind(&now)
        .bind(run)
        .execute(&self.pool)
        .await
        .map_err(dberr("approve run testcases"))?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Learnings
    // -----------------------------------------------------------------------

    pub async fn create_learning(&self, l: NewLearning) -> Result<ProductLearning> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO product_learnings
             (id, workspace_id, kind, title, body, tags, refs_json, source_story_id,
              active, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&l.workspace_id)
        .bind(&l.kind)
        .bind(&l.title)
        .bind(&l.body)
        .bind(&l.tags)
        .bind(&l.refs_json)
        .bind(&l.source_story_id)
        .bind(&l.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create learning"))?;
        self.get_learning(&id).await
    }

    pub async fn list_learnings(
        &self,
        ws: &Id,
        active_only: bool,
    ) -> Result<Vec<ProductLearning>> {
        let rows = if active_only {
            sqlx::query(
                "SELECT * FROM product_learnings
                 WHERE workspace_id = ? AND active = 1 ORDER BY created_at DESC",
            )
            .bind(ws)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list learnings active"))?
        } else {
            sqlx::query(
                "SELECT * FROM product_learnings WHERE workspace_id = ? ORDER BY created_at DESC",
            )
            .bind(ws)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list learnings"))?
        };
        rows.iter().map(row_to_learning).collect()
    }

    pub async fn get_learning(&self, id: &Id) -> Result<ProductLearning> {
        let row = sqlx::query("SELECT * FROM product_learnings WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get learning"))?;
        row_to_learning(&row)
    }

    pub async fn update_learning(&self, id: &Id, p: LearningPatch) -> Result<ProductLearning> {
        let existing = self.get_learning(id).await?;
        let kind = p.kind.as_deref().unwrap_or(&existing.kind);
        let title = p.title.as_deref().unwrap_or(&existing.title);
        let body = p.body.as_deref().unwrap_or(&existing.body);
        let tags = p.tags.as_deref().unwrap_or(&existing.tags);
        let refs_json = p.refs_json.as_deref().unwrap_or(&existing.refs_json);
        let active = p.active.unwrap_or(existing.active);
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE product_learnings
             SET kind = ?, title = ?, body = ?, tags = ?, refs_json = ?, active = ?,
                 updated_at = ?
             WHERE id = ?",
        )
        .bind(kind)
        .bind(title)
        .bind(body)
        .bind(tags)
        .bind(refs_json)
        .bind(i64::from(active))
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update learning"))?;
        self.get_learning(id).await
    }

    pub async fn delete_learning(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM product_learnings WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete learning"))?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Transcripts
    // -----------------------------------------------------------------------

    pub async fn create_transcript(&self, t: NewTranscript) -> Result<ProductTranscript> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO product_transcripts (id, story_id, title, body, created_by, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&t.story_id)
        .bind(&t.title)
        .bind(&t.body)
        .bind(&t.created_by)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create transcript"))?;
        self.get_transcript(&id).await
    }

    pub async fn list_transcripts(&self, story: &Id) -> Result<Vec<ProductTranscript>> {
        let rows = sqlx::query(
            "SELECT * FROM product_transcripts WHERE story_id = ? ORDER BY created_at DESC",
        )
        .bind(story)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list transcripts"))?;
        rows.iter().map(row_to_transcript).collect()
    }

    pub async fn get_transcript(&self, id: &Id) -> Result<ProductTranscript> {
        let row = sqlx::query("SELECT * FROM product_transcripts WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get transcript"))?;
        row_to_transcript(&row)
    }

    pub async fn delete_transcript(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM product_transcripts WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete transcript"))?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Version body edit (for in-place draft editing)
    // -----------------------------------------------------------------------

    /// Edit a version's title and body in place. Intended for the single
    /// `kind='draft'` version of a discovery story; returns the updated row.
    pub async fn update_version_body(
        &self,
        version_id: &Id,
        title: &str,
        body_md: &str,
    ) -> Result<ProductStoryVersion> {
        sqlx::query(
            "UPDATE product_story_versions SET title = ?, body_md = ? WHERE id = ?",
        )
        .bind(title)
        .bind(body_md)
        .bind(version_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update version body"))?;
        self.get_version(version_id).await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_pool() -> SqlitePool {
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

    /// Seed a minimal user record so FK constraints are satisfied.
    async fn seed_user(pool: &SqlitePool) -> Id {
        let uid = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
             VALUES (?, ?, ?, ?, 0, ?)",
        )
        .bind(&uid)
        .bind("testuser")
        .bind("hash")
        .bind("Test User")
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        uid
    }

    /// Seed a minimal workspace.
    async fn seed_workspace(pool: &SqlitePool, _user_id: &Id) -> Id {
        let wid = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, created_at)
             VALUES (?, ?, ?, ?)",
        )
        .bind(&wid)
        .bind("ws")
        .bind("/tmp")
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        wid
    }

    fn new_story_input(ws: &Id, user: &Id) -> NewStory {
        NewStory {
            workspace_id: ws.clone(),
            source_kind: "jira".into(),
            account_id: user.clone(),
            source_key: "PROJ-1".into(),
            title: "My Story".into(),
            url: "https://example.com".into(),
            issue_type: Some("Story".into()),
            stage: "imported".into(),
            cwd: None,
            created_by: user.clone(),
        }
    }

    // -----------------------------------------------------------------------
    // create_story / get_story / list_stories
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn create_and_get_story_round_trips_all_fields() {
        let pool = mem_pool().await;
        let repo = ProductRepo::new(pool.clone());
        let user = seed_user(&pool).await;
        let ws = seed_workspace(&pool, &user).await;

        let s = repo.create_story(new_story_input(&ws, &user)).await.unwrap();

        assert_eq!(s.workspace_id, ws);
        assert_eq!(s.source_kind, "jira");
        assert_eq!(s.source_key, "PROJ-1");
        assert_eq!(s.title, "My Story");
        assert_eq!(s.url, "https://example.com");
        assert_eq!(s.issue_type, Some("Story".into()));
        assert_eq!(s.stage, "imported");
        assert!(!s.watch_enabled);
        assert_eq!(s.watch_cadence_min, 15);
        assert!(s.watch_cursor.is_none());

        let fetched = repo.get_story(&s.id).await.unwrap();
        assert_eq!(fetched.id, s.id);
        assert_eq!(fetched.title, s.title);
    }

    #[tokio::test]
    async fn list_stories_filters_by_workspace() {
        let pool = mem_pool().await;
        let repo = ProductRepo::new(pool.clone());
        let user = seed_user(&pool).await;
        let ws1 = seed_workspace(&pool, &user).await;
        let ws2 = seed_workspace(&pool, &user).await;

        repo.create_story(new_story_input(&ws1, &user)).await.unwrap();
        repo.create_story(new_story_input(&ws1, &user)).await.unwrap();
        repo.create_story(new_story_input(&ws2, &user)).await.unwrap();

        let list1 = repo.list_stories(&ws1).await.unwrap();
        let list2 = repo.list_stories(&ws2).await.unwrap();
        assert_eq!(list1.len(), 2);
        assert_eq!(list2.len(), 1);
    }

    // -----------------------------------------------------------------------
    // add_version / version_no auto-increment
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn add_version_auto_increments_version_no() {
        let pool = mem_pool().await;
        let repo = ProductRepo::new(pool.clone());
        let user = seed_user(&pool).await;
        let ws = seed_workspace(&pool, &user).await;
        let story = repo.create_story(new_story_input(&ws, &user)).await.unwrap();

        let v1 = repo
            .add_version(NewVersion {
                story_id: story.id.clone(),
                kind: "source".into(),
                title: "v1".into(),
                body_md: "# v1".into(),
                raw_json: None,
                change_notes: None,
                created_by: user.clone(),
            })
            .await
            .unwrap();

        let v2 = repo
            .add_version(NewVersion {
                story_id: story.id.clone(),
                kind: "suggested".into(),
                title: "v2".into(),
                body_md: "# v2".into(),
                raw_json: None,
                change_notes: Some("improved".into()),
                created_by: user.clone(),
            })
            .await
            .unwrap();

        assert_eq!(v1.version_no, 1);
        assert_eq!(v2.version_no, 2);
    }

    // -----------------------------------------------------------------------
    // update_question patches selected fields, leaves others unchanged
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn update_question_patches_status_and_answer_leaves_text_unchanged() {
        let pool = mem_pool().await;
        let repo = ProductRepo::new(pool.clone());
        let user = seed_user(&pool).await;
        let ws = seed_workspace(&pool, &user).await;
        let story = repo.create_story(new_story_input(&ws, &user)).await.unwrap();

        let q = repo
            .create_question(NewQuestion {
                story_id: story.id.clone(),
                analysis_id: None,
                text: "What is the acceptance criterion?".into(),
                rationale: "Need clarity".into(),
                category: "requirements".into(),
                created_by: user.clone(),
            })
            .await
            .unwrap();

        let updated = repo
            .update_question(
                &q.id,
                QuestionPatch {
                    text: None, // keep unchanged
                    rationale: None,
                    category: None,
                    status: Some("answered".into()),
                    answer: Some(Some("The criterion is X".into())),
                    posted_ref: None,
                },
            )
            .await
            .unwrap();

        // text unchanged
        assert_eq!(updated.text, "What is the acceptance criterion?");
        // status patched
        assert_eq!(updated.status, "answered");
        // answer patched
        assert_eq!(updated.answer, Some("The criterion is X".into()));
    }

    // -----------------------------------------------------------------------
    // approve_run_testcases flips only that run's draft cases
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn approve_run_testcases_flips_only_that_runs_draft_cases() {
        let pool = mem_pool().await;
        let repo = ProductRepo::new(pool.clone());
        let user = seed_user(&pool).await;
        let ws = seed_workspace(&pool, &user).await;
        let story = repo.create_story(new_story_input(&ws, &user)).await.unwrap();

        let run_a = repo.create_testcase_run(&story.id, &user).await.unwrap();
        let run_b = repo.create_testcase_run(&story.id, &user).await.unwrap();

        // Add testcases to run_a
        let tc_a1 = repo
            .add_testcase(NewTestcase {
                run_id: run_a.id.clone(),
                story_id: story.id.clone(),
                title: "TC-A1".into(),
                category: "happy".into(),
                priority: "medium".into(),
                steps_json: "{}".into(),
                order_idx: 0,
            })
            .await
            .unwrap();

        // Add a testcase to run_b
        let tc_b1 = repo
            .add_testcase(NewTestcase {
                run_id: run_b.id.clone(),
                story_id: story.id.clone(),
                title: "TC-B1".into(),
                category: "error".into(),
                priority: "high".into(),
                steps_json: "{}".into(),
                order_idx: 0,
            })
            .await
            .unwrap();

        // Approve run_a only
        repo.approve_run_testcases(&run_a.id).await.unwrap();

        let tc_a1_after = repo.get_testcase(&tc_a1.id).await.unwrap();
        let tc_b1_after = repo.get_testcase(&tc_b1.id).await.unwrap();

        // run_a's case should be approved
        assert_eq!(tc_a1_after.status, "approved");
        // run_b's case should remain draft
        assert_eq!(tc_b1_after.status, "draft");
    }

    // -----------------------------------------------------------------------
    // list_learnings(active_only=true) excludes inactive
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_learnings_active_only_excludes_inactive() {
        let pool = mem_pool().await;
        let repo = ProductRepo::new(pool.clone());
        let user = seed_user(&pool).await;
        let ws = seed_workspace(&pool, &user).await;

        let l1 = repo
            .create_learning(NewLearning {
                workspace_id: ws.clone(),
                kind: "pattern".into(),
                title: "Good pattern".into(),
                body: "Do this".into(),
                tags: "".into(),
                refs_json: "[]".into(),
                source_story_id: None,
                created_by: user.clone(),
            })
            .await
            .unwrap();

        let l2 = repo
            .create_learning(NewLearning {
                workspace_id: ws.clone(),
                kind: "avoid".into(),
                title: "Bad pattern".into(),
                body: "Don't do this".into(),
                tags: "".into(),
                refs_json: "[]".into(),
                source_story_id: None,
                created_by: user.clone(),
            })
            .await
            .unwrap();

        // Deactivate l2
        repo.update_learning(
            &l2.id,
            LearningPatch {
                kind: None,
                title: None,
                body: None,
                tags: None,
                refs_json: None,
                active: Some(false),
            },
        )
        .await
        .unwrap();

        let active = repo.list_learnings(&ws, true).await.unwrap();
        let all = repo.list_learnings(&ws, false).await.unwrap();

        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, l1.id);
        assert_eq!(all.len(), 2);
    }

    // -----------------------------------------------------------------------
    // analysis agent: session_id round-trips through add + set_agent_session
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn analysis_agent_session_id_round_trips() {
        let pool = mem_pool().await;
        let repo = ProductRepo::new(pool.clone());
        let user = seed_user(&pool).await;
        let ws = seed_workspace(&pool, &user).await;
        let story = repo.create_story(new_story_input(&ws, &user)).await.unwrap();

        let analysis = repo
            .create_analysis(NewAnalysis {
                story_id: story.id.clone(),
                source_version_id: None,
                status: "running".into(),
                created_by: user.clone(),
            })
            .await
            .unwrap();

        // Created with no session_id.
        let agent = repo
            .add_analysis_agent(NewAnalysisAgent {
                analysis_id: analysis.id.clone(),
                name: "PO Overview \u{00b7} claude".into(),
                skill: "po-story-overview".into(),
                provider: "claude".into(),
                model: String::new(),
                status: "running".into(),
                session_id: None,
            })
            .await
            .unwrap();
        assert!(agent.session_id.is_none());

        // set_agent_session records the live session id.
        let sid = new_id();
        repo.set_agent_session(&agent.id, &sid).await.unwrap();

        let agents = repo.list_analysis_agents(&analysis.id).await.unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].session_id.as_ref(), Some(&sid));

        // An agent created WITH a session_id keeps it.
        let agent2 = repo
            .add_analysis_agent(NewAnalysisAgent {
                analysis_id: analysis.id.clone(),
                name: "Architecture \u{00b7} codex".into(),
                skill: "story-architecture-overview".into(),
                provider: "codex".into(),
                model: String::new(),
                status: "running".into(),
                session_id: Some(new_id()),
            })
            .await
            .unwrap();
        assert!(agent2.session_id.is_some());
    }

    // -----------------------------------------------------------------------
    // delete_story removes the story and its versions
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn delete_story_removes_story_and_versions() {
        let pool = mem_pool().await;
        let repo = ProductRepo::new(pool.clone());
        let user = seed_user(&pool).await;
        let ws = seed_workspace(&pool, &user).await;

        let story = repo.create_story(new_story_input(&ws, &user)).await.unwrap();
        let _v = repo
            .add_version(NewVersion {
                story_id: story.id.clone(),
                kind: "source".into(),
                title: "v1".into(),
                body_md: "body".into(),
                raw_json: None,
                change_notes: None,
                created_by: user.clone(),
            })
            .await
            .unwrap();

        // Verify story and version exist
        assert!(repo.get_story(&story.id).await.is_ok());
        let versions_before = repo.list_versions(&story.id).await.unwrap();
        assert_eq!(versions_before.len(), 1);

        // Delete
        repo.delete_story(&story.id).await.unwrap();

        // Story should be gone
        let res = repo.get_story(&story.id).await;
        assert!(res.is_err());

        // Versions should be gone
        let versions_after = repo.list_versions(&story.id).await.unwrap();
        assert_eq!(versions_after.len(), 0);
    }

    // -----------------------------------------------------------------------
    // create_transcript → get/list round-trip
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn create_transcript_get_list_round_trip() {
        let pool = mem_pool().await;
        let repo = ProductRepo::new(pool.clone());
        let user = seed_user(&pool).await;
        let ws = seed_workspace(&pool, &user).await;
        let story = repo.create_story(new_story_input(&ws, &user)).await.unwrap();

        let t = repo
            .create_transcript(NewTranscript {
                story_id: story.id.clone(),
                title: "Brainstorm Session 1".into(),
                body: "# Ideas\n- Idea A\n- Idea B".into(),
                created_by: user.clone(),
            })
            .await
            .unwrap();

        assert_eq!(t.story_id, story.id);
        assert_eq!(t.title, "Brainstorm Session 1");
        assert_eq!(t.body, "# Ideas\n- Idea A\n- Idea B");
        assert_eq!(t.created_by, user);

        // get round-trip
        let fetched = repo.get_transcript(&t.id).await.unwrap();
        assert_eq!(fetched.id, t.id);
        assert_eq!(fetched.title, t.title);

        // list shows the transcript
        let list = repo.list_transcripts(&story.id).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, t.id);
    }

    // -----------------------------------------------------------------------
    // delete_transcript removes only the target record
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn delete_transcript_removes_only_target() {
        let pool = mem_pool().await;
        let repo = ProductRepo::new(pool.clone());
        let user = seed_user(&pool).await;
        let ws = seed_workspace(&pool, &user).await;
        let story = repo.create_story(new_story_input(&ws, &user)).await.unwrap();

        let t1 = repo
            .create_transcript(NewTranscript {
                story_id: story.id.clone(),
                title: "First".into(),
                body: "content 1".into(),
                created_by: user.clone(),
            })
            .await
            .unwrap();
        let t2 = repo
            .create_transcript(NewTranscript {
                story_id: story.id.clone(),
                title: "Second".into(),
                body: "content 2".into(),
                created_by: user.clone(),
            })
            .await
            .unwrap();

        repo.delete_transcript(&t1.id).await.unwrap();

        // t1 should be gone
        assert!(repo.get_transcript(&t1.id).await.is_err());
        // t2 should remain
        assert!(repo.get_transcript(&t2.id).await.is_ok());

        let list = repo.list_transcripts(&story.id).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, t2.id);
    }

    // -----------------------------------------------------------------------
    // update_version_body changes title + body_md
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn update_version_body_changes_title_and_body() {
        let pool = mem_pool().await;
        let repo = ProductRepo::new(pool.clone());
        let user = seed_user(&pool).await;
        let ws = seed_workspace(&pool, &user).await;
        let story = repo.create_story(new_story_input(&ws, &user)).await.unwrap();

        let v = repo
            .add_version(NewVersion {
                story_id: story.id.clone(),
                kind: "draft".into(),
                title: "Original Title".into(),
                body_md: "# Original Body".into(),
                raw_json: None,
                change_notes: None,
                created_by: user.clone(),
            })
            .await
            .unwrap();

        let updated = repo
            .update_version_body(&v.id, "Revised Title", "# Revised Body\nNew content.")
            .await
            .unwrap();

        assert_eq!(updated.id, v.id);
        assert_eq!(updated.title, "Revised Title");
        assert_eq!(updated.body_md, "# Revised Body\nNew content.");
        // Other fields unchanged
        assert_eq!(updated.kind, "draft");
        assert_eq!(updated.version_no, v.version_no);
    }

    // -----------------------------------------------------------------------
    // update_story sets tags; get_story returns them (round-trip)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn update_story_sets_tags_and_get_story_returns_them() {
        let pool = mem_pool().await;
        let repo = ProductRepo::new(pool.clone());
        let user = seed_user(&pool).await;
        let ws = seed_workspace(&pool, &user).await;

        let story = repo.create_story(new_story_input(&ws, &user)).await.unwrap();
        // New story starts with empty tags.
        assert_eq!(story.tags, "");

        // Patch tags.
        let updated = repo
            .update_story(
                &story.id,
                StoryPatch {
                    tags: Some("auth,payments,mvp".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.tags, "auth,payments,mvp");

        // get_story returns the same tags.
        let fetched = repo.get_story(&story.id).await.unwrap();
        assert_eq!(fetched.tags, "auth,payments,mvp");

        // Patch with None leaves tags unchanged.
        let same = repo
            .update_story(
                &story.id,
                StoryPatch {
                    stage: Some("review".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(same.tags, "auth,payments,mvp");
        assert_eq!(same.stage, "review");

        // Patch to empty string clears tags.
        let cleared = repo
            .update_story(
                &story.id,
                StoryPatch {
                    tags: Some("".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(cleared.tags, "");
    }

    // -----------------------------------------------------------------------
    // delete_story removes transcripts (cascade)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn delete_story_removes_transcripts() {
        let pool = mem_pool().await;
        let repo = ProductRepo::new(pool.clone());
        let user = seed_user(&pool).await;
        let ws = seed_workspace(&pool, &user).await;
        let story = repo.create_story(new_story_input(&ws, &user)).await.unwrap();

        let t = repo
            .create_transcript(NewTranscript {
                story_id: story.id.clone(),
                title: "Draft notes".into(),
                body: "some content".into(),
                created_by: user.clone(),
            })
            .await
            .unwrap();

        // Confirm transcript exists
        assert!(repo.get_transcript(&t.id).await.is_ok());

        // Delete the story
        repo.delete_story(&story.id).await.unwrap();

        // Transcript should be gone
        assert!(repo.get_transcript(&t.id).await.is_err());

        let list = repo.list_transcripts(&story.id).await.unwrap();
        assert_eq!(list.len(), 0);
    }
}
