# Verified NL→SQL Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a user type a question in plain English and get back a **read** SQL/Mongo query that the agent has already **validated with `EXPLAIN`** against the live schema — never a hallucinated, unrunnable guess.

**Architecture:** A pure, trait-generic loop (`drive_nl_to_sql`) lives in `otto-dbviewer` and is unit-tested with stubs. It asks an injected `SqlDrafter` for candidate SQL, rejects anything the existing `statement_is_write` classifier flags (this feature only emits reads), validates the candidate through an injected `SqlValidator` (which runs `EXPLAIN` via `DbViewerService`), and on a validation error feeds the error back to the drafter for a bounded number of retries. The **LLM call stays in `otto-server`** behind a `SqlDrafter` the `DbViewerCtx` exposes — exactly the decoupling pattern `on_confirmed_write` already uses — so `otto-dbviewer` keeps no agent dependency and stays testable.

**Tech Stack:** Rust (axum, `async-trait`, `serde`, `tokio`), the existing `otto-dbviewer` `Driver`/`DbViewerService`/`DbViewerCtx` surface, Svelte 5 + TypeScript UI.

## Global Constraints

- **Read-only output.** The loop must reject any candidate that `statement_is_write(engine, sql)` classifies as a write/DDL. NL→SQL never emits a mutation. (Classifier is conservative: unknown ⇒ write.)
- **No new agent dependency in `otto-dbviewer`.** The drafter is a trait; the model call is wired in `otto-server` only.
- **Validation runs through the existing guarded path.** `EXPLAIN` is a read, so it passes `guard_write` even on a Prod/read-only connection — but the loop still classifies the candidate as a read *before* validating, so a write is never sent to the engine at all.
- **Role gate = `Editor`** (global connections: root) — same as `run_query`, because validation executes `EXPLAIN` against the live database. Use `check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor)`.
- **Contract lockstep.** New endpoint ⇒ update `docs/contracts/api.md`, `ui/src/lib/api/types.ts`, and `docs/features/database-explorer.md` together.
- **`max_attempts` is clamped** to `1..=4` server-side (default 3) so a misbehaving drafter can't loop expensively.
- Match surrounding code: dense, documented, `#[async_trait]` for async traits, `ApiErr`/`Problem` error mapping.

---

### Task 1: `nl` module — types + the SQL extractor

**Files:**
- Create: `crates/otto-dbviewer/src/nl.rs`
- Modify: `crates/otto-dbviewer/src/lib.rs` (add `pub mod nl;`)

**Interfaces:**
- Produces: `nl::DraftContext`, `nl::FailedAttempt`, `nl::NlToSqlOutcome`, `nl::extract_sql(&str) -> String`.

- [ ] **Step 1: Write the failing test for the extractor**

Add to `crates/otto-dbviewer/src/nl.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

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
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p otto-dbviewer nl::tests::extract_sql_strips_markdown_fences_and_prose`
Expected: FAIL — `cannot find function extract_sql`.

- [ ] **Step 3: Write the module header, types, and `extract_sql`**

At the top of `crates/otto-dbviewer/src/nl.rs`:

```rust
//! Verified natural-language → SQL. The model drafts a candidate query; we
//! reject writes, validate the candidate with `EXPLAIN` against the live schema,
//! and feed any engine error back to the model for a bounded retry — so the user
//! only ever receives a query that has been proven to parse and run as a read.
//!
//! The model call itself is NOT here: this module is generic over a [`SqlDrafter`]
//! (wired to the agent/LLM in `otto-server`) and a [`SqlValidator`] (the
//! `EXPLAIN` runner, backed by `DbViewerService`), so the loop is unit-testable
//! with stubs and `otto-dbviewer` keeps no agent dependency.

use serde::{Deserialize, Serialize};

use crate::types::Engine;

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
```

Then register the module — in `crates/otto-dbviewer/src/lib.rs` add (alongside the other `pub mod` lines):

