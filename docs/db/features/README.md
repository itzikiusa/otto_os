# Database Explorer — feature design & implementation plans

This folder holds the **design and the implementation plans** for the next wave of
work on Otto's Database Explorer (`otto-dbviewer`). It exists to answer one
question: *what do we add so a developer reaches for Otto's databases section
instead of DBeaver, DataGrip, TablePlus, Navicat, NoSQLBooster, or a CLI?*

> **Read this first.** The single most important fact about this roadmap is that
> the Database Explorer is **already a mature feature**, not a v1. Plans here
> describe only **net-new** work. Everything in §"Already built" is done and must
> not be re-planned or rebuilt. Every claim below was checked against the live
> code (`crates/otto-dbviewer/`, `docs/features/database-explorer.md`) on
> 2026-06-25.

## Index

This folder was produced by several parallel design passes on 2026-06-25, so it
holds **two layers**: a small set of *execution-ready, code-calibrated plans*
(the `000N` series, **canonical — build from these**) and two *broader
product-vision design+plan pairs* that explore the same space at a wider
altitude. See "How these documents relate" below before picking one up.

### Canonical, execution-ready (build from these)

| Doc | What it is |
|---|---|
| `README.md` (this file) | Strategy, calibrated current state, the net-new roadmap, sequencing, and the architecture extension points every plan plugs into. |
| `0001-verified-nl-to-sql.md` | **Full implementation plan** — closed-loop natural-language→SQL that the agent *verifies* with `EXPLAIN` before it ever reaches the editor. The moat feature. |
| `0002-data-import.md` | **Full implementation plan** — file→table import (CSV/NDJSON/JSON), mirroring the existing streaming export path. Closes the most glaring functional gap. |
| `0003-roadmap-design-sketches.md` | Design sketches (API surface + task outline + difficulty) for the remaining net-new features: in-grid FK navigation, generate-SQL-from-rows, searchable history/snippets, color tags + palette, the Mongo aggregation pipeline builder + schema analyzer, writable Redis + profiler, ClickHouse insights, EXPLAIN-plan visualization, FK-aware autocomplete, schema diff, agent-authored migrations, cross-connection run-and-diff, scheduled jobs, and the signature "wedge" features. Promote each to its own full plan when it's picked up. |

### Broader design explorations (wider scope, overlapping — mine for breadth)

| Doc | What it is |
|---|---|
| `2026-06-25-database-workbench-design.md` | Broad "safe agentic workbench" UX design in three layers (trust/speed foundation → engine-native inspectors → Otto-specific moat). Product-positioning altitude; wider than the `000N` plans. |
| `2026-06-25-database-workbench-implementation-plan.md` | Milestone-structured plan (M0 audit → M1 trust/speed → M2 engine-native → M3 dashboards/export/integrations → M4 UX/release) for the workbench design above. |
| `data-transfer-and-mongo-power-tools-design.md` | Focused design for **Otto Data Transfer** (export/import + connection-to-connection copy + reusable transfer profiles + run history) and **Mongo Power Tools** (aggregation builder, visual explain, doc editing, schema analysis, SQL→Mongo, snippets). |
| `data-transfer-and-mongo-power-tools-plan.md` | 12-milestone plan for the design above. |

### How these documents relate

The three efforts agree on the thesis (don't out-DBeaver DBeaver; win on
*safe + agent-native + in-the-repo*) and on the "already built" baseline. They
differ in **altitude and overlap**:

- **Build from the `000N` series.** It is the most tightly calibrated to the live
  code (every claim cites `file:line`), follows the `superpowers:writing-plans`
  task/step/TDD shape, and is correct that the **safety layer is already built**
  (`guard_write` / `statement_is_write` / `confirm_write` / masking / auto-LIMIT)
  — so it does not re-plan it.
