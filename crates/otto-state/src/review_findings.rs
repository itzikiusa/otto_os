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
use otto_core::{new_id, Id, Result};

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
    pub pr_number: u64,
    pub path: Option<&'a str>,
    pub line: Option<i64>,
    pub severity: &'a str,
    pub category: Option<&'a str>,
    pub body: &'a str,
    /// sha2 hex fingerprint — computed by `compute_fingerprint`.
    pub fingerprint: &'a str,
    /// The review run id used to populate first_seen_run / last_seen_run.
    pub run_id: &'a str,
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

    /// Upsert a finding from the current run into the database, applying
    /// lifecycle transitions. Returns the upserted row.
    ///
    /// Transition rules:
    /// - No existing row for this fingerprint → INSERT with state `open`.
    /// - Existing row with state `open` or `fixing` → UPDATE `last_seen_run`,
    ///   leave state unchanged (still actively open).
    /// - Existing row with state `resolved` or `declined` → UPDATE to `regressed`.
    pub async fn upsert(&self, f: &NewFinding<'_>) -> Result<ReviewFindingRow> {
        let now = fmt(Utc::now());

        // Check whether a row with this fingerprint already exists for the
        // (workspace, repo, pr) triple.
        let existing: Option<sqlx::sqlite::SqliteRow> = sqlx::query(
            "SELECT id, state, first_seen_run FROM review_findings \
             WHERE workspace_id = ? AND repo_id = ? AND pr_number = ? AND fingerprint = ? \
             LIMIT 1",
        )
        .bind(f.workspace_id)
        .bind(f.repo_id)
        .bind(f.pr_number as i64)
        .bind(f.fingerprint)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("upsert finding lookup"))?;

        if let Some(row) = existing {
            let existing_id: String = row.get("id");
            let prev_state: String =
                row.try_get("state").unwrap_or_else(|_| "open".to_string());
            // Reappearance of a resolved/declined finding → regressed.
            let next_state = match prev_state.as_str() {
                "resolved" | "declined" => "regressed",
                other => other, // open / fixing / regressed — keep as-is
            };
            sqlx::query(
                "UPDATE review_findings \
                 SET state = ?, review_id = ?, last_seen_run = ?, updated_at = ? \
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

            let updated = sqlx::query(
                "SELECT * FROM review_findings WHERE id = ?",
            )
            .bind(&existing_id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("upsert finding fetch"))?;
            Self::row_to_finding(&updated)
        } else {
            // Brand-new finding for this fingerprint.
            let id = new_id();
            sqlx::query(
                "INSERT INTO review_findings \
                 (id, workspace_id, repo_id, pr_number, review_id, fingerprint, \
                  path, line, severity, category, body, status, state, \
                  first_seen_review_id, last_seen_review_id, first_seen_run, last_seen_run, \
                  occurrence_count, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'open', 'open', ?, ?, ?, ?, 1, ?, ?)",
            )
            .bind(&id)
            .bind(f.workspace_id)
            .bind(f.repo_id)
            .bind(f.pr_number as i64)
            .bind(f.review_id)
            .bind(f.fingerprint)
            .bind(f.path)
            .bind(f.line)
            .bind(f.severity)
            .bind(f.category)
            .bind(f.body)
            .bind(f.review_id) // first_seen_review_id
            .bind(f.review_id) // last_seen_review_id
            .bind(f.run_id)    // first_seen_run
            .bind(f.run_id)    // last_seen_run
            .bind(&now)
            .bind(&now)
            .execute(&self.pool)
            .await
            .map_err(dberr("upsert finding insert"))?;

            let inserted = sqlx::query(
                "SELECT * FROM review_findings WHERE id = ?",
            )
            .bind(&id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("upsert finding fetch after insert"))?;
            Self::row_to_finding(&inserted)
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
}
