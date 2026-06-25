# Database Explorer — roadmap & design sketches

Design sketches for the net-new features beyond the two full plans (`0001`
verified NL→SQL, `0002` data import). Each sketch is **actionable but not yet a
full TDD plan**: it gives the feature, why it wins, the API/plug-in surface
(grounded in the real code), a task outline, difficulty, and the moat flag.
Promote a sketch to its own numbered full plan (`superpowers:writing-plans`
format, like `0001`/`0002`) when it's picked up.

Tiers mirror the competitive benchmark (see memory `otto-db-explorer-competitive-roadmap`):
**T1** = grid/editor quick wins, **T2** = per-engine depth, **T3** = big bets on
owned infra, **W** = signature wedges no incumbent can copy.

> **Calibration reminder:** every "already built" item from `README.md` is done.
> These sketches are only the gaps. Re-confirm against the code before starting —
> the explorer moves fast.

---

## T1 — grid & editor quick wins (cheap, high delight)

### 0003a · In-grid foreign-key navigation  ·  Easy  ·  moat: —

**What:** In a single-table result, a cell in an FK column gets a "→ go to
referenced row" affordance that opens a new tab with
`SELECT * FROM <ref_table> WHERE <ref_col> = <value> LIMIT 1`. The DataGrip/DBeaver
staple; it makes the grid feel relational instead of flat.

**Why it's nearly free:** `ObjectDetail.foreign_keys: Vec<ForeignKey>` is already
populated (it drives the ERD — `service.rs:356` `schema_graph`), and the grid
already fetches `object_detail` to decide edit-ability. Reuse both.

**Surface / plug-in:**
- No new endpoint required. The grid, when the result is single-table (the same
  detection that gates inline editing), fetches `object_detail` (already wired)
  and reads `foreign_keys`. For each FK whose `columns` are present, mark those
  result columns "navigable".
- Optional helper `DbViewerService::table_foreign_keys(conn_id, node) -> Vec<ForeignKey>`
  if the grid wants FKs without the full detail payload — but `object_detail`
  already returns them, so v1 needs **no backend change**.

**Task outline:**
1. UI: from the active single-table result, resolve the table's `foreign_keys`
   (reuse the object-detail call already made for edit-ability).
2. UI: render an FK chip / context-menu item "Go to `ref_table`" on FK cells.
3. UI: clicking opens a new Query tab with the generated `SELECT … WHERE … LIMIT 1`
   (uses the existing "open in new tab + run" path).
4. Test: a Svelte/unit test that the generated SQL is correct for composite FKs.

**Difficulty:** Easy (UI-only; reuses existing introspection). **Best first quick win.**

---

### 0003b · Generate SQL from selected rows  ·  Easy  ·  moat: —

**What:** Select rows in the grid → "Copy as `INSERT` / `UPDATE` / `WHERE id IN (…)`".
A constant small-but-loved feature in TablePlus/DataGrip.

**Why cheap:** `import::sql_string_literal` + `build_insert_statements` from `0002`
already produce safe `INSERT`s. Reuse them client-shaped (or a tiny endpoint).

**Surface:** Pure client-side once `0002` lands (the grid has the rows + the
table name from single-table detection). Generate `INSERT` via the same escaping
rules; `WHERE pk IN (…)` from the PK columns already known for edit-ability.

**Task outline:** UI menu action → build statement from selection → new tab /
clipboard. **Difficulty:** Easy (depends on `0002`'s escaping helpers being
shared, or re-implemented in TS).

---

### 0003c · Searchable history + parameterized snippets + local autosave  ·  Easy–Medium  ·  moat: —

**What:** Today history is a recent-list with no search (`service.rs:698`
`list_history`), and there's no per-tab autosave or reusable parameterized
snippet library. Add: a **search box** over history, **named snippets** with
`:param` placeholders (the editor already parses `:name`/`{name}` — `QueryEditor.svelte`),
and **local autosave** of unsaved tab contents.

**Surface:**
- History search: extend `GET …/db/history` with an optional `q` filter (or filter
  client-side over the fetched window for v1). Repo: `DbExplorerRepo::list_history*`.
- Snippets: reuse the **saved-queries** tables/routes (`…/db/saved-queries`) — a
  snippet is a saved query with params; no new schema if you tag it.
- Autosave: client-side (localStorage / app state) keyed by tab; no backend.

**Task outline:** (1) history `q` filter (server or client), (2) snippet = saved
query + param UI, (3) autosave restore on reopen. **Difficulty:** Easy–Medium.

---

### 0003d · Per-connection color tags + command palette (⌘P)  ·  Easy  ·  moat: —