- **Known overlaps to resolve before executing in parallel:**
  - *Data import* — `0002-data-import.md` (minimal file→table) **⊂**
    `data-transfer-and-mongo-power-tools-*` (file→table **plus** DB↔DB copy +
    transfer profiles + run history). Treat `0002` as the first, shippable slice;
    the data-transfer docs as the broader vision it grows into.
  - *Mongo aggregation builder + schema analyzer* — sketched in `0003` (0003e/0003f),
    fully specced in `data-transfer-and-mongo-power-tools-*` (Mongo Power Tools,
    M9–M10). Use the data-transfer plan as the deep version of the `0003` sketch.
  - *Trust/speed quick wins* (searchable history, snippets, palette, read-mode) —
    sketched in `0003` (0003c/0003d), fully specced in
    `2026-06-25-database-workbench-implementation-plan.md` (M1).
  - *Verified NL→SQL* — uniquely deep in `0001`; the workbench/data-transfer docs
    cover agent **integration** but not the EXPLAIN-validated draft loop.
- **Recommendation:** keep the `000N` series as the source of truth, and fold the
  two broader pairs into it as each feature is picked up (promote a `0003` sketch
  to a full `000N` plan, importing the relevant milestones from the broader docs)
  rather than executing all three sets independently. Consolidating or archiving
  the broader pair once their unique content is absorbed will keep this folder
  unambiguous.

To execute a plan, use **`superpowers:subagent-driven-development`** (a fresh
subagent per task, review between) or **`superpowers:executing-plans`** (inline,
batched checkpoints). Each plan's tasks use `- [ ]` checkbox steps for tracking.

---

## Strategy: where the leverage actually is