```rust
pub mod nl;
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p otto-dbviewer nl::tests::extract_sql_strips_markdown_fences_and_prose`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otto-dbviewer/src/nl.rs crates/otto-dbviewer/src/lib.rs
git commit -m "feat(db): nl module skeleton + markdown-fence SQL extractor"
```

---

### Task 2: the verified-draft loop (`drive_nl_to_sql`)

**Files:**
- Modify: `crates/otto-dbviewer/src/nl.rs`

**Interfaces:**
- Consumes: `extract_sql`, `crate::types::statement_is_write`, `crate::types::Engine`.
- Produces:
  - `#[async_trait] trait SqlDrafter { async fn draft(&self, ctx: &DraftContext) -> otto_core::Result<String>; }`
  - `#[async_trait] trait SqlValidator { async fn validate(&self, sql: &str) -> std::result::Result<String, String>; }` (Ok = plan text, Err = engine error message)
  - `async fn drive_nl_to_sql(engine: Engine, question: &str, schema_summary: &str, drafter: &dyn SqlDrafter, validator: &dyn SqlValidator, max_attempts: u32) -> otto_core::Result<NlToSqlOutcome>`

- [ ] **Step 1: Write the failing tests for the loop**

Add to the `tests` module in `crates/otto-dbviewer/src/nl.rs`:

```rust
    use std::sync::Mutex;
    use async_trait::async_trait;

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
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p otto-dbviewer nl::tests`
Expected: FAIL — `SqlDrafter` / `SqlValidator` / `drive_nl_to_sql` not found.

- [ ] **Step 3: Implement the traits and the loop**

Add to `crates/otto-dbviewer/src/nl.rs` (above the `tests` module):

```rust
use async_trait::async_trait;
use otto_core::{Error, Result};

use crate::types::statement_is_write;

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
    let mut last_error =
        "the model did not produce a runnable read query".to_string();

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
            prior = Some(FailedAttempt { sql, error: last_error.clone() });
            continue;
        }
        // Read-only contract: never send a write/DDL to the engine.
        if statement_is_write(engine, &sql) {
            last_error =
                "NL→SQL only produces read queries, but the draft was a write/DDL".to_string();
            warnings.push(format!("attempt {attempt}: discarded a non-read draft"));
            prior = Some(FailedAttempt { sql, error: last_error.clone() });
            continue;
        }
        match validator.validate(&sql).await {
            Ok(plan) => {
                return Ok(NlToSqlOutcome { sql, plan, attempts: attempt, warnings });
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p otto-dbviewer nl::tests`
Expected: PASS (all four loop tests + the extractor test).

- [ ] **Step 5: Commit**

```bash
git add crates/otto-dbviewer/src/nl.rs
git commit -m "feat(db): verified NL->SQL loop (reject writes, EXPLAIN-validate, bounded retry)"
```

---

### Task 3: `DbViewerService` as the validator + schema summary

**Files:**
- Modify: `crates/otto-dbviewer/src/service.rs`

**Interfaces:**
- Consumes: `self.resolve`, `self.run`, `crate::types::{QueryRequest, Engine}`, `crate::nl::SqlValidator`.
- Produces:
  - `DbViewerService::explain_validate(&self, conn_id: &Id, user_id: &Id, node: Option<&str>, sql: &str) -> std::result::Result<String, String>`
  - `DbViewerService::schema_summary(&self, conn_id: &Id, node: Option<&str>, max_tables: usize) -> Result<String>`
  - `struct ServiceValidator<'a> { db: &'a DbViewerService, conn_id: Id, user_id: Id, node: Option<String> }` implementing `crate::nl::SqlValidator`.

- [ ] **Step 1: Add `explain_validate` to the service**

In `crates/otto-dbviewer/src/service.rs`, inside `impl DbViewerService`, after `run`:

