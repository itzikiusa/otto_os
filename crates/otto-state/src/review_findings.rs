//! Persistent finding identity for the A1 verified-review loop.
//!
//! A "finding" is a deduplicated, fingerprinted review comment tracked across
//! multiple review runs. Each run computes a sha2 fingerprint for each finding
//! and UPSERTs into `review_findings` with lifecycle transitions:
//!   brand-new → `open`
//!   was `open`, absent this run → `resolved`
//!   was `resolved` or `declined`, reappears → `regressed`
//!
//! The `state` column (added by migration 0054) is the authoritative lifecycle
//! field; the legacy `status` column from 0049 is no longer written here.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt};
use otto_core::finding::{Finding, FindingSeverity, FindingStatus};
use otto_core::{new_id, Error, Id, Result};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Lifecycle states for a review finding (matches the `state` column values).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingState {
    Open,
    Fixing,
    Resolved,
    Regressed,
    Declined,
}

impl FindingState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Fixing => "fixing",
            Self::Resolved => "resolved",
            Self::Regressed => "regressed",
            Self::Declined => "declined",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "open" => Some(Self::Open),
            "fixing" => Some(Self::Fixing),
            "resolved" => Some(Self::Resolved),
            "regressed" => Some(Self::Regressed),
            "declined" => Some(Self::Declined),
            _ => None,
        }
    }
}

/// A review finding row as returned by the list/update routes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewFindingRow {
    pub id: String,
    pub review_id: String,
    pub fingerprint: String,
    pub path: Option<String>,
    pub line: Option<i64>,
    pub severity: String,
    pub category: Option<String>,
    pub body: String,
    pub state: FindingState,
    pub fix_session_id: Option<String>,
    pub updated_at: String,
}

/// Input for creating or upserting a finding within a review run.
pub struct NewFinding<'a> {
    pub review_id: &'a str,
    pub workspace_id: &'a str,
    pub repo_id: &'a str,
    /// `None` for a local (non-PR) review — those dedup within the review_id,
    /// not across runs of a PR. Stored as the `0` sentinel in the NOT NULL column.
    pub pr_number: Option<u64>,
    pub path: Option<&'a str>,
    pub line: Option<i64>,
    pub line_end: Option<i64>,
    pub severity: &'a str,
    pub category: Option<&'a str>,
    pub title: &'a str,
    pub body: &'a str,
    pub evidence: &'a str,
    pub agent_reasoning_summary: &'a str,
    pub suggested_fix: Option<&'a str>,
    pub produced_by_agent: Option<&'a str>,
    pub reviewer: &'a str,
    /// sha2 hex fingerprint — computed by `compute_fingerprint`.
    pub fingerprint: &'a str,
    /// The review run id used to populate first_seen_run / last_seen_run.
    pub run_id: &'a str,
}

/// A partial update of a finding's workflow artifact/gate fields. Each `Some`
/// is written; `None` keeps the current value. Never includes `status` (use
/// [`ReviewFindingsRepo::set_status`] for that).
#[derive(Debug, Clone, Default)]
pub struct FindingPatch {
    pub linked_commit: Option<String>,
    pub linked_test: Option<String>,
    pub jira_key: Option<String>,
    pub jira_url: Option<String>,
    pub fix_session_id: Option<String>,
    pub repo_rule_id: Option<String>,
    pub requires_human_approval: Option<bool>,
    pub approved_by: Option<String>,
    pub approved_at: Option<String>,
    pub approval_decision: Option<String>,
    pub reviewer: Option<String>,
    pub produced_by_agent: Option<String>,
}

// ---------------------------------------------------------------------------
// Fingerprint computation
// ---------------------------------------------------------------------------