**What:** Color-code connections (beyond the existing PROD/RO rails) and a
**⌘P command palette** to jump to a connection/table/saved query and run actions
— the keyboard-first workflow that keeps CLI/TablePlus users happy
([pgcli context-aware speed](https://www.pgcli.com/completion)).

**Surface:** `Connection` already has `environment`/`read_only`/`pinned`; add an
optional `color` to `Connection.params` (no migration — params is JSON). Palette
is UI-only over the data the page already holds. **Difficulty:** Easy.

---

## T2 — per-engine depth (beat NoSQLBooster / Compass / RedisInsight)

### 0003e · Mongo aggregation pipeline builder with live per-stage preview  ·  Medium  ·  moat: ◑

**What:** The signature NoSQLBooster/Compass feature: build an aggregation as
**ordered stages**, each in its own panel with an **input→output preview**
([Compass](https://www.mongodb.com/docs/compass/create-agg-pipeline/)). Otto's
twist: it **already translates SQL→Mongo** (`drivers/mongo_sql.rs`) — surface the
translated pipeline as **editable stages**, and add "describe the stage in
English → `$stage`" via the `0001` drafter.

**Surface / plug-in:**
- Reuse `Driver::run` with `explain` for stage validation; reuse the Mongo
  driver's existing `aggregate` path.
- New endpoint (thin): `POST …/db/agg-preview` `{ collection, pipeline: [stage], sample: n }`
  → runs the pipeline truncated to `n` docs and returns the output; the service
  builds `db.coll.aggregate(pipeline)` and routes through `run` (read; guard-safe).
- The per-stage preview = run the pipeline **prefix** up to stage *k* with a small
  `$limit`. Bounded cost via the sample size.
- NL→stage reuses `SqlDrafter` from `0001` (engine = Mongo).

**Task outline:** (1) `agg-preview` endpoint (prefix-runner), (2) stage-list UI
mirroring the JOIN-builder canvas idioms (`QueryBuilder.svelte`), (3) SQL→stages
seeding from `mongo_sql`, (4) NL→stage via the drafter. **Difficulty:** Medium.
**Moat ◑** (the NL→stage + SQL-seeded stages are agent-native; the builder itself is parity).

### 0003f · Mongo schema analyzer  ·  Medium  ·  moat: —

**What:** Field presence % + type-distribution bars across sampled docs (Compass's
schema tab). Otto already `$sample`s 100 docs for field completion — extend that to
compute per-field type histograms and render bars in the Structure view.
**Surface:** extend the Mongo driver's `object_detail.extra` (already used for
sampled types) with distributions; UI renders. **Difficulty:** Medium.

### 0003g · Writable Redis + memory-by-prefix + slowlog/profiler  ·  Medium  ·  moat: —

**What:** Redis is read-only in the grid today (`database-explorer.md` §5/§12).
Add type-aware **edits** (`SET`/`HSET`/`LPUSH`/`ZADD`/`EXPIRE`/`DEL`), a
**memory-by-prefix** view (bounded `SCAN` + `MEMORY USAGE`, reusing the existing
bounded-SCAN machinery so it never blocks like RedisInsight does), and a
**slowlog / profiler** panel.

**Surface:**
- Edits route through the **guarded write path** (`statement_is_write` already
  classifies Redis writes; `guard_write` applies) — so a Prod Redis needs
  confirmation automatically. The Redis driver gains an editable-cell → command
  builder (mirror the SQL/Mongo edit-statement builders).
- Memory-by-prefix: new driver call using `SCAN MATCH prefix*` (bounded, exists)
  + pipelined `MEMORY USAGE`; surface as a tree/structure panel.
- Slowlog: `SLOWLOG GET` / pub-sub monitor as a read panel.

**Task outline:** (1) Redis edit→command builder + Review modal reuse, (2)
memory-by-prefix panel, (3) slowlog panel. **Difficulty:** Medium. (Biggest Redis
upside — it's the least-developed engine.)

### 0003h · ClickHouse system-table insights panel  ·  Medium  ·  moat: ◑

**What:** A first-class panel over `system.*` — running/slow queries
(`system.query_log`), parts/merges (`system.parts`, `system.merges`), table sizes.
"No generic GUI nails this for ClickHouse" — a real differentiator for CH-heavy
users. **Surface:** curated read queries run through the existing path, rendered
in a dashboard-like panel (reuse `Chart.svelte`/widget rendering). The agent can
narrate anomalies (moat ◑). **Difficulty:** Medium.

### 0003i · Visual EXPLAIN-plan  ·  Medium  ·  moat: ◑

**What:** Today `EXPLAIN` output is text rows. Render a **visual execution tree**
highlighting full scans vs index use (Compass shows a visual tree; DataGrip 2026.1
enhanced its explain flow). **Surface:** parse the engine's `EXPLAIN` result
(MySQL `EXPLAIN FORMAT=JSON`, ClickHouse `EXPLAIN`, Mongo explain — the latter
already returned) into a tree model; UI renders nodes. Pairs with `0001`
(validated plan is already fetched) and the agent can suggest the index (moat ◑,
see wedge W3). **Difficulty:** Medium.

### 0003j · FK-aware autocomplete  ·  Medium  ·  moat: —

**What:** Completion currently offers keyword/function/identifier lists but is not
FK-aware (memory note). After `FROM orders o JOIN`, suggest the FK-implied join
(`users u ON u.id = o.user_id`). **Surface:** the `completion` path
(`service.rs:669` → driver `completion`) gains FK context from `object_detail`.
**Difficulty:** Medium.

---

## T3 — big bets on owned infra (compose DB with the rest of Otto)

### 0003k · Schema/structure sync → migration script  ·  Medium–Hard  ·  moat: ◑

**What:** Compare two connections/schemas (stg vs prod; `pr_bo_{brandId}`
multi-tenant pain) and generate the **sync DDL**. Navicat's structure-sync is a
top paid reason; Bytebase/Flyway own the migration category
([Bytebase](https://www.bytebase.com/blog/top-database-schema-change-tool-evolution/)).
**Surface:** diff two `schema_graph`/`object_detail` snapshots → DDL generator;
new endpoint `POST …/db/schema-diff` `{ other_conn_id, schema }` → `{ ddl, changes[] }`.
**Difficulty:** Medium–Hard (DDL generation per engine). **Feeds 0003l.**

### 0003l · Agent-authored migrations via the PR flow  ·  Medium–Hard  ·  moat: ✅

**What — the second moat:** "add a `status` column to orders" → the agent drafts
the migration **and rollback**, diffs against the live schema (`0003k`), and opens
a **PR through Otto's existing git integration** (`otto-git`). No GUI tool has an
agent or a git pipeline; no migration tool has a GUI+agent. Otto has all three.

**Surface / plug-in:**
- Reuse the `0001` `SqlDrafter` (engine = SQL, prompt = "emit an idempotent
  migration + rollback for this change against this schema") + the `schema_summary`.
- Validate the migration with `0001`'s loop *adapted for DDL* (dry-run on a
  scratch/transaction where possible; otherwise `EXPLAIN`/parse-check) — note DDL
  can't always be `EXPLAIN`'d, so validation is best-effort + the existing
  **`db-query-reviewer` skill** as the safety review (wedge W4).
- Open the PR via `otto-git` (the crate that already does repos/diffs/commits/PRs).
- The migration file lands in the repo's migrations dir; **append-only** rule
  applies (`crates/otto-state/migrations/` for Otto's own; user repos per their
  convention).

**Task outline:** (1) migration drafter prompt + best-effort validation, (2)
schema-diff grounding (`0003k`), (3) `otto-git` PR open with the migration +
rollback, (4) route + UI "Propose migration". **Difficulty:** Medium–Hard.
**Build after `0001`** (shares the drafter) and ideally after `0003k`.

### 0003m · Cross-connection run-and-diff  ·  Medium  ·  moat: ✅

**What — a true differentiator (not parity):** run one query across **N
connections** (all brand DBs, or staging-vs-prod) and **diff the results**.
"Neither Navicat nor DataGrip does this well." Otto's global connection library +
daemon make it natural.

**Surface:** new endpoint `POST …/db/multi-run` `{ conn_ids: [Id], statement, node? }`
→ runs each through the guarded `run` path (read-gated; writes blocked unless each
is confirmed), returns per-connection `QueryResult` + a computed diff. Concurrency
bounded like `schema_graph` (buffer_unordered). **Difficulty:** Medium. Pairs with
per-connection color tags (`0003d`) for the picker. **Moat ✅** (owned global library).

### 0003n · Scheduled query jobs with Slack/Telegram notify  ·  Medium  ·  moat: ✅

**What:** Schedule a query to run on a cadence and push the result (or an alert on
a threshold) to **Slack/Telegram** — Otto already has channels (`otto-channels`)
+ a daemon + dashboard auto-refresh cadence. Navicat has scheduling; none has it
wired to chat. **Surface:** a scheduler (reuse the dashboard refresh cadence
machinery / a cron) + `otto-channels` send. **Difficulty:** Medium. **Moat ✅**
(channels integration).

### 0003o · FK-correct test-data generator & "Find Usages"  ·  Medium  ·  moat: ◑

**What:** Generate referentially-valid sample data (respecting FKs) and "Find
Usages" of a table/column across saved queries + the connected codebase (code↔DB
fusion, wedge W5). **Surface:** FK graph from `schema_graph` → topological insert
order → the `0002` import path; Find Usages = search saved queries + repo.
**Difficulty:** Medium.

### 0003p · Shareable query + result snapshot  ·  Medium  ·  moat: ✅

**What:** A flat-out gap in **all three** incumbents (DBeaver/DataGrip/TablePlus):
no result-sharing — *"workflows require copying SQL into Slack… emailing files."*
Add a "Share" action that produces a read-only snapshot (the statement + its
result, or a link) the user can drop into a teammate's view or a channel.

**Why Otto:** it already has the **share/remote module** (`ui/src/modules/share`)
and **Slack/Telegram bridges** (`otto-channels`) — sharing a query + snapshot is a
natural composition no DB tool offers. **Surface:** persist a snapshot (statement
+ captured `QueryResult`, owner-private like saved queries) + a share affordance
that posts to a channel or yields a link. Respect masking (`mask` flag) so shared
snapshots don't leak PII. **Difficulty:** Medium. **Moat ✅** (channels + share).

> **Design principle (not a feature) — no silent open transactions.** DBeaver's
> single worst footgun is a *silent* uncommitted transaction holding locks on prod
> (documented real outages). Otto's per-edit Review-modal model already avoids
> this — each edit is an individual, confirmed statement, not a staged open
> transaction. **If batched "pending changes → commit/discard" editing is ever
> added** (the most-loved TablePlus pattern), it MUST ship with a **loud,
> persistent "N uncommitted changes / open transaction" banner** from day one.

---

## W — signature wedges (no incumbent can copy)

These are the *strategy*, realized by the features above. Listed so each feature
is built in service of a wedge, not as a checkbox.

- **W1 · Agentic NL→query loop** — introspect → draft → dry-run/`EXPLAIN` →
  self-correct → **always show SQL before any write**. This is **`0001`** (and
  `0003l` for DDL). Single-shot NL→SQL scores ~21% on realistic agentic benchmarks
  and ~46% of devs distrust AI SQL — the *loop with grounding + visible SQL +
  safety gate* is what actually works, and it's Otto's home turf.
- **W2 · Vault-grounded semantic layer** — RAG over DDL + a business glossary
  using `otto-memory` (the vault). Grounds the drafter (`0001`) in *local*
  knowledge, answering DBeaver's "schema-to-cloud privacy" complaint without
  sending schema anywhere. **Plug-in:** the `SqlDrafter` prompt's `schema_summary`
  becomes "schema_summary + retrieved vault context." **Difficulty:** Medium.
- **W3 · Deterministic index advisor** — the **algorithm** decides the index via
  hypothetical-index simulation (Postgres-MCP-Pro architecture); the LLM only
  *narrates* the recommendation. Deterministic = trustworthy. **Plug-in:** a
  service routine that runs `EXPLAIN` with/without a hypothetical index and reports
  the cost delta; surfaced from the visual EXPLAIN (`0003i`). **Difficulty:** Medium–Hard.
- **W4 · AI migration safety review** — every agent-authored migration (`0003l`)
  passes the existing **`db-query-reviewer` skill** before a PR is opened. Reuses
  an asset we already have.
- **W5 · Code↔DB fusion (the real moat)** — one agent sees the **app code AND the
  live DB**. "Why is this query slow?" → it reads the ORM call site *and* runs
  `EXPLAIN`. "Find usages of this column" → repo search *and* schema. No database
  tool can do this; no IDE has the live DB. Otto has both. Every agent-native
  feature above (W1, 0003l, 0003o) is an instance of it.

---

## Suggested promotion order

1. **`0003a` FK navigation** — easiest delight, reuses existing introspection.
2. **`0003e` Mongo aggregation builder** — biggest NoSQL parity gap; seeds from the existing SQL→Mongo translator.
3. **`0003g` writable Redis** — closes the largest single-engine gap (Redis is read-only).
4. **`0003m` cross-connection run-and-diff** — a genuine differentiator that maps onto the global connection library.
5. **`0003l` agent-authored migrations** — the second moat; build after `0001` (shares the drafter) and `0003k` (schema diff).
6. **W2/W3** — deepen `0001` into the vault-grounded, index-advising loop.

Each becomes a full `superpowers:writing-plans` document (numbered `0004+`) when
scheduled. Re-verify the "already built" baseline before starting any of them.

## Avoid the leaders' mistakes (keep winning)

DBeaver memory bloat / Java sludge; TablePlus nag pop-ups + large/remote-data
flakiness; RedisInsight's blocking memory scan; Navicat/DataGrip price + learning
curve. Otto's Tauri/Rust + bounded-SCAN + virtualized grid already win most of
these — **keep winning on large-result performance and Keychain-not-cloud
secrets.** Pricing wedge: DataGrip ~$99–259/yr, Navicat ~$799/yr (or ~$1,599
perpetual) — Otto is license-free and in-ADE.