DBeaver is the market leader and its **number-one user complaint is that it is a
bloated Java/Eclipse app** — 4.7 GB of RAM, multi-second navigator clicks, UI
freezes on a 150-line script ([DBeaver #38117](https://github.com/dbeaver/dbeaver/issues/38117),
[#39861](https://github.com/dbeaver/dbeaver/issues/39861)). TablePlus won a large
following by being the opposite: fast, native, and **safe** — its "review pending
changes before you commit" flow is the feature users single out as making them
"feel more secure" ([TablePlus docs](https://docs.tableplus.com/gui-tools/code-review-and-safemode)).
Otto is Rust + native and **already has** the approval-gated edit flow, so the
table-stakes battle is largely won. Adding "more DBeaver features" is not the
lever.

The lever is the one thing **no other database tool structurally has**: Otto runs
a real coding agent as a first-class session that can read the live schema, run
`EXPLAIN`, iterate, edit code, and open PRs. Every competitor bolts a one-shot
chat box onto a static editor — DataGrip's AI Assistant, TablePlus AI,
NoSQLBooster 10's NL query, Outerbase EZQL. Their chronic failure is **trust**:
hallucinated columns and wrong joins ([Bytebase](https://www.bytebase.com/blog/top-text-to-sql-query-tools/),
[AI2SQL](https://builder.ai2sql.io/blog/ai-generated-sql-safety-guide)). Otto can
close that loop — draft → `EXPLAIN`-validate → self-correct → present a query it
has *proven* parses and uses an index. That is plan `0001`, and it is the moat.

So the sequence is: **close the embarrassing functional gaps (import), then build
the agent-native experiences nobody can copy, then deepen NoSQL.** Safety — the
usual prerequisite for trusting AI against a real database — is already done here
(see below), which is exactly why Otto can ship the agent loop ahead of everyone.

### AI trust principles (govern `0001` and every W feature)

The competitive research is blunt that the AI-in-DB trust bar is brutal: *"error
rate must = 0 for database tools," "wrong data is strictly worse than no data,"*
and ~46% of developers distrust AI-generated SQL. Single-shot text-to-SQL scores
~21% on realistic agentic benchmarks. So every AI feature in this folder obeys
these non-negotiables:

1. **Always show the SQL before any write.** Nothing the model produces mutates
   data without the user seeing the exact statement (this is why `0001` is
   read-only and routes any chosen action through the existing Review/confirm
   path).
2. **Ground in the live schema, not the model's guess.** The drafter is given
   real tables/columns/FKs (`schema_summary`), and `0001` *validates with
   `EXPLAIN`* before returning — verified, not vibed.
3. **Human-in-the-loop, never autonomous on prod.** The guard layer
   (`is_write_guarded` + typed confirmation) already enforces this; AI output
   inherits it.
4. **Local-first / privacy.** Prefer BYO-key and local models (Ollama); keep the
   vault-grounded semantic layer (`0003` W2) *local* so schema never leaves the
   machine — this directly answers DBeaver's schema-to-cloud privacy backlash.

---

## Already built — do NOT re-plan these

Verified against the code on 2026-06-25. Citations are `file` or `file:line`.

| Capability | Where | Notes |
|---|---|---|
| **Production / read-only write guard** | `service.rs:316` `guard_write`; `types.rs:791` `statement_is_write`; `Connection::is_write_guarded()` | A `Prod`/`read_only` connection refuses any write/DDL unless `QueryRequest.confirm_write` is set. Classifier is conservative (unknown ⇒ write); per-engine (`sql_is_write`/`redis_is_write`/`mongo_is_write`). `explain:true` cannot bypass it. |
| **Typed-confirmation + audit** | `http.rs:343` `on_confirmed_write`; `database-explorer.md` §13 | Confirmed writes on a guarded connection fire `db.write_confirmed`. UI shows PROD/RO rails + badges. |
| **Server-side PII masking** | `service.rs:533`; `QueryRequest.mask` (`types.rs:459`) | `otto_core::redact` runs in the daemon; raw cells never reach the browser; `masked` badge. |
| **Approval-gated inline editing** ("review before commit") | `database-explorer.md` §5; `ResultsGrid.svelte` | SQL single-table-with-PK and Mongo find-with-`_id` build an `UPDATE`/`updateOne` into a Review modal; nothing runs until you press Run. |
| **Automatic read `LIMIT`** | `types.rs:703` `inject_row_limit` | Conservative; leaves `SHOW`/`DESC`/`UNION`/`FORMAT`/multi-statement untouched. |
| **Engine-native query cancel** | `driver.rs:65` `run_tracked` + `cancel`; `service.rs:647` | MySQL `KILL QUERY`, ClickHouse `KILL QUERY WHERE query_id` (HTTP). Cross-connection-safe. |
| **Visual JOIN builder** | `QueryBuilder.svelte`; `database-explorer.md` §6 | Drag columns to draw joins, live SQL, FK-suggestion chips. |
| **Read-only ERD** | `service.rs:356` `schema_graph`; `DiagramView.svelte` | Tables + FK edges, parallel introspection (concurrency 8), table cap 60 (1..200). |
| **ClickHouse dashboards/widgets** | `service.rs:712`+; `Dashboards.svelte`, `Chart.svelte` | number/table/line/area/bar/pie, auto-refresh, owner-private. |
| **Streaming export to file** | `export.rs` `ExportSink`; `service.rs:584`; `http.rs:538` | Bounded-memory, NDJSON progress, csv/tsv/(±header)/json/ndjson. |
| **SQL→Mongo translation** | `drivers/mongo_sql.rs` | SELECT/WHERE/ORDER/LIMIT/COUNT/GROUP BY/aggregates/INNER+LEFT equi-join → `$lookup`. |
| **Schema-aware autocomplete, multi-tab editor, query history, saved queries** | `QueryEditor.svelte`; `service.rs:669` `completion`; §11 | Per-engine completion; per-user history (migration 0042). |
| **"Examine with an agent" hand-off** | `crates/otto-server/src/modules.rs` `db/explain-with-agent`; §9 | Spawns a *new agent session* seeded with object DDL / result context. This is a session hand-off, **not** an inline draft-and-verify loop — which is the gap `0001` fills. |

If a future request asks to "add safe editing" or "add a write guard," the answer
is: it already exists — point the requester at the rows above.

---

## Net-new roadmap

Each row is genuinely absent today (verified). "Moat" = uniquely enabled by Otto
being agent-native. Difficulty is engineering size, not value.

| # | Feature | Category | Moat | Difficulty | Status |
|---|---|---|:--:|---|---|
| 0001 | **Verified NL→SQL** (draft → `EXPLAIN`-validate → self-correct → editor) | AI / moat | ✅ | Medium | Full plan |
| 0002 | **Data import** (file→table, CSV/NDJSON/JSON; DB→DB later) | Table-stakes | — | Medium | Full plan |
| 0003a | **In-grid FK navigation** (click an FK cell → jump to referenced row) | Table-stakes | — | Easy | Sketch |
| 0003b | **Mongo aggregation pipeline builder** (per-stage preview; NL→stages) | NoSQL depth | ◑ | Medium | Sketch |
| 0003c | **Agent-authored migrations** (NL → migration + rollback → PR via `otto-git`) | AI / moat | ✅ | Medium-Hard | Sketch |
| 0003d | **Redis write ops + profiler** (type-aware edits, `EXPIRE`/`DEL`, slowlog, pub/sub) | NoSQL depth | — | Medium | Sketch |
| 0003e | **EXPLAIN-plan visualization** (visual execution tree, scan/index highlight) | Table-stakes | ◑ | Medium | Sketch |
| 0003f | **Schema diff / compare** (two connections → sync DDL) | Table-stakes | ◑ | Medium | Sketch |

### Sequencing rationale

1. **0002 Data import first.** It's the one *functional* hole (the explorer is
   export-only) and it needs no agent plumbing, so it ships value immediately and
   warms up the import/parse code paths.
2. **0001 Verified NL→SQL second.** The moat. It depends on nothing in 0002 and
   reuses the already-built write classifier (`statement_is_write`) and `EXPLAIN`.
   This is the headline that differentiates Otto.
3. **0003a FK navigation** any time — it's a cheap, high-delight quick win that
   reuses `ObjectDetail.foreign_keys` (already populated by the ERD path).
4. **0003c Agent-authored migrations** after 0001 — it reuses the same
   `SqlDrafter` extension point and composes it with `otto-git`.
5. **0003b/d/e/f** as depth follow-ups, prioritized by which engine your users
   hit hardest.

---

## Architecture: the extension points every plan uses

Grounded in the real code so plans don't reinvent the wiring.

- **Engine logic** implements the `Driver` trait (`crates/otto-dbviewer/src/driver.rs`).
  Trait methods take a fully-resolved `&ResolvedConfig` (SSH tunnel already open,
  secret already fetched). Methods with sensible engine-agnostic fallbacks are
  **default methods** (e.g. `export_to_path` buffers as a last resort; `run_tracked`
  delegates to `run`). **A new capability that not every engine supports is added
  as a default-erroring trait method, then overridden per engine** — this is the
  pattern `0002` (`import_from_path`) follows.
- **Orchestration** is `DbViewerService` (`service.rs`): it `resolve`s a
  connection, opens/caches the SSH tunnel, dispatches to the `Driver`, applies the
  **write guard** (`guard_write`) and **masking**, and records history. New
  server-side operations add a method here.
- **HTTP** is `api_router::<S: DbViewerCtx>()` (`http.rs:162`). Handlers are
  generic over the `DbViewerCtx` trait, take
  `State<S>` + `Extension<AuthUser>` + `Path<Id>` + `Json<Req>`, gate with
  `check_conn_role(&ctx, &user, &conn, WorkspaceRole::{Viewer|Editor})`, and return
  `ApiResult<Response>`. Errors map through `ApiErr` → RFC-7807 `Problem`.
- **The server-side hook pattern.** `DbViewerCtx` carries capabilities the engine
  crate must stay decoupled from, as **default-noop / default-error trait methods
  the server overrides** — e.g. `on_confirmed_write` (audit). `0001` adds
  `draft_sql` here the same way, so the LLM/agent call lives in `otto-server`
  while `otto-dbviewer` stays free of agent deps and stays unit-testable with a
  stub.
- **Streaming + progress.** Long operations stream **NDJSON progress lines** over
  a `tokio::sync::mpsc` channel + `Body::from_stream` (see `export_to_path`,
  `http.rs:538`). `0002` reuses this verbatim.
- **The contract is authoritative.** Any new endpoint updates
  `docs/contracts/api.md` + `ui/src/lib/api/types.ts` **in lockstep**, and the
  feature doc `docs/features/database-explorer.md`. Migrations (if any) are
  **append-only** under `crates/otto-state/migrations/`.

### Testing posture (matches the existing code)

- The engine crate's tests **don't stand up a live `DbViewerService`** (it needs a
  DB pool + secret store). Instead they test **pure functions and trait-level
  logic with stubs** — see `service.rs` `cancel_handle_for` tests and `http.rs`
  `StubRoles`/`TestCtx`. Plans here follow suit: the testable core of each feature
  is a **pure function or a trait-generic function exercised with stub
  drivers/drafters**, so TDD doesn't require network or a database.
- Component/integration coverage for the live path is added the same way the
  existing drivers are covered.

---

## Non-goals (for this wave)

- **Re-implementing the safety layer.** Done (see "Already built").
- **A from-scratch chart engine / BI suite.** ClickHouse dashboards already exist;
  generalizing them is out of scope here.
- **New engines** (Postgres, SQL Server, …). Valuable, but orthogonal to the
  "pull people off other tools" thesis and tracked separately.
- **Weakening any default.** Loopback-only listener, Keychain secrets, write
  guard, and auto-LIMIT stay exactly as they are; every new write path routes
  through `guard_write`.

---

## Research sources

Competitive findings that shaped this roadmap (gathered 2026-06):

- DBeaver pain points: [#38117](https://github.com/dbeaver/dbeaver/issues/38117), [#39861](https://github.com/dbeaver/dbeaver/issues/39861); [Beekeeper "easy alternative"](https://www.beekeeperstudio.io/alternatives/dbeaver)
- TablePlus safe-edit: [docs](https://docs.tableplus.com/gui-tools/code-review-and-safemode), [Capterra reviews](https://www.capterra.com/p/170642/TablePlus/reviews/)
- DataGrip AI Assistant (object-scoped context, explain/optimize): [JetBrains 2025.2](https://blog.jetbrains.com/datagrip/2025/07/29/datagrip-2025-2-database-object-context-in-the-ai-chat-introspection-by-levels-for-postgresql-and-ms-sql-server-and-more/), [2026.1 AI agents](https://blog.jetbrains.com/datagrip/2026/03/26/datagrip-2026-1-redesigned-query-files-data-source-templates-in-your-jetbrains-account-ai-agents-in-the-ai-chat-explain-plan-flow-enhancements-and-more/)
- Navicat data transfer / structure sync: [Navicat Premium](https://www.navicat.com/en/products/navicat-premium)
- NoSQLBooster (SQL-for-Mongo, NL query): [10.0 release](https://nosqlbooster.com/blog/announcing-nosqlbooster-10/); MongoDB Compass aggregation builder + visual explain: [Compass docs](https://www.mongodb.com/docs/compass/create-agg-pipeline/), [explain plan](https://www.mongodb.com/docs/compass/agg-pipeline-builder/view-pipeline-explain-plan/)
- RedisInsight (Profiler, Workbench, RediSearch): [Redis Insight](https://redis.io/insight/)
- AI-SQL trust/safety: ["Don't let AI touch your prod DB"](https://boringsql.com/posts/dont-let-ai-to-prod/), [Text2SQL Safe Mode (AST)](https://www.text2sql.ai/introducing-safe-mode), [Prisma AI guardrails](https://www.prisma.io/blog/orm-6-15-0-ai-safety-guardrails-for-destructive-commands-and-more)
- Schema migration category: [Bytebase evolution](https://www.bytebase.com/blog/top-database-schema-change-tool-evolution/), [Galaxy tools 2025](https://www.getgalaxy.io/learn/data-tools/best-database-schema-migration-version-control-tools-2025)
- CLI holdouts (context-aware completion): [pgcli](https://www.pgcli.com/completion)