/// Compute a stable sha2-256 fingerprint for a finding across runs.
///
/// Inputs: repo slug + PR number + normalized path + category + first 512 chars
/// of body text. Case-normalised and stripped of leading/trailing whitespace so
/// minor reformatting across runs doesn't create new fingerprints.
pub fn compute_fingerprint(
    repo_id: &str,
    pr_number: u64,
    path: Option<&str>,
    category: Option<&str>,
    body: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(repo_id.as_bytes());
    hasher.update(b"|");
    hasher.update(pr_number.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(path.unwrap_or("").trim().to_lowercase().as_bytes());
    hasher.update(b"|");
    hasher.update(category.unwrap_or("").trim().to_lowercase().as_bytes());
    hasher.update(b"|");
    // Cap body to 512 chars (unicode-safe via char boundary trim) to avoid
    // tiny wording changes producing different fingerprints.
    let body_trimmed = body.trim();
    let cap = body_trimmed
        .char_indices()
        .nth(512)
        .map(|(i, _)| i)
        .unwrap_or(body_trimmed.len());
    hasher.update(body_trimmed[..cap].to_lowercase().as_bytes());
    hex::encode(hasher.finalize())
}

// ---------------------------------------------------------------------------
// Repository
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ReviewFindingsRepo {
    pool: SqlitePool,
}

impl ReviewFindingsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // -- Row mapping ----------------------------------------------------------

    fn row_to_finding(r: &sqlx::sqlite::SqliteRow) -> Result<ReviewFindingRow> {
        let state_raw: String = r.try_get("state").unwrap_or_else(|_| "open".to_string());
        Ok(ReviewFindingRow {
            id: r.get("id"),
            review_id: r.get("review_id"),
            fingerprint: r.get("fingerprint"),
            path: r.get("path"),
            line: r.get("line"),
            severity: r.try_get("severity").unwrap_or_else(|_| "info".to_string()),
            category: r.try_get("category").ok(),
            body: r.get("body"),
            state: FindingState::parse(&state_raw).unwrap_or(FindingState::Open),
            fix_session_id: r.try_get("fix_session_id").ok().flatten(),
            updated_at: r.try_get("updated_at").unwrap_or_else(|_| String::new()),
        })
    }

    // -- Queries --------------------------------------------------------------

    /// List all findings for a review run (latest run for the pr).
    pub async fn list_for_review(&self, review_id: &Id) -> Result<Vec<ReviewFindingRow>> {
        let rows = sqlx::query(
            "SELECT * FROM review_findings WHERE review_id = ? ORDER BY severity DESC, path, line",
        )
        .bind(review_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list findings for review"))?;
        rows.iter().map(Self::row_to_finding).collect()
    }

    /// Upsert a finding from the current run, applying the engine DETECTION
    /// lifecycle (the `state` axis ONLY — the workflow `status` is owned by the
    /// action endpoints and is NEVER touched here, so human triage survives a
    /// re-review; §15.1). Returns the full [`Finding`] and whether it was newly
    /// created (so the caller can append a `created` audit event).
    ///
    /// Dedup key: PR reviews dedup across runs by `(workspace, repo, pr, fingerprint)`;
    /// local reviews (`pr_number=None`) dedup within `(review_id, fingerprint)`.
    pub async fn upsert(&self, f: &NewFinding<'_>) -> Result<(Finding, bool)> {
        let now = fmt(Utc::now());
        let severity = FindingSeverity::normalize(f.severity); // normalize on write

        // Locate any existing row for this fingerprint within its dedup scope.
        let existing: Option<sqlx::sqlite::SqliteRow> = match f.pr_number {
            Some(pr) => sqlx::query(
                "SELECT id, state FROM review_findings \
                 WHERE workspace_id = ? AND repo_id = ? AND pr_number = ? AND fingerprint = ? \
                 LIMIT 1",
            )
            .bind(f.workspace_id)
            .bind(f.repo_id)
            .bind(pr as i64)
            .bind(f.fingerprint),
            None => sqlx::query(
                "SELECT id, state FROM review_findings \
                 WHERE review_id = ? AND fingerprint = ? LIMIT 1",
            )
            .bind(f.review_id)
            .bind(f.fingerprint),
        }
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("upsert finding lookup"))?;

        let pr_col = f.pr_number.unwrap_or(0) as i64;

        if let Some(row) = existing {
            let existing_id: String = row.get("id");
            let prev_state: String = row.try_get("state").unwrap_or_else(|_| "open".to_string());
            // Reappearance of a resolved/declined finding → regressed (detection axis).
            let next_state = match prev_state.as_str() {
                "resolved" | "declined" => "regressed",
                other => other, // open / fixing / regressed — keep as-is
            };
            sqlx::query(
                "UPDATE review_findings \
                 SET state = ?, review_id = ?, last_seen_run = ?, \
                     occurrence_count = occurrence_count + 1, updated_at = ? \
                 WHERE id = ?",
            )
            .bind(next_state)
            .bind(f.review_id)
            .bind(f.run_id)
            .bind(&now)
            .bind(&existing_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("upsert finding update"))?;
            Ok((self.get_full(&existing_id).await?, false))
        } else {
            // Brand-new finding for this fingerprint.
            let id = new_id();
            sqlx::query(
                "INSERT INTO review_findings \
                 (id, workspace_id, repo_id, pr_number, review_id, fingerprint, \
                  path, line, line_end, severity, category, title, body, evidence, \
                  agent_reasoning_summary, suggested_fix, reviewer, produced_by_agent, \
                  status, state, first_seen_review_id, last_seen_review_id, \
                  first_seen_run, last_seen_run, occurrence_count, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, \
                         'open', 'open', ?, ?, ?, ?, 1, ?, ?)",
            )
            .bind(&id)
            .bind(f.workspace_id)
            .bind(f.repo_id)
            .bind(pr_col)
            .bind(f.review_id)
            .bind(f.fingerprint)
            .bind(f.path)
            .bind(f.line)
            .bind(f.line_end)
            .bind(severity.as_str())
            .bind(f.category)
            .bind(f.title)
            .bind(f.body)
            .bind(f.evidence)
            .bind(f.agent_reasoning_summary)
            .bind(f.suggested_fix)
            .bind(f.reviewer)
            .bind(f.produced_by_agent)
            .bind(f.review_id) // first_seen_review_id
            .bind(f.review_id) // last_seen_review_id
            .bind(f.run_id) // first_seen_run
            .bind(f.run_id) // last_seen_run
            .bind(&now)
            .bind(&now)
            .execute(&self.pool)
            .await
            .map_err(dberr("upsert finding insert"))?;
            Ok((self.get_full(&id).await?, true))
        }
    }

    /// Mark all `open` findings for (workspace, repo, pr) that are NOT in
    /// `seen_fingerprints` as `resolved` (they disappeared from this run).
    /// Findings already in a terminal or manually-set state are left untouched.
    pub async fn resolve_absent(
        &self,
        workspace_id: &str,
        repo_id: &str,
        pr_number: u64,
        seen_fingerprints: &[&str],
        run_id: &str,
    ) -> Result<u64> {
        if seen_fingerprints.is_empty() {
            // Nothing was seen → resolve all open findings.
            let now = fmt(Utc::now());
            let res = sqlx::query(
                "UPDATE review_findings \
                 SET state = 'resolved', last_seen_run = ?, updated_at = ? \
                 WHERE workspace_id = ? AND repo_id = ? AND pr_number = ? \
                   AND state IN ('open', 'fixing')",
            )
            .bind(run_id)
            .bind(&now)
            .bind(workspace_id)
            .bind(repo_id)
            .bind(pr_number as i64)
            .execute(&self.pool)
            .await
            .map_err(dberr("resolve absent findings"))?;
            return Ok(res.rows_affected());
        }

        // SQLite doesn't support `NOT IN (?)` with a slice directly; build the
        // placeholder list. This is safe (fp values are hex strings).
        let placeholders = seen_fingerprints
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "UPDATE review_findings \
             SET state = 'resolved', last_seen_run = ?, updated_at = ? \
             WHERE workspace_id = ? AND repo_id = ? AND pr_number = ? \
               AND state IN ('open', 'fixing') \
               AND fingerprint NOT IN ({placeholders})"
        );
        let now = fmt(Utc::now());
        let mut q = sqlx::query(&sql).bind(run_id).bind(&now).bind(workspace_id).bind(repo_id).bind(pr_number as i64);
        for fp in seen_fingerprints {
            q = q.bind(*fp);
        }
        let res = q
            .execute(&self.pool)
            .await
            .map_err(dberr("resolve absent findings"))?;
        Ok(res.rows_affected())
    }

    /// Fetch a single finding row by its id.
    pub async fn get(&self, id: &Id) -> Result<ReviewFindingRow> {
        let row = sqlx::query("SELECT * FROM review_findings WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get finding"))?;
        Self::row_to_finding(&row)
    }

    /// Update the lifecycle state (and optionally fix_session_id) of a finding.
    pub async fn set_state(
        &self,
        review_id: &Id,
        fingerprint: &str,
        new_state: FindingState,
        fix_session_id: Option<&str>,
    ) -> Result<ReviewFindingRow> {
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE review_findings \
             SET state = ?, fix_session_id = ?, updated_at = ? \
             WHERE review_id = ? AND fingerprint = ?",
        )
        .bind(new_state.as_str())
        .bind(fix_session_id)
        .bind(&now)
        .bind(review_id)
        .bind(fingerprint)
        .execute(&self.pool)
        .await
        .map_err(dberr("set finding state"))?;

        let row = sqlx::query(
            "SELECT * FROM review_findings WHERE review_id = ? AND fingerprint = ? LIMIT 1",
        )
        .bind(review_id)
        .bind(fingerprint)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("get finding after state update"))?;
        Self::row_to_finding(&row)
    }

    // -- Workflow (status-axis) API — used by the action endpoints ------------

    /// Map a full row to the rich [`Finding`] DTO (the workflow shape).
    fn row_to_full(r: &sqlx::sqlite::SqliteRow) -> Result<Finding> {
        let status_raw: String = r.try_get("status").unwrap_or_else(|_| "open".to_string());
        let sev_raw: String = r.try_get("severity").unwrap_or_else(|_| "info".to_string());
        let state: String = r.try_get("state").unwrap_or_else(|_| "open".to_string());
        let pr_num: i64 = r.try_get("pr_number").unwrap_or(0);
        let line: Option<i64> = r.try_get("line").ok().flatten();
        let line_end: Option<i64> = r.try_get("line_end").ok().flatten();
        let req_appr: i64 = r.try_get("requires_human_approval").unwrap_or(0);
        let occ: i64 = r.try_get("occurrence_count").unwrap_or(1);
        let category: Option<String> = r.try_get("category").ok().flatten();
        let path: Option<String> = r.try_get("path").ok().flatten();
        Ok(Finding {
            id: r.get("id"),
            review_id: r.try_get("review_id").unwrap_or_default(),
            workspace_id: r.get("workspace_id"),
            repo_id: r.get("repo_id"),
            pr_number: if pr_num > 0 { Some(pr_num as u64) } else { None },
            fingerprint: r.get("fingerprint"),
            severity: FindingSeverity::normalize(&sev_raw),
            category,
            path,
            line: line.map(|l| l as u32),
            line_end: line_end.map(|l| l as u32),
            title: r.try_get("title").unwrap_or_default(),
            body: r.try_get("body").unwrap_or_default(),
            evidence: r.try_get("evidence").unwrap_or_default(),
            agent_reasoning_summary: r.try_get("agent_reasoning_summary").unwrap_or_default(),
            suggested_fix: r.try_get("suggested_fix").ok().flatten(),
            status: FindingStatus::parse(&status_raw).unwrap_or(FindingStatus::Open),
            linked_commit: r.try_get("linked_commit").ok().flatten(),
            linked_test: r.try_get("linked_test").ok().flatten(),
            reviewer: r.try_get("reviewer").unwrap_or_default(),
            state: state.clone(),
            regressed: state == "regressed",
            requires_human_approval: req_appr != 0,
            approval_decision: r.try_get("approval_decision").ok().flatten(),
            approved_by: r.try_get("approved_by").ok().flatten(),
            approved_at: r.try_get("approved_at").ok().flatten(),
            jira_key: r.try_get("jira_key").ok().flatten(),
            jira_url: r.try_get("jira_url").ok().flatten(),
            produced_by_agent: r.try_get("produced_by_agent").ok().flatten(),
            repo_rule_id: r.try_get("repo_rule_id").ok().flatten(),
            fix_session_id: r.try_get("fix_session_id").ok().flatten(),
            occurrence_count: occ,
            created_at: r.try_get("created_at").unwrap_or_default(),
            updated_at: r.try_get("updated_at").unwrap_or_default(),
        })
    }

    /// Fetch the full workflow [`Finding`] by its stable id.
    pub async fn get_full(&self, id: &str) -> Result<Finding> {
        let row = sqlx::query("SELECT * FROM review_findings WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get full finding"))?;
        Self::row_to_full(&row)
    }

    /// List the full workflow findings for a review run (the board data).
    pub async fn list_full_for_review(&self, review_id: &str) -> Result<Vec<Finding>> {
        let rows =
            sqlx::query("SELECT * FROM review_findings WHERE review_id = ? ORDER BY created_at")
                .bind(review_id)
                .fetch_all(&self.pool)
                .await
                .map_err(dberr("list full findings"))?;
        rows.iter().map(Self::row_to_full).collect()
    }

    /// Transition a finding's workflow `status` (validated against the machine).
    /// `actor` becomes the finding's current `reviewer`. Same-status is an
    /// idempotent no-op; an illegal edge returns `Error::Invalid`.
    pub async fn set_status(&self, id: &str, to: FindingStatus, actor: &str) -> Result<Finding> {
        let cur = self.get_full(id).await?;
        if cur.status != to && !cur.status.can_transition(to) {
            return Err(Error::Invalid(format!(
                "illegal finding transition {} -> {}",
                cur.status.as_str(),
                to.as_str()
            )));
        }
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE review_findings SET status = ?, reviewer = ?, updated_at = ? WHERE id = ?",
        )
        .bind(to.as_str())
        .bind(actor)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set finding status"))?;
        self.get_full(id).await
    }

    /// Patch the workflow artifact/gate fields (each `Some` is written, `None`
    /// keeps the existing value via `COALESCE`). Never touches `status`.
    pub async fn set_fields(&self, id: &str, p: &FindingPatch) -> Result<Finding> {
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE review_findings SET \
             linked_commit = COALESCE(?, linked_commit), \
             linked_test = COALESCE(?, linked_test), \
             jira_key = COALESCE(?, jira_key), \
             jira_url = COALESCE(?, jira_url), \
             fix_session_id = COALESCE(?, fix_session_id), \
             repo_rule_id = COALESCE(?, repo_rule_id), \
             requires_human_approval = COALESCE(?, requires_human_approval), \
             approved_by = COALESCE(?, approved_by), \
             approved_at = COALESCE(?, approved_at), \
             approval_decision = COALESCE(?, approval_decision), \
             reviewer = COALESCE(?, reviewer), \
             produced_by_agent = COALESCE(?, produced_by_agent), \
             updated_at = ? \
             WHERE id = ?",
        )
        .bind(&p.linked_commit)
        .bind(&p.linked_test)
        .bind(&p.jira_key)
        .bind(&p.jira_url)
        .bind(&p.fix_session_id)
        .bind(&p.repo_rule_id)
        .bind(p.requires_human_approval.map(|b| i64::from(b)))
        .bind(&p.approved_by)
        .bind(&p.approved_at)
        .bind(&p.approval_decision)
        .bind(&p.reviewer)
        .bind(&p.produced_by_agent)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set finding fields"))?;
        self.get_full(id).await
    }
}

