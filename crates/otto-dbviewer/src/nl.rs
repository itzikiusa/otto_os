//! Verified natural-language → SQL. The model drafts a candidate query; we
//! reject writes, validate the candidate with `EXPLAIN` against the live schema,
//! and feed any engine error back to the model for a bounded retry — so the user
//! only ever receives a query that has been proven to parse and run as a read.
//!
//! The model call itself is NOT here: this module is generic over a [`SqlDrafter`]
//! (wired to the agent/LLM in `otto-server`) and a [`SqlValidator`] (the
//! `EXPLAIN` runner, backed by `DbViewerService`), so the loop is unit-testable
//! with stubs and `otto-dbviewer` keeps no agent dependency.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use otto_core::{Error, Result};

use crate::types::{statement_is_write, Engine};

/// Everything the drafter needs to write one candidate query: the engine, a
/// compact schema summary to ground it, the user's question, and — on a retry —
/// the previous attempt and the engine error it produced.
#[derive(Debug, Clone)]
pub struct DraftContext {
    pub engine: Engine,
    pub schema_summary: String,
    pub question: String,
    pub prior: Option<FailedAttempt>,
}

/// A candidate that failed validation (or was a write), fed back to the drafter.
#[derive(Debug, Clone)]
pub struct FailedAttempt {
    pub sql: String,
    pub error: String,
}

/// The result handed back to the UI: a validated read query, its plan text, how
/// many drafting attempts it took, and any non-fatal notes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NlToSqlOutcome {
    pub sql: String,
    pub plan: String,
    pub attempts: u32,
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// Pull the runnable statement out of a model reply: unwrap a ```` ```sql ```` /
/// bare ```` ``` ```` fence if present, else take the trimmed text verbatim.
pub fn extract_sql(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(start) = trimmed.find("```") {
        // Skip the opening fence + optional language tag up to the newline.
        let after = &trimmed[start + 3..];
        let body = after.split_once('\n').map(|(_, b)| b).unwrap_or(after);
        if let Some(end) = body.find("```") {
            return body[..end].trim().to_string();
        }
        return body.trim().to_string();
    }
    trimmed.to_string()
}

/// Produces a candidate query for a [`DraftContext`]. Implemented in `otto-server`
/// by an adapter that calls the agent/LLM; stubbed in tests.
#[async_trait]
pub trait SqlDrafter: Send + Sync {
    async fn draft(&self, ctx: &DraftContext) -> Result<String>;
}

/// Validates a candidate read query by running its plan (`EXPLAIN`). `Ok` carries
/// human-readable plan text; `Err` carries the engine's error message (fed back
/// to the drafter on the next attempt). Implemented by `DbViewerService`.
#[async_trait]
pub trait SqlValidator: Send + Sync {
    async fn validate(&self, sql: &str) -> std::result::Result<String, String>;
}