```rust
    /// Validate a candidate **read** query by asking the engine for its plan,
    /// without returning rows. SQL engines run `EXPLAIN <sql>`; Mongo runs with
    /// the `explain` flag. Returns the plan rendered as text on success, or the
    /// engine's error message on failure (so the NL loop can feed it back to the
    /// drafter). The caller has already classified `sql` as a read.
    pub async fn explain_validate(
        &self,
        conn_id: &Id,
        user_id: &Id,
        node: Option<&str>,
        sql: &str,
    ) -> std::result::Result<String, String> {
        let conn = self.connections.get(conn_id).await.map_err(|e| e.to_string())?;
        let engine = Engine::from_kind(conn.kind)
            .ok_or_else(|| "connection is not a browsable database".to_string())?;
        let req = match engine {
            Engine::Mysql | Engine::Clickhouse => QueryRequest {
                statement: format!("EXPLAIN {sql}"),
                node: node.map(str::to_string),
                ..QueryRequest::default()
            },
            Engine::Mongodb => QueryRequest {
                statement: sql.to_string(),
                explain: true,
                node: node.map(str::to_string),
                ..QueryRequest::default()
            },
            // Redis has no SQL/plan surface; NL→SQL is gated off for it in the UI.
            Engine::Redis => return Err("EXPLAIN is not supported for Redis".to_string()),
        };
        match self.run(conn_id, user_id, &req).await {
            Ok(res) => Ok(render_plan_text(&res)),
            Err(e) => Err(e.to_string()),
        }
    }
```

And a small free function near the bottom of the file (before `#[cfg(test)]`):

```rust
/// Render a plan `QueryResult` as compact text for display + drafter feedback.
fn render_plan_text(res: &crate::types::QueryResult) -> String {
    let mut out = String::new();
    for row in &res.rows {
        let line: Vec<String> = row
            .iter()
            .map(|c| match c {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect();
        out.push_str(&line.join(" | "));
        out.push('\n');
    }
    if out.is_empty() {
        out.push_str("(empty plan)");
    }
    out
}
```

- [ ] **Step 2: Add `schema_summary` to the service**

In the same `impl`, add:

```rust
    /// A compact, model-grounding summary of the schema under `node` (or the
    /// connection's default): up to `max_tables` tables, each as
    /// `table(col type, col type, …)`. Built from the same lazy tree the UI
    /// browses, so it's engine-agnostic. Best-effort — an object that fails to
    /// introspect is skipped.
    pub async fn schema_summary(
        &self,
        conn_id: &Id,
        node: Option<&str>,
        max_tables: usize,
    ) -> Result<String> {
        let schema = node
            .and_then(|n| NodePath::parse(n).get("db").map(str::to_string))
            .unwrap_or_default();
        let graph = self.schema_graph(conn_id, &schema, max_tables).await?;
        let mut out = String::new();
        for t in &graph.tables {
            let cols: Vec<String> = t
                .columns
                .iter()
                .map(|c| format!("{} {}", c.name, c.data_type))
                .collect();
            out.push_str(&format!("{}({})\n", t.name, cols.join(", ")));
        }
        if out.is_empty() {
            out.push_str("(no tables introspected)");
        }
        Ok(out)
    }
```

> Note: `schema_summary` reuses `schema_graph`, which already enumerates tables +
> columns with bounded concurrency and a table cap — no new introspection code.

- [ ] **Step 3: Add the `ServiceValidator` adapter implementing `SqlValidator`**

At the bottom of `service.rs` (outside `impl`, before tests):