// ---------------------------------------------------------------------------
// Unit tests — lifecycle transitions + fingerprint stability
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable_across_identical_inputs() {
        let fp1 = compute_fingerprint("repo-abc", 42, Some("src/main.rs"), Some("bug"), "use of unsafe block");
        let fp2 = compute_fingerprint("repo-abc", 42, Some("src/main.rs"), Some("bug"), "use of unsafe block");
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn fingerprint_normalises_case_and_whitespace() {
        let fp1 = compute_fingerprint("repo-abc", 42, Some("src/main.rs"), Some("Bug"), "  Use of Unsafe Block  ");
        let fp2 = compute_fingerprint("repo-abc", 42, Some("src/main.rs"), Some("bug"), "use of unsafe block");
        assert_eq!(fp1, fp2, "fingerprints must be case-and-whitespace normalised");
    }

    #[test]
    fn fingerprint_differs_on_different_path() {
        let fp1 = compute_fingerprint("repo", 1, Some("a.rs"), None, "issue");
        let fp2 = compute_fingerprint("repo", 1, Some("b.rs"), None, "issue");
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn fingerprint_differs_on_different_pr() {
        let fp1 = compute_fingerprint("repo", 1, None, None, "issue");
        let fp2 = compute_fingerprint("repo", 2, None, None, "issue");
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn finding_state_round_trip() {
        for (s, expected) in &[
            ("open", FindingState::Open),
            ("fixing", FindingState::Fixing),
            ("resolved", FindingState::Resolved),
            ("regressed", FindingState::Regressed),
            ("declined", FindingState::Declined),
        ] {
            let parsed = FindingState::parse(s).expect("parseable");
            assert_eq!(&parsed, expected);
            assert_eq!(parsed.as_str(), *s);
        }
    }

    #[test]
    fn unknown_state_parses_to_none() {
        assert!(FindingState::parse("garbage").is_none());
    }

    async fn mem_pool() -> SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool
    }

    fn sample<'a>(fp: &'a str, severity: &'a str) -> NewFinding<'a> {
        NewFinding {
            review_id: "rev1",
            workspace_id: "ws1",
            repo_id: "repo1",
            pr_number: Some(7),
            path: Some("src/db.rs"),
            line: Some(42),
            line_end: Some(48),
            severity,
            category: Some("security"),
            title: "SQL injection",
            body: "format! into a query",
            evidence: "let q = format!(\"... {}\", name);",
            agent_reasoning_summary: "user input reaches the query unescaped",
            suggested_fix: Some("use a parameterized query"),
            produced_by_agent: Some("grill"),
            reviewer: "grill",
            fingerprint: fp,
            run_id: "run1",
        }
    }

    #[tokio::test]
    async fn upsert_enriches_and_dedups_then_workflow_status_survives() {
        let repo = ReviewFindingsRepo::new(mem_pool().await);

        // First sighting → created, all 11 fields populated, severity normalized.
        let (f, created) = repo.upsert(&sample("fp-aaa", "bug")).await.unwrap();
        assert!(created);
        assert_eq!(f.status, FindingStatus::Open);
        assert_eq!(f.severity, FindingSeverity::High); // bug -> high (normalized on write)
        assert_eq!(f.evidence, "let q = format!(\"... {}\", name);");
        assert_eq!(f.agent_reasoning_summary, "user input reaches the query unescaped");
        assert_eq!(f.suggested_fix.as_deref(), Some("use a parameterized query"));
        assert_eq!(f.line, Some(42));
        assert_eq!(f.line_end, Some(48));
        assert_eq!(f.reviewer, "grill");

        // Human triages it to `verified` (workflow axis).
        let f = repo.set_status(&f.id, FindingStatus::Accepted, "alice").await.unwrap();
        let f = repo.set_status(&f.id, FindingStatus::Verified, "alice").await.unwrap();
        assert_eq!(f.status, FindingStatus::Verified);
        assert_eq!(f.reviewer, "alice");

        // Re-review re-emits the SAME fingerprint → dedup (not created), and the
        // engine MUST NOT clobber the human's `verified` disposition (§15.1).
        let (f2, created2) = repo.upsert(&sample("fp-aaa", "bug")).await.unwrap();
        assert!(!created2);
        assert_eq!(f2.id, f.id);
        assert_eq!(f2.status, FindingStatus::Verified, "re-detection must not reset workflow status");
        assert_eq!(f2.occurrence_count, 2);
    }

    #[tokio::test]
    async fn set_status_rejects_illegal_transition() {
        let repo = ReviewFindingsRepo::new(mem_pool().await);
        let (f, _) = repo.upsert(&sample("fp-bbb", "warn")).await.unwrap();
        // open -> verified is illegal (must go via accepted/fixed)
        let err = repo.set_status(&f.id, FindingStatus::Verified, "u1").await;
        assert!(err.is_err(), "open -> verified must be rejected");
        // open -> accepted is legal
        assert!(repo.set_status(&f.id, FindingStatus::Accepted, "u1").await.is_ok());
    }

    #[tokio::test]
    async fn set_fields_patches_artifacts_only() {
        let repo = ReviewFindingsRepo::new(mem_pool().await);
        let (f, _) = repo.upsert(&sample("fp-ccc", "minor")).await.unwrap();
        let patched = repo
            .set_fields(
                &f.id,
                &FindingPatch {
                    linked_commit: Some("abc123".into()),
                    linked_test: Some("tests/db_test.rs::no_injection".into()),
                    requires_human_approval: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(patched.linked_commit.as_deref(), Some("abc123"));
        assert_eq!(patched.linked_test.as_deref(), Some("tests/db_test.rs::no_injection"));
        assert!(patched.requires_human_approval);
        // status untouched by set_fields
        assert_eq!(patched.status, FindingStatus::Open);
        // a second patch leaving linked_commit None keeps the old value (COALESCE)
        let again = repo
            .set_fields(&f.id, &FindingPatch { jira_key: Some("PROJ-1".into()), ..Default::default() })
            .await
            .unwrap();
        assert_eq!(again.linked_commit.as_deref(), Some("abc123"));
        assert_eq!(again.jira_key.as_deref(), Some("PROJ-1"));
    }

    #[tokio::test]
    async fn local_review_findings_dedup_within_review() {
        let repo = ReviewFindingsRepo::new(mem_pool().await);
        let mut nf = sample("fp-ddd", "info");
        nf.pr_number = None;
        nf.review_id = "local-rev-A";
        let (a, created_a) = repo.upsert(&nf).await.unwrap();
        assert!(created_a);
        // same fingerprint, DIFFERENT local review → a distinct finding (no cross-review dedup)
        let mut nf2 = sample("fp-ddd", "info");
        nf2.pr_number = None;
        nf2.review_id = "local-rev-B";
        let (b, created_b) = repo.upsert(&nf2).await.unwrap();
        assert!(created_b);
        assert_ne!(a.id, b.id);
    }
}