/// Drive the draft → reject-writes → `EXPLAIN`-validate → retry loop.
///
/// On each attempt the drafter is given the question, the schema summary, and the
/// previous failed attempt (if any). A candidate classified as a write by
/// [`statement_is_write`] is **never executed** — it's recorded as a failed
/// attempt and the loop retries; if every attempt is a write, the call fails
/// closed with a read-only error. A candidate that validates is returned with its
/// plan. After `max_attempts` validation failures the call returns the last
/// engine error so the UI can show why.
pub async fn drive_nl_to_sql(
    engine: Engine,
    question: &str,
    schema_summary: &str,
    drafter: &dyn SqlDrafter,
    validator: &dyn SqlValidator,
    max_attempts: u32,
) -> Result<NlToSqlOutcome> {
    let max_attempts = max_attempts.clamp(1, 4);
    let mut prior: Option<FailedAttempt> = None;
    let mut warnings: Vec<String> = Vec::new();
    let mut last_error = "the model did not produce a runnable read query".to_string();

    for attempt in 1..=max_attempts {
        let ctx = DraftContext {
            engine,
            schema_summary: schema_summary.to_string(),
            question: question.to_string(),
            prior: prior.clone(),
        };
        let raw = drafter.draft(&ctx).await?;
        let sql = extract_sql(&raw);
        if sql.is_empty() {
            last_error = "the model returned an empty query".to_string();
            prior = Some(FailedAttempt {
                sql,
                error: last_error.clone(),
            });
            continue;
        }
        // Read-only contract: never send a write/DDL to the engine.
        if statement_is_write(engine, &sql) {
            last_error =
                "NL→SQL only produces read queries, but the draft was a write/DDL".to_string();
            warnings.push(format!("attempt {attempt}: discarded a non-read draft"));
            prior = Some(FailedAttempt {
                sql,
                error: last_error.clone(),
            });
            continue;
        }
        match validator.validate(&sql).await {
            Ok(plan) => {
                return Ok(NlToSqlOutcome {
                    sql,
                    plan,
                    attempts: attempt,
                    warnings,
                });
            }
            Err(e) => {
                last_error = e.clone();
                prior = Some(FailedAttempt { sql, error: e });
            }
        }
    }

    Err(Error::Invalid(format!(
        "could not produce a valid read query after {max_attempts} attempts; last error: {last_error}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use std::sync::Mutex;

    #[test]
    fn extract_sql_strips_markdown_fences_and_prose() {
        // Bare SQL passes through untouched.
        assert_eq!(extract_sql("SELECT 1"), "SELECT 1");
        // A ```sql fence is unwrapped.
        assert_eq!(
            extract_sql("Here you go:\n```sql\nSELECT * FROM t\n```"),
            "SELECT * FROM t"
        );
        // A bare ``` fence (no language) is unwrapped too.
        assert_eq!(extract_sql("```\nSELECT 2\n```"), "SELECT 2");
        // Leading/trailing whitespace and a trailing semicolon-free body trim.
        assert_eq!(extract_sql("  SELECT 3  "), "SELECT 3");
    }

    /// A drafter that returns a fixed sequence of candidates (one per attempt),
    /// recording how many times it was called and the prior error it last saw.
    struct ScriptedDrafter {
        replies: Vec<String>,
        calls: Mutex<usize>,
        last_prior_error: Mutex<Option<String>>,
    }
    impl ScriptedDrafter {
        fn new(replies: &[&str]) -> Self {
            Self {
                replies: replies.iter().map(|s| s.to_string()).collect(),
                calls: Mutex::new(0),
                last_prior_error: Mutex::new(None),
            }
        }
    }
    #[async_trait]
    impl SqlDrafter for ScriptedDrafter {
        async fn draft(&self, ctx: &DraftContext) -> otto_core::Result<String> {
            let mut n = self.calls.lock().unwrap();
            *self.last_prior_error.lock().unwrap() = ctx.prior.as_ref().map(|p| p.error.clone());
            let reply = self.replies.get(*n).cloned().unwrap_or_default();
            *n += 1;
            Ok(reply)
        }
    }

    /// A validator that accepts a configured "good" SQL and rejects everything
    /// else with a fixed engine error.
    struct GoodIff(&'static str);
    #[async_trait]
    impl SqlValidator for GoodIff {
        async fn validate(&self, sql: &str) -> std::result::Result<String, String> {
            if sql == self.0 {
                Ok(format!("PLAN FOR: {sql}"))
            } else {
                Err("Unknown column 'nope' in 'field list'".to_string())
            }
        }
    }

    #[tokio::test]
    async fn returns_validated_sql_on_first_try() {
        let drafter = ScriptedDrafter::new(&["SELECT id FROM users"]);
        let validator = GoodIff("SELECT id FROM users");
        let out = drive_nl_to_sql(Engine::Mysql, "ids", "", &drafter, &validator, 3)
            .await
            .expect("ok");
        assert_eq!(out.sql, "SELECT id FROM users");
        assert_eq!(out.attempts, 1);
        assert!(out.plan.contains("PLAN FOR"));
        assert!(*drafter.calls.lock().unwrap() == 1);
    }

    #[tokio::test]
    async fn self_corrects_after_an_engine_error() {
        // First draft is wrong (validator rejects); second draft is right. The
        // drafter must have seen the engine error on its second call.
        let drafter = ScriptedDrafter::new(&[
            "SELECT nope FROM users",
            "```sql\nSELECT id FROM users\n```", // fenced — extractor must unwrap
        ]);
        let validator = GoodIff("SELECT id FROM users");
        let out = drive_nl_to_sql(Engine::Mysql, "ids", "", &drafter, &validator, 3)
            .await
            .expect("ok");
        assert_eq!(out.sql, "SELECT id FROM users");
        assert_eq!(out.attempts, 2);
        assert_eq!(
            drafter.last_prior_error.lock().unwrap().as_deref(),
            Some("Unknown column 'nope' in 'field list'")
        );
    }

    #[tokio::test]
    async fn rejects_a_write_without_running_it() {
        // The drafter returns a DELETE; the loop must NOT call the validator for
        // it (we never send a write to the engine), and must surface a warning.
        struct NeverValidates;
        #[async_trait]
        impl SqlValidator for NeverValidates {
            async fn validate(&self, _sql: &str) -> std::result::Result<String, String> {
                panic!("a write must never reach the validator");
            }
        }
        let drafter = ScriptedDrafter::new(&["DELETE FROM users", "DROP TABLE users"]);
        let err = drive_nl_to_sql(Engine::Mysql, "remove all", "", &drafter, &NeverValidates, 2)
            .await
            .expect_err("write-only drafts must fail closed");
        assert!(err.to_string().to_lowercase().contains("read"));
    }

    #[tokio::test]
    async fn gives_up_after_max_attempts_with_last_error() {
        let drafter = ScriptedDrafter::new(&["SELECT bad1", "SELECT bad2", "SELECT bad3"]);
        let validator = GoodIff("SELECT good"); // never matches
        let err = drive_nl_to_sql(Engine::Mysql, "q", "", &drafter, &validator, 3)
            .await
            .expect_err("exhausted");
        assert!(*drafter.calls.lock().unwrap() == 3);
        assert!(err.to_string().contains("Unknown column"));
    }
}
