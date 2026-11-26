//! Run history with open-text search (`GET /workspaces/{wid}/workflow-runs`):
//! one bounded, keyset-paginated query over a workspace's runs, matching every
//! search term against the workflow name, the run input (trigger message,
//! prompt, goals, repo, branch, PR), the run error, the step names (the
//! workflow graph) and the step summaries (phases, step errors).
//!
//! Bounded: only the newest [`SEARCH_WINDOW`] runs of the workspace are
//! scanned, a query has at most [`MAX_TERMS`] terms of at most [`MAX_Q`] chars,
//! and a page holds at most [`MAX_LIMIT`] rows. No schema change: SQLite
//! `LIKE` (ASCII case-insensitive) over the stored JSON, like the other
//! history lookups here.

use crate::convert::{dberr, ts};
use crate::DbPool;
use otto_core::workflows::{
    NodeStatus, RunListItem, RunListPage, RunStatus, Subject, SubjectPr, SubjectTicket,
    WorkflowGraph,
};
use otto_core::{Error, Id, Result};
use serde_json::Value;
use sqlx::Row;

/// Newest runs per workspace a search scans.
pub const SEARCH_WINDOW: i64 = 5_000;
pub const MAX_TERMS: usize = 8;
pub const MAX_Q: usize = 200;
pub const DEFAULT_LIMIT: u32 = 30;
pub const MAX_LIMIT: u32 = 100;