```rust
/// Adapts `DbViewerService::explain_validate` to the `nl::SqlValidator` trait so
/// the NL loop can validate candidates without knowing about the service.
pub struct ServiceValidator<'a> {
    pub db: &'a DbViewerService,
    pub conn_id: Id,
    pub user_id: Id,
    pub node: Option<String>,
}

#[async_trait::async_trait]
impl crate::nl::SqlValidator for ServiceValidator<'_> {
    async fn validate(&self, sql: &str) -> std::result::Result<String, String> {
        self.db
            .explain_validate(&self.conn_id, &self.user_id, self.node.as_deref(), sql)
            .await
    }
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build -p otto-dbviewer`
Expected: builds clean (no test added here — these methods need a live service; they're exercised by the component test in Task 9 and by the loop tests via the stub validator).

- [ ] **Step 5: Commit**

```bash
git add crates/otto-dbviewer/src/service.rs
git commit -m "feat(db): EXPLAIN-based SqlValidator + schema_summary on DbViewerService"
```

---

### Task 4: `DbViewerCtx` drafter accessor + the route

**Files:**
- Modify: `crates/otto-dbviewer/src/http.rs`

**Interfaces:**
- Consumes: `crate::nl::{SqlDrafter, drive_nl_to_sql, NlToSqlOutcome}`, `crate::service::ServiceValidator`, `check_conn_role`.
- Produces:
  - `DbViewerCtx::drafter(&self) -> Option<std::sync::Arc<dyn crate::nl::SqlDrafter>>` (default `None`).
  - Route `POST /connections/{id}/db/nl-to-sql` → `nl_to_sql::<S>`.
  - `struct NlToSqlReq { question: String, node: Option<String>, max_attempts: Option<u32> }`.

- [ ] **Step 1: Add the drafter accessor to `DbViewerCtx`**

In `crates/otto-dbviewer/src/http.rs`, extend the `DbViewerCtx` trait (after `on_confirmed_write`):

```rust
    /// The natural-language→SQL drafter, when the server has configured one
    /// (it wires this to the agent/LLM). Default `None` keeps this crate free of
    /// any agent dependency; the `nl-to-sql` route returns a clear 400 when it's
    /// unset. Mirrors the `db()`/`roles()` accessor style.
    fn drafter(&self) -> Option<std::sync::Arc<dyn crate::nl::SqlDrafter>> {
        None
    }
```

- [ ] **Step 2: Register the route**

In `api_router::<S>()`, add alongside the connection-scoped routes (after `db/completion`):

```rust
        .route("/connections/{id}/db/nl-to-sql", post(nl_to_sql::<S>))
```

- [ ] **Step 3: Add the request type and the handler**

Add near the other request structs:

```rust
#[derive(Debug, Deserialize)]
struct NlToSqlReq {
    /// The user's plain-English question.
    question: String,
    /// Optional active-database node (same semantics as `QueryRequest.node`).
    #[serde(default)]
    node: Option<String>,
    /// Draft/validate retries; clamped 1..=4 server-side (default 3).
    #[serde(default)]
    max_attempts: Option<u32>,
}
```

And the handler near `run_query`:

```rust
/// Draft a **read** query from natural language and return it only after it has
/// been validated with `EXPLAIN` against the live schema. Gated at `Editor`
/// (global connections: root) — it runs `EXPLAIN` against the database. Returns
/// 400 when no drafter is configured.
async fn nl_to_sql<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<NlToSqlReq>,
) -> ApiResult<Response> {
    let conn = ctx.db().get_connection(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor).await?;

    let engine = Engine::from_kind(conn.kind)
        .ok_or_else(|| Error::Invalid("connection is not a browsable database".into()))?;
    let drafter = ctx
        .drafter()
        .ok_or_else(|| Error::Invalid("NL-to-SQL is not configured on this server".into()))?;

    let summary = ctx
        .db()
        .schema_summary(&id, req.node.as_deref(), 40)
        .await
        .unwrap_or_default();

    let validator = crate::service::ServiceValidator {
        db: ctx.db(),
        conn_id: id.clone(),
        user_id: user.id.clone(),
        node: req.node.clone(),
    };

    let outcome = crate::nl::drive_nl_to_sql(
        engine,
        &req.question,
        &summary,
        drafter.as_ref(),
        &validator,
        req.max_attempts.unwrap_or(3),
    )
    .await?;

    Ok(Json(outcome).into_response())
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build -p otto-dbviewer`
Expected: builds clean.

- [ ] **Step 5: Add a role-gate test (reuse the existing `StubRoles`/`TestCtx`)**

In the `http.rs` `tests` module, the `TestCtx` already satisfies `DbViewerCtx` via the defaults — `drafter()` defaults to `None`, so no change is needed there. Add:

```rust
    #[tokio::test]
    async fn nl_to_sql_gate_requires_editor() {
        // The NL→SQL route shares run_query's gate: a Viewer is denied Editor.
        let stub = Arc::new(StubRoles::new(WorkspaceRole::Viewer));
        let ctx = TestCtx { roles: stub.clone() };
        let c = conn(Some(new_id()));
        let err = check_conn_role(&ctx, &user(false), &c, WorkspaceRole::Editor)
            .await
            .expect_err("viewer denied");
        assert!(matches!(err, Error::Forbidden(_)));
        assert_eq!(*stub.last_min.lock().unwrap(), Some(WorkspaceRole::Editor));
    }
```

- [ ] **Step 6: Run the gate test**

Run: `cargo test -p otto-dbviewer http::tests::nl_to_sql_gate_requires_editor`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/otto-dbviewer/src/http.rs
git commit -m "feat(db): POST /db/nl-to-sql route + DbViewerCtx drafter accessor"
```

---

### Task 5: Wire the real drafter in `otto-server`

**Files:**
- Modify: `crates/otto-server/src/modules.rs` (where `ServerCtx` implements `DbViewerCtx` and where `db/explain-with-agent` already spawns an agent — same neighbourhood)
- Create: `crates/otto-server/src/db_drafter.rs`

**Interfaces:**
- Consumes: `otto_dbviewer::nl::{SqlDrafter, DraftContext}`, the server's existing agent/LLM client used by `explain-with-agent`.
- Produces: `db_drafter::AgentSqlDrafter` implementing `SqlDrafter`; `ServerCtx::drafter()` returning `Some(Arc::new(AgentSqlDrafter{…}))`.

> **Why this task is described, not coded line-for-line:** the exact agent/LLM
> client call lives in `otto-server` and is the same one `db/explain-with-agent`
> already uses. The implementer wires `AgentSqlDrafter::draft` to that client. The
> *prompt* is fully specified below; the transport is whatever
> `explain-with-agent` already calls.

- [ ] **Step 1: Write the drafter with the grounding prompt**

Create `crates/otto-server/src/db_drafter.rs`:

```rust
//! Wires the DB Explorer's NL→SQL drafter to the agent/LLM. Kept in otto-server
//! so otto-dbviewer stays agent-free. The prompt grounds the model in the real
//! schema and demands a single read-only statement; on a retry it includes the
//! prior attempt and the exact engine error so the model self-corrects.

use std::sync::Arc;

use async_trait::async_trait;
use otto_core::Result;
use otto_dbviewer::nl::{DraftContext, SqlDrafter};

pub struct AgentSqlDrafter {
    /// The same model/agent client `explain-with-agent` uses (injected by ServerCtx).
    pub client: Arc<dyn crate::AgentTextClient>, // existing server trait/alias
}

#[async_trait]
impl SqlDrafter for AgentSqlDrafter {
    async fn draft(&self, ctx: &DraftContext) -> Result<String> {
        let dialect = match ctx.engine.as_str() {
            "mysql" => "MySQL",
            "clickhouse" => "ClickHouse SQL",
            "mongodb" => "a MongoDB shell expression (db.coll.find/aggregate)",
            other => other,
        };
        let mut prompt = format!(
            "You write a SINGLE read-only query in {dialect}. Output ONLY the query, no prose.\n\
             It MUST be a read (SELECT / find / aggregate) — never INSERT/UPDATE/DELETE/DROP/DDL.\n\
             Use ONLY tables and columns from this schema:\n{schema}\n\nQuestion: {q}\n",
            schema = ctx.schema_summary,
            q = ctx.question,
        );
        if let Some(prior) = &ctx.prior {
            prompt.push_str(&format!(
                "\nYour previous attempt failed. Fix it.\nPrevious query:\n{}\nEngine error:\n{}\n",
                prior.sql, prior.error
            ));
        }
        // Replace with the actual call the server already uses for explain-with-agent.
        self.client.complete(&prompt).await
    }
}
```

> The `crate::AgentTextClient` name is a placeholder for whatever single-shot
> text client `explain-with-agent` uses — the implementer substitutes the real
> type/method and adjusts the `use`. The **prompt is the deliverable** here.

- [ ] **Step 2: Return it from `ServerCtx::drafter()`**

In `crates/otto-server/src/modules.rs`, in `impl DbViewerCtx for ServerCtx`, add:

```rust
    fn drafter(&self) -> Option<std::sync::Arc<dyn otto_dbviewer::nl::SqlDrafter>> {
        Some(std::sync::Arc::new(crate::db_drafter::AgentSqlDrafter {
            client: self.agent_text_client(), // the existing accessor explain-with-agent uses
        }))
    }
```

And register the module in `crates/otto-server/src/lib.rs` (or wherever modules are declared): `mod db_drafter;`.

- [ ] **Step 3: Verify it compiles**

Run: `cargo build -p otto-server`
Expected: builds clean once `AgentTextClient`/`agent_text_client()` are pointed at the real server types.

- [ ] **Step 4: Commit**

```bash
git add crates/otto-server/src/db_drafter.rs crates/otto-server/src/modules.rs crates/otto-server/src/lib.rs
git commit -m "feat(server): wire NL->SQL drafter to the agent/LLM client"
```

---

### Task 6: Contract + TypeScript types

**Files:**
- Modify: `docs/contracts/api.md`
- Modify: `ui/src/lib/api/types.ts`
- Modify: `ui/src/lib/api/*` (the DB client module — same file the other `db/*` calls live in)

**Interfaces:**
- Produces: TS types `NlToSqlReq`, `NlToSqlOutcome`; client fn `dbNlToSql(connId, body)`.

- [ ] **Step 1: Document the endpoint in the contract**

In `docs/contracts/api.md`, in the DB Explorer engine-access table, add a row:

```
| `POST …/db/nl-to-sql` | draft a read query from NL, validated with EXPLAIN (`{question, node?, max_attempts?}` → `NlToSqlOutcome`); `Editor` |
```

- [ ] **Step 2: Mirror the types in `types.ts`**

Add to `ui/src/lib/api/types.ts` (near the other DB request/response types):

```ts
export interface NlToSqlReq {
  question: string;
  node?: string;
  max_attempts?: number;
}

export interface NlToSqlOutcome {
  sql: string;
  plan: string;
  attempts: number;
  warnings: string[];
}
```

- [ ] **Step 3: Add the client function**

In the DB API client module (the file exporting the other `db*` calls), following the existing call style:

```ts
export function dbNlToSql(connId: string, body: NlToSqlReq): Promise<NlToSqlOutcome> {
  return apiPost(`/connections/${connId}/db/nl-to-sql`, body);
}
```

(Use the module's existing `apiPost`/request helper and import `NlToSqlReq`/`NlToSqlOutcome`.)

- [ ] **Step 4: Type-check**

Run: `cd ui && npm run check`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add docs/contracts/api.md ui/src/lib/api/types.ts ui/src/lib/api/
git commit -m "feat(db): contract + TS client for /db/nl-to-sql"
```

---

### Task 7: UI — the "Ask in English" bar in the query editor

**Files:**
- Modify: `ui/src/modules/database/QueryEditor.svelte`

> **Component spec (follows the existing `QueryEditor.svelte` toolbar/run
> patterns).** This task adds UI; mirror the file's existing button/loading/error
> idioms rather than inventing new ones.

- [ ] **Step 1: Add the NL input affordance**

Add an **"Ask in English"** input + **Generate** button to the Query toolbar
(next to the existing **Ask AI** action), shown only when the engine's
`query_language` is `sql` or `mongo` (hidden for Redis — `EXPLAIN`-validation is
unavailable there, mirroring `explain_validate`'s Redis rejection).

- [ ] **Step 2: Call the endpoint and show the verified result**

On **Generate**: call `dbNlToSql(connId, { question, node: activeDatabaseNode, max_attempts: 3 })`. While in flight, show the existing pulsing/loading indicator. On success, render a small panel with:
- the returned **`sql`** in a read-only CodeMirror block,
- a collapsible **plan** (`outcome.plan`) labelled "Validated with EXPLAIN",
- any `warnings` as a dim note,
- two buttons: **Insert into editor** (puts `sql` into the active tab's statement, does **not** run) and **Run** (inserts then triggers the existing run path).

- [ ] **Step 3: Handle the not-configured / failure cases**

A 400 `Problem` whose message starts with "NL-to-SQL is not configured" → show a one-line "Ask AI is not set up on this server" hint. A loop-exhausted 400 ("could not produce a valid read query…") → show the message verbatim so the user sees the last engine error.

- [ ] **Step 4: Type-check + build**

Run: `cd ui && npm run check && npm run build`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add ui/src/modules/database/QueryEditor.svelte
git commit -m "feat(db): 'Ask in English' verified NL->SQL bar in the query editor"
```

---

### Task 8: Component test for the live validate path + docs

**Files:**
- Create/Modify: `crates/otto-dbviewer/tests/` (a component test using the existing testcontainer harness, if the crate has one) **or** extend an existing MySQL driver test module.
- Modify: `docs/features/database-explorer.md`

- [ ] **Step 1: Component test — EXPLAIN-validate accepts a real SELECT, rejects a bad column**

Using the crate's existing MySQL test harness pattern (testcontainer), assert:
- `explain_validate` returns `Ok(plan)` for `SELECT 1` (or a seeded table),
- returns `Err(msg)` containing the engine's error for `SELECT nope FROM missing`,
- and that a write candidate is rejected by `drive_nl_to_sql` **before** any DB call (already covered by the unit test in Task 2, referenced here).

```bash
# Run (only if the crate's component tests are wired for the harness):
cargo test -p otto-dbviewer --features <component-test-feature> explain_validate
```

(If `otto-dbviewer` has no in-crate testcontainer harness, place this in the workspace's existing DB component-test crate following its conventions; the unit-level guarantees from Tasks 1–2 stand regardless.)

- [ ] **Step 2: Document the feature**

In `docs/features/database-explorer.md` §9 ("Examine with an agent"), add a subsection **"Ask in English (verified NL→SQL)"** describing: input → draft → `statement_is_write` rejection of non-reads → `EXPLAIN` validation → bounded self-correcting retry → "Insert / Run". Note it is **read-only by contract**, `Editor`-gated, unavailable for Redis, and that no draft reaches the editor until it has a valid plan.

- [ ] **Step 3: Full gate**

```bash
cargo build --workspace && cargo test -p otto-dbviewer
cd ui && npm run check
```
Expected: green.

- [ ] **Step 4: Commit**

```bash
git add crates/otto-dbviewer docs/features/database-explorer.md
git commit -m "test(db): validate path coverage + document verified NL->SQL"
```

---

## Self-review

- **Read-only contract:** enforced in `drive_nl_to_sql` via `statement_is_write` *before* validation; unit-tested (`rejects_a_write_without_running_it`). ✓
- **No agent dep in `otto-dbviewer`:** the drafter is a trait; the model call is in `otto-server` (Task 5). ✓
- **Role gate = Editor:** `nl_to_sql` calls `check_conn_role(..Editor)`; tested. ✓
- **Bounded retries:** `max_attempts.clamp(1,4)`; unit-tested (`gives_up_after_max_attempts_with_last_error`). ✓
- **Type consistency:** `SqlDrafter`/`SqlValidator`/`drive_nl_to_sql`/`NlToSqlOutcome` names match across Tasks 1–7; `ServiceValidator` field names (`db`,`conn_id`,`user_id`,`node`) match the handler construction in Task 4. ✓
- **Contract lockstep:** Task 6 updates `api.md` + `types.ts`; Task 8 updates the feature doc. ✓
- **Placeholders:** the only non-literal code is the `otto-server` agent client type in Task 5 (`AgentTextClient`/`agent_text_client()`), explicitly flagged because the real symbol lives in code not read for this plan; the prompt and wiring shape are complete.

## Execution handoff

Plan complete and saved to `docs/db/features/0001-verified-nl-to-sql.md`. Execute with **`superpowers:subagent-driven-development`** (fresh subagent per task + review) or **`superpowers:executing-plans`** (inline, batched checkpoints). Tasks 1–2 are the pure testable core and should land first.
