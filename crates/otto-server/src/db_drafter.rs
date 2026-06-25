//! Wires the DB Explorer's verified NL→SQL drafter (`otto_dbviewer::nl::SqlDrafter`)
//! to the real agent/LLM via the orchestrator's one-shot text path — the same
//! `Orchestrator::run_agent` call the commit-/PR-draft endpoints use.
//!
//! The `otto-dbviewer` crate keeps no agent dependency: it defines the
//! [`SqlDrafter`](otto_dbviewer::nl::SqlDrafter) trait + the
//! draft→reject-writes→`EXPLAIN`-validate→retry loop, and this adapter supplies
//! the model call. `ServerCtx::drafter()` hands one of these back so the
//! `nl-to-sql` route is live (the trait's default is `None`, which 400s).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use otto_dbviewer::nl::{DraftContext, SqlDrafter};
use otto_orchestrator::Orchestrator;

/// Same generous stuck-window the other one-shot agent endpoints use (a cold
/// claude spawn alone is ~25-30s). Not a wall-clock cap — only a stall this
/// long with no progress is treated as wedged.
const NO_PROGRESS: Duration = Duration::from_secs(150);

/// Adapter that drafts one candidate query per [`DraftContext`] by running a
/// single headless agent turn. Held behind `Arc<dyn SqlDrafter>` on `ServerCtx`.
pub struct AgentSqlDrafter {
    pub orchestrator: Arc<Orchestrator>,
    /// Working directory for the one-shot planner turn. The drafter has no repo
    /// or connection context, so this is just a neutral, always-valid directory
    /// captured at construction — the NL→SQL planner call reads no repo files;
    /// it is grounded entirely by the schema summary in the prompt.
    pub cwd: String,
}

#[async_trait]
impl SqlDrafter for AgentSqlDrafter {
    async fn draft(&self, ctx: &DraftContext) -> otto_core::Result<String> {
        // Human-readable dialect name so the model writes the right syntax.
        let dialect = match ctx.engine.as_str() {
            "mysql" => "MySQL",
            "clickhouse" => "ClickHouse SQL",
            "mongodb" => "a MongoDB shell expression (db.coll.find/aggregate)",
            other => other,
        };

        // Ground the model in the real schema and demand a single read-only
        // statement, output verbatim with no prose.
        let mut prompt = format!(
            "You are a {dialect} query author. Output ONLY the query, no prose. \
             It MUST be a read (SELECT / find / aggregate) — never \
             INSERT/UPDATE/DELETE/DROP/DDL. Use ONLY tables and columns from this \
             schema:\n{schema}\n\nQuestion: {question}",
            schema = ctx.schema_summary,
            question = ctx.question,
        );

        // On a retry, include the prior attempt + engine error so the model can
        // self-correct against what actually failed.
        if let Some(prior) = &ctx.prior {
            prompt.push_str(&format!(
                "\nYour previous attempt failed. Fix it.\nPrevious query:\n{sql}\n\
                 Engine error:\n{err}\n",
                sql = prior.sql,
                err = prior.error,
            ));
        }

        // One-shot agent text turn (mirrors `draft_commit_message`). The result
        // is already `otto_core::Result<String>`, matching the trait return.
        self.orchestrator
            .run_agent(&prompt, &self.cwd, None, NO_PROGRESS)
            .await
    }
}