/// `GET /workspaces/{wid}/workflow-runs` query, validated by [`search_runs`].
#[derive(Debug, Clone, Default)]
pub struct RunSearch {
    pub workspace_id: Id,
    pub workflow_id: Option<Id>,
    /// Open text; whitespace-separated terms, all must match.
    pub q: Option<String>,
    /// Status filter (`running`, `success`, …); empty = any.
    pub statuses: Vec<String>,
    /// Only runs waiting on a person: running with a step on a person or an
    /// approval pause, or held blocked / exhausted (the Runs inbox "Needs you").
    pub attention: bool,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

/// `%term%` with LIKE's wildcards escaped (`\` is the ESCAPE char).
fn like_pattern(term: &str) -> String {
    let mut out = String::with_capacity(term.len() + 2);
    out.push('%');
    for c in term.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// Search terms: trimmed, at most [`MAX_TERMS`], the query at most [`MAX_Q`]
/// chars. `400` beyond those bounds rather than silently truncating.
pub fn terms(q: Option<&str>) -> Result<Vec<String>> {
    let q = q.map(str::trim).unwrap_or_default();
    if q.chars().count() > MAX_Q {
        return Err(Error::Invalid(format!(
            "q must be at most {MAX_Q} characters"
        )));
    }
    let t: Vec<String> = q.split_whitespace().map(str::to_string).collect();
    if t.len() > MAX_TERMS {
        return Err(Error::Invalid(format!(
            "q must have at most {MAX_TERMS} terms"
        )));
    }
    Ok(t)
}

fn parse_cursor(c: &str) -> Result<(String, String)> {
    let (at, id) = c
        .split_once('|')
        .filter(|(at, id)| !at.is_empty() && !id.is_empty() && ts(at).is_ok())
        .ok_or_else(|| Error::Invalid("bad cursor".into()))?;
    Ok((at.to_string(), id.to_string()))
}

pub async fn search_runs(pool: &DbPool, s: &RunSearch) -> Result<RunListPage> {
    let terms = terms(s.q.as_deref())?;
    for st in &s.statuses {
        if RunStatus::parse(st).is_none() {
            return Err(Error::Invalid(format!("unknown status '{st}'")));
        }
    }
    let limit = s.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let cursor = s.cursor.as_deref().map(parse_cursor).transpose()?;
    let mut sql = String::from(
        "SELECT r.id, r.workflow_id, w.name AS workflow_name, r.status, r.started_at,
                r.finished_at, r.error, r.input_json, r.overview_json, r.rev,
                COALESCE(r.progress_json, r.nodes_json) AS nodes_json, w.graph_json
         FROM (SELECT * FROM workflow_runs WHERE workspace_id = ?",
    );
    if s.attention {
        // Repeats the partial index's predicate, so the scan uses
        // `idx_workflow_runs_ws_attention` (running or held rows only).
        sql.push_str(
            " AND (status = 'running' OR hold_json IS NOT NULL)
              AND ((status = 'running' AND (attention > 0 OR waiting_approval = 1))
                   OR (status = 'error'
                       AND json_extract(hold_json, '$.kind') IN ('blocked', 'exhausted')))",
        );
    }
    sql.push_str(
        " ORDER BY started_at DESC, id DESC LIMIT ?) r
         JOIN workflows w ON w.id = r.workflow_id
         WHERE 1 = 1",
    );
    if s.workflow_id.is_some() {
        sql.push_str(" AND r.workflow_id = ?");
    }
    if !s.statuses.is_empty() {
        sql.push_str(" AND r.status IN (");
        sql.push_str(&vec!["?"; s.statuses.len()].join(","));
        sql.push(')');
    }
    for _ in &terms {
        sql.push_str(
            " AND (w.name LIKE ?1x ESCAPE '\\' OR r.input_json LIKE ?1x ESCAPE '\\'
                   OR COALESCE(r.error, '') LIKE ?1x ESCAPE '\\'
                   OR w.graph_json LIKE ?1x ESCAPE '\\'
                   OR COALESCE(r.progress_json, '') LIKE ?1x ESCAPE '\\')",
        );
    }
    if cursor.is_some() {
        sql.push_str(" AND (r.started_at < ? OR (r.started_at = ? AND r.id < ?))");
    }
    sql.push_str(" ORDER BY r.started_at DESC, r.id DESC LIMIT ?");
    // One `?` per occurrence (sqlx binds positionally).
    let sql = sql.replace("?1x", "?");
    // Only fixed fragments and `?` placeholders are spliced; every value binds.
    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(&s.workspace_id)
        .bind(SEARCH_WINDOW);
    if let Some(w) = &s.workflow_id {
        q = q.bind(w);
    }
    for st in &s.statuses {
        q = q.bind(st);
    }
    for t in &terms {
        let p = like_pattern(t);
        for _ in 0..5 {
            q = q.bind(p.clone());
        }
    }
    if let Some((at, id)) = &cursor {
        q = q.bind(at).bind(at).bind(id);
    }
    q = q.bind(i64::from(limit) + 1);
    let rows = q.fetch_all(pool).await.map_err(dberr("search runs"))?;
    let more = rows.len() > limit as usize;
    let mut items = Vec::with_capacity(rows.len().min(limit as usize));
    for r in rows.iter().take(limit as usize) {
        items.push(row_to_item(r)?);
    }
    // The cursor carries the row's STORED `started_at` text, so the keyset
    // comparison is exact whatever format the row was written with.
    let next_cursor = more
        .then(|| rows.get(limit as usize - 1))
        .flatten()
        .map(|r| {
            format!(
                "{}|{}",
                r.get::<String, _>("started_at"),
                r.get::<String, _>("id")
            )
        });
    Ok(RunListPage { items, next_cursor })
}

fn clip(s: &str, n: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= n {
        t.to_string()
    } else {
        format!("{}…", t.chars().take(n).collect::<String>())
    }
}

fn str_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn basename(p: &str) -> String {
    p.trim()
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(p)
        .to_string()
}

fn row_to_item(r: &sqlx::sqlite::SqliteRow) -> Result<RunListItem> {
    let input: Value =
        serde_json::from_str(&r.get::<String, _>("input_json")).unwrap_or(Value::Null);
    let started_at = ts(&r.get::<String, _>("started_at"))?;
    let finished_at = r
        .get::<Option<String>, _>("finished_at")
        .as_deref()
        .map(ts)
        .transpose()?;
    let status = RunStatus::parse(&r.get::<String, _>("status"))
        .ok_or_else(|| Error::Internal("bad run status".into()))?;
    let repo = input_repo(&input);
    let overview = crate::workflows::fresh_overview(
        r.get::<Option<String>, _>("overview_json").as_deref(),
        r.get("rev"),
    );
    let pr = match input.get("pr") {
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    };
    let current_step = (status == RunStatus::Running)
        .then(|| current_step(r))
        .flatten();
    Ok(RunListItem {
        id: r.get("id"),
        workflow_id: r.get("workflow_id"),
        workflow_name: r.get("workflow_name"),
        status,
        started_at,
        finished_at,
        duration_ms: finished_at.map(|f| (f - started_at).num_milliseconds().max(0) as u64),
        trigger: str_of(&input, "trigger").unwrap_or("manual").to_string(),
        message: str_of(&input, "msg")
            .or_else(|| str_of(&input, "prompt"))
            .map(|m| clip(m, 160)),
        repo,
        branch: str_of(&input, "pr_branch")
            .or_else(|| str_of(&input, "base"))
            .map(str::to_string),
        pr,
        current_step,
        error: r
            .get::<Option<String>, _>("error")
            .filter(|e| !e.trim().is_empty())
            .map(|e| clip(&e, 240)),
        subject: overview
            .as_ref()
            .and_then(|o| o.subject.clone())
            .or_else(|| subject_from_input(&input)),
        overview,
    })
}

/// Repo (last path segment of the working directory / first declared repo).
fn input_repo(input: &Value) -> Option<String> {
    str_of(input, "working_directory")
        .and_then(|w| w.split(',').next())
        .map(basename)
        .or_else(|| {
            input
                .get("repos")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|e| str_of(e, "repo").or_else(|| str_of(e, "name")))
                .map(basename)
        })
}

/// What a run is about, from its input alone — the pull request, the ticket,
/// the branch and the repository it names — for a run whose overview carries
/// no subject (every run from before overviews existed). The base branch is
/// where the work merges, not what it is about, so it is never the subject.
/// `None` when the input names none of them.
pub fn subject_from_input(input: &Value) -> Option<Subject> {
    let repo = input_repo(input);
    // The PRs admission resolved (`prs[]`, as in the run's prs.json), else the
    // one the input names.
    let resolved = input
        .get("prs")
        .and_then(Value::as_array)
        .and_then(|a| a.first());
    let pr = match resolved {
        Some(p) => p
            .get("number")
            .and_then(Value::as_u64)
            .map(|number| SubjectPr {
                repo: str_of(p, "repo_path")
                    .map(basename)
                    .or_else(|| repo.clone())
                    .unwrap_or_default(),
                number,
                url: str_of(p, "url").unwrap_or_default().to_string(),
                title: str_of(p, "title").map(str::to_string),
            }),
        None => input.get("pr").and_then(|v| named_pr(v, repo.as_deref())),
    };
    let ticket = str_of(input, "jira_ticket").and_then(|t| {
        Some(SubjectTicket {
            key: otto_core::text::issue_key(&t.to_ascii_uppercase())?,
            url: Some(t)
                .filter(|t| t.starts_with("http"))
                .unwrap_or_default()
                .to_string(),
            title: None,
        })
    });
    let subject = Subject {
        pr,
        ticket,
        repo,
        branch: str_of(input, "pr_branch").map(str::to_string),
    };
    (subject != Subject::default()).then_some(subject)
}

/// The first pull request `input.pr` names: `15`, `"#15"`,
/// `"repo-a#15, repo-b#340"` or a PR's URL (`…/repo/pull/15`,
/// `…/repo/pull-requests/15`); the run's repo when it names none.
fn named_pr(v: &Value, run_repo: Option<&str>) -> Option<SubjectPr> {
    let text = match v {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s
            .split(',')
            .next()?
            .trim()
            .trim_end_matches('/')
            .to_string(),
        _ => return None,
    };
    let is_url = text.starts_with("http");
    let (before, number) = text.rsplit_once(['#', '/']).unwrap_or(("", &text));
    let number: u64 = number.trim().parse().ok().filter(|n| *n > 0)?;
    // A URL's repo is the segment before "pull" / "pull-requests".
    let named = if is_url {
        before.rsplit('/').nth(1).filter(|r| *r != "-")
    } else {
        Some(before.trim()).filter(|r| !r.is_empty())
    };
    Some(SubjectPr {
        repo: named.or(run_repo).map(basename).unwrap_or_default(),
        number,
        url: if is_url { text.clone() } else { String::new() },
        title: None,
    })
}

/// The first running node's display name (name, else kind).
fn current_step(r: &sqlx::sqlite::SqliteRow) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct N {
        #[serde(default)]
        node_id: String,
        status: NodeStatus,
    }
    let nodes: Vec<N> = serde_json::from_str(&r.get::<String, _>("nodes_json")).ok()?;
    let id = nodes
        .into_iter()
        .find(|n| n.status == NodeStatus::Running)?
        .node_id;
    let graph: WorkflowGraph = serde_json::from_str(&r.get::<String, _>("graph_json")).ok()?;
    let node = graph.nodes.into_iter().find(|n| n.id == id)?;
    Some(if node.name.trim().is_empty() {
        node.kind
    } else {
        node.name
    })
}

#[cfg(test)]
#[path = "workflow_run_search_tests.rs"]
mod tests;
