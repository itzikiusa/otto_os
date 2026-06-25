# Database Workbench Competitive UX Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a safer, faster, more engine-native Database Explorer that can replace routine use of DBeaver, TablePlus, Navicat, NoSQLBooster, Redis Insight, ClickHouse consoles, and CLI tools for Otto-centered engineering workflows.

**Architecture:** Extend the existing `otto-dbviewer` driver/service/http stack instead of creating a parallel DB subsystem. Add typed API contracts, append-only SQLite migrations, focused Svelte components under `ui/src/modules/database/`, and e2e coverage using the existing DB specs. Ship as independent feature slices: foundation first, then engine-native workbenches, then Otto-specific agent/workflow integrations.

**Tech Stack:** Rust, Axum, SQLite/sqlx, Tauri sidecar daemon, Svelte 5, TypeScript, Vite, Playwright, existing Otto RBAC, Keychain, SSH tunnel, context packet, workflow, Vault, Jira/Confluence, Slack/Telegram modules.

---

## Execution Rules

- Do not implement this whole plan in one PR.
- Use one feature branch per task or small group of tightly related tasks.
- Preserve user data. Migrations are append-only.
- Update `docs/contracts/api.md` and `ui/src/lib/api/types.ts` together for every API change.
- Keep secrets out of logs, history, saved queries, evidence bundles, packets, and docs.
- Preserve existing loopback-only daemon defaults.
- Run the smallest relevant test after each step, then the broader gates before merging.
- Do not add a new UI dependency unless the task explicitly proves the existing UI cannot support the feature.

## File Map

### Backend

- Modify: `crates/otto-dbviewer/src/types.rs` - shared request/response types.
- Modify: `crates/otto-dbviewer/src/driver.rs` - optional engine extension methods.
- Modify: `crates/otto-dbviewer/src/service.rs` - guardrails, driver dispatch, history, evidence, export jobs.
- Modify: `crates/otto-dbviewer/src/http.rs` - routes and RBAC checks.
- Modify: `crates/otto-dbviewer/src/drivers/mysql.rs` - quick search, explain, health.
- Modify: `crates/otto-dbviewer/src/drivers/redis.rs` - quick search, key inspector, safe mutations.
- Modify: `crates/otto-dbviewer/src/drivers/mongodb.rs` - quick search, explain, pipeline preview.
- Modify: `crates/otto-dbviewer/src/drivers/clickhouse.rs` - quick search, explain, progress/analytics.
- Modify: `crates/otto-state/src/db_explorer.rs` - saved query metadata, history filters, evidence, export jobs.
- Create: task-specific append-only migrations under `crates/otto-state/migrations/`. As of 2026-06-25 the latest migration is `0075_canvas_scene_meta.sql`; if another migration lands first, use the next available number and update the task text.
- Modify: `crates/otto-server/src/context_packet.rs` - evidence bundle packet formatting only if the existing packet payload is insufficient.
- Modify: `crates/otto-server/src/workflow_engine.rs` - only in the workflow integration task.

### Frontend

- Modify: `ui/src/lib/api/types.ts` - mirror new API types.
- Modify: `ui/src/lib/stores/database.svelte.ts` - store orchestration and API calls.
- Modify: `ui/src/modules/database/DatabasePage.svelte` - shell placement and tabs.
- Modify: `ui/src/modules/database/SchemaTree.svelte` - quick-open actions and search affordances.
- Modify: `ui/src/modules/database/QueryEditor.svelte` - production read mode controls, explain, query library actions.
- Modify: `ui/src/modules/database/ResultsGrid.svelte` - evidence bundles, explain links, export jobs.
- Modify: `ui/src/modules/database/Dashboards.svelte` - dashboard editor entry points.
- Modify: `ui/src/modules/database/WidgetCard.svelte` - widget edit/duplicate/open-query actions.
- Create: `ui/src/modules/database/QuickOpen.svelte`.
- Create: `ui/src/modules/database/QueryLibraryPanel.svelte`.
- Create: `ui/src/modules/database/ConnectionHealthPanel.svelte`.
- Create: `ui/src/modules/database/ProductionReadModeBanner.svelte`.
- Create: `ui/src/modules/database/EvidenceBundleDialog.svelte`.
- Create: `ui/src/modules/database/ExplainWorkbench.svelte`.
- Create: `ui/src/modules/database/RedisInspector.svelte`.
- Create: `ui/src/modules/database/MongoPipelineLab.svelte`.
- Create: `ui/src/modules/database/ClickHouseAnalyticsPanel.svelte`.
- Create: `ui/src/modules/database/DashboardEditor.svelte`.
- Create: `ui/src/modules/database/ExportJobsPanel.svelte`.

### Tests

- Modify/add Rust tests in `crates/otto-dbviewer/src/drivers/*`.
- Modify/add Rust tests in `crates/otto-dbviewer/src/service.rs`.
- Modify/add Rust tests in `crates/otto-state/src/db_explorer.rs`.
- Modify: `ui/e2e/db-sweep-mysql.spec.ts`.
- Modify: `ui/e2e/db-sweep-redis.spec.ts`.
- Modify: `ui/e2e/db-sweep-mongodb.spec.ts`.
- Modify: `ui/e2e/db-sweep-clickhouse.spec.ts`.
- Modify: `ui/e2e/db-export.spec.ts`.
- Modify: `ui/e2e/db-mobile.spec.ts`.
- Create: `ui/e2e/db-workbench-ux.spec.ts`.

## Spec Coverage Matrix

| Design requirement | Owning task |
|---|---|
| Contract/doc cleanup and dialect drift audit | Task 0.1 |
| Production Read Mode source of truth, API, defaults, audit/evidence | Task 1.1 |
| Global DB Quick Open across workspace records and live catalog objects | Task 1.2 |
| Saved query metadata, filters, folders, favorites, history filters, delete/clear | Task 1.3 |
| Redacted evidence bundles and agent packet handoff | Task 1.4, Task 3.2 |
| Connection health, tunnel/TLS state, redacted CLI command | Task 1.5 |
| Structured explain/performance warnings | Task 2.1 |
| Redis key inspector and safe mutations | Task 2.2 |
| Mongo pipeline lab | Task 2.3 |
| ClickHouse analytics loop | Task 2.4 |
| Export job history, list/cancel routes, dashboard editor | Task 3.1 |
| API+DB correlation, workflow hooks, Jira/Slack/Vault destinations | Task 3.3 |
| Responsive/mobile/accessibility fit | Task 4.1 |
| Final feature and contract docs | Task 4.2 |

## Milestone 0: Contract and Current UX Audit

### Task 0.1: DB Contract Audit

**Files:**
- Modify: `docs/contracts/api.md`
- Modify: `docs/features/database-explorer.md`
- Modify: `ui/src/lib/api/types.ts` only if a documented shape is missing.

- [ ] **Step 1: Compare documented DB routes against `crates/otto-dbviewer/src/http.rs`.**

Run:

```bash
rg -n "db/(schema|query|export|saved-queries|dashboards|widgets|history)" docs/contracts/api.md docs/features/database-explorer.md crates/otto-dbviewer/src/http.rs ui/src/lib/api/types.ts
```

Expected: all route descriptions are internally consistent.

- [ ] **Step 2: Fix export wording.**

`docs/contracts/api.md` and `docs/features/database-explorer.md` must agree on
whether `POST /connections/{id}/db/export` is capped or uncapped. Prefer the
current backend behavior, then adjust UI copy if needed.

- [ ] **Step 3: Add a "planned workbench extensions" note.**

Add a short section to `docs/features/database-explorer.md` linking to this
design and explaining that future work extends the current DB Explorer rather
than replacing it.

- [ ] **Step 4: Verify docs-only diff.**

Run:

```bash
git diff -- docs/contracts/api.md docs/features/database-explorer.md ui/src/lib/api/types.ts
```

Expected: no code behavior changes.

- [ ] **Step 5: Commit.**

```bash
git add docs/contracts/api.md docs/features/database-explorer.md ui/src/lib/api/types.ts
git commit -m "docs(db): align database explorer contract before workbench work"
```

## Milestone 1: Trust and Speed Foundation

### Task 1.1: Production Read Mode Types and Enforcement

**Files:**
- Modify: `crates/otto-dbviewer/src/types.rs`
- Modify: `crates/otto-dbviewer/src/service.rs`
- Modify: `crates/otto-dbviewer/src/http.rs`
- Modify: `crates/otto-dbviewer/src/drivers/redis.rs`
- Modify: `crates/otto-dbviewer/src/drivers/mongodb.rs`
- Modify: `crates/otto-dbviewer/src/drivers/clickhouse.rs`
- Modify: `docs/contracts/api.md`
- Modify: `ui/src/lib/api/types.ts`
- Modify: `ui/src/lib/stores/database.svelte.ts`
- Create: `ui/src/modules/database/ProductionReadModeBanner.svelte`
- Modify: `ui/src/modules/database/DatabasePage.svelte`
- Modify: `ui/src/modules/database/QueryEditor.svelte`

- [ ] **Step 1: Write service tests for production read mode.**

Add tests proving:

- production/read-only connection defaults masking on.
- risky writes still require confirmation.
- Redis `KEYS *` is rejected on production/read-only connections.
- per-query timeout is forwarded for MongoDB and ClickHouse.
- risky read override, cancellation, and confirmed guarded-write events produce
  audit/evidence records.

Run:

```bash
cargo test -p otto-dbviewer production_read_mode -- --nocapture
```

Expected: FAIL because the enforcement does not exist yet.

- [ ] **Step 2: Add shared API types.**

In `crates/otto-dbviewer/src/types.rs`, add:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DbWorkbenchMode {
    pub production_read_mode: bool,
    pub mask_default: bool,
    pub default_max_rows: Option<usize>,
    pub default_timeout_ms: Option<u64>,
    pub reasons: Vec<String>,
}
```

Mirror this in `ui/src/lib/api/types.ts`.

- [ ] **Step 3: Add authoritative route and contract.**

Add `GET /connections/{id}/db/workbench-mode` in `http.rs`. It requires viewer
role and returns `DbWorkbenchMode`. Document it in `docs/contracts/api.md`.

- [ ] **Step 4: Implement mode derivation in service.**

In `service.rs`, derive mode from `Connection.environment`, read-only flags, and
connection params. Keep this helper private until another crate needs it. The UI
must render this backend result, not recompute the authority client-side.

- [ ] **Step 5: Enforce risky Redis protections.**

In `drivers/redis.rs` or service-level classification, reject broad `KEYS`
patterns when production read mode is active. Return an actionable error telling
the user to use the safe key inspector or `SCAN`.

- [ ] **Step 6: Add audit/evidence events.**

Record audit/evidence facts for risky read override, cancellation, and confirmed
guarded write paths. Reuse existing audit hooks where possible; do not store raw
unmasked result data.

- [ ] **Step 7: Add UI banner.**

`ProductionReadModeBanner.svelte` renders environment, read-only state, masking,
row limit, timeout, and a short "why this is guarded" message. Place it below the
connection status row in `DatabasePage.svelte`.

- [ ] **Step 8: Thread defaults into the query editor.**

When active mode changes, `QueryEditor.svelte` should show the effective row
limit, timeout, and mask defaults without silently changing an already-customized
tab. New tabs inherit safe defaults.

- [ ] **Step 9: Run tests.**

```bash
cargo test -p otto-dbviewer production_read_mode -- --nocapture
cd ui && npm run check
```

Expected: PASS.

- [ ] **Step 10: Commit.**

```bash
git add crates/otto-dbviewer/src/types.rs crates/otto-dbviewer/src/service.rs crates/otto-dbviewer/src/http.rs crates/otto-dbviewer/src/drivers/redis.rs crates/otto-dbviewer/src/drivers/mongodb.rs crates/otto-dbviewer/src/drivers/clickhouse.rs docs/contracts/api.md ui/src/lib/api/types.ts ui/src/lib/stores/database.svelte.ts ui/src/modules/database/ProductionReadModeBanner.svelte ui/src/modules/database/DatabasePage.svelte ui/src/modules/database/QueryEditor.svelte
git commit -m "feat(db): add production read mode guardrails"
```

### Task 1.2: Global DB Quick Open

**Files:**
- Modify: `crates/otto-dbviewer/src/types.rs`
- Modify: `crates/otto-dbviewer/src/driver.rs`
- Modify: `crates/otto-dbviewer/src/service.rs`
- Modify: `crates/otto-dbviewer/src/http.rs`
- Modify: `crates/otto-dbviewer/src/drivers/mysql.rs`
- Modify: `crates/otto-dbviewer/src/drivers/redis.rs`
- Modify: `crates/otto-dbviewer/src/drivers/mongodb.rs`
- Modify: `crates/otto-dbviewer/src/drivers/clickhouse.rs`
- Modify: `crates/otto-state/src/db_explorer.rs`
- Modify: `docs/contracts/api.md`
- Modify: `ui/src/lib/api/types.ts`
- Modify: `ui/src/lib/stores/database.svelte.ts`
- Create: `ui/src/modules/database/QuickOpen.svelte`
- Modify: `ui/src/modules/database/DatabasePage.svelte`
- Modify: `ui/e2e/db-workbench-ux.spec.ts`

- [ ] **Step 1: Write failing driver/service tests.**

Add tests for:

- workspace quick open returns connections, sections, saved queries, history,
  dashboards, and widgets without touching live DBs.
- MySQL catalog search returns databases/tables/columns capped by limit.
- Redis catalog search requires or benefits from prefix and uses bounded scanning.
- Mongo catalog search returns database/collection/field hits.
- ClickHouse catalog search returns database/table/column hits.

Run:

```bash
cargo test -p otto-dbviewer quick_open -- --nocapture
```

Expected: FAIL.

- [ ] **Step 2: Add API types.**

Add Rust and TypeScript shapes:

```rust
pub struct QuickOpenReq {
    pub query: String,
    pub connection_ids: Vec<String>,
    pub include_workspace: bool,
    pub include_catalog: bool,
    pub limit: Option<usize>,
}

pub struct CatalogSearchReq {
    pub query: String,
    pub node: Option<String>,
    pub kinds: Vec<String>,
    pub limit: Option<usize>,
}

pub struct QuickOpenHit {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub path: Vec<String>,
    pub detail: Option<String>,
    pub action: String,
}
```

- [ ] **Step 3: Extend `Driver`.**

Add `catalog_search` with a default unsupported or empty implementation. Keep
caps server-side even if the UI sends a high limit. Driver catalog search returns
only live DB objects; it must not know about saved queries/history/dashboards.

- [ ] **Step 4: Add routes.**

Add:

- `POST /workspaces/{wid}/db/quick-open` - viewer role, merges workspace records
  and optional catalog hits.
- `POST /connections/{id}/db/catalog-search` - viewer role, dispatches through
  service/driver for live catalog objects only.

- [ ] **Step 5: Implement UI quick open.**

`QuickOpen.svelte` supports keyboard open/close, debounced search, arrow-key
navigation, enter to run primary action, and secondary actions visible as icon
buttons with tooltips.

- [ ] **Step 6: Wire actions.**

Actions call existing store methods where possible:

- open connection.
- open structure.
- set active DB/keyspace.
- open saved query/history in tab.
- open dashboard/widget.
- insert starter query.
- run safe preview.
- send object or saved query context to agent.

- [ ] **Step 7: Add e2e.**

In `ui/e2e/db-workbench-ux.spec.ts`, test that quick open can find:

- a connection.
- a saved query.
- a history row.
- a dashboard/widget when seeded.
- a known seeded table/collection/key without manual tree expansion.

Also test at least one send-to-agent preview action.

- [ ] **Step 8: Run gates.**

```bash
cargo test -p otto-dbviewer quick_open -- --nocapture
cd ui && npm run check
cd ui && npm run test:e2e -- db-workbench-ux
```

Expected: PASS or e2e skip only when the isolated DB seed is unavailable.

- [ ] **Step 9: Commit.**

```bash
git add crates/otto-dbviewer/src/types.rs crates/otto-dbviewer/src/driver.rs crates/otto-dbviewer/src/service.rs crates/otto-dbviewer/src/http.rs crates/otto-dbviewer/src/drivers/mysql.rs crates/otto-dbviewer/src/drivers/redis.rs crates/otto-dbviewer/src/drivers/mongodb.rs crates/otto-dbviewer/src/drivers/clickhouse.rs crates/otto-state/src/db_explorer.rs docs/contracts/api.md ui/src/lib/api/types.ts ui/src/lib/stores/database.svelte.ts ui/src/modules/database/QuickOpen.svelte ui/src/modules/database/DatabasePage.svelte ui/e2e/db-workbench-ux.spec.ts
git commit -m "feat(db): add global quick open for database objects"
```

### Task 1.3: Query Library and History Manager

**Files:**
- Create: `crates/otto-state/migrations/0076_db_query_library.sql` or the next available migration number.
- Modify: `crates/otto-state/src/db_explorer.rs`
- Modify: `crates/otto-dbviewer/src/http.rs`
- Modify: `crates/otto-dbviewer/src/service.rs`
- Modify: `docs/contracts/api.md`
- Modify: `ui/src/lib/api/types.ts`
- Modify: `ui/src/lib/stores/database.svelte.ts`
- Create: `ui/src/modules/database/QueryLibraryPanel.svelte`
- Modify: `ui/src/modules/database/DatabasePage.svelte`
- Modify: `ui/e2e/db-workbench-ux.spec.ts`

- [ ] **Step 1: Write state repository tests.**

Add tests proving saved queries can be listed by search/tag/folder/favorite and
history can be searched/promoted. Include history filters for success/failure,
connection, date range, duration range, row-count range, delete one owned row,
and clear own history.

Run:

```bash
cargo test -p otto-state db_query_library -- --nocapture
```

Expected: FAIL.

- [ ] **Step 2: Add append-only migration.**

Create `0076_db_query_library.sql` with compatible additions:

```sql
ALTER TABLE db_saved_queries ADD COLUMN description TEXT;
ALTER TABLE db_saved_queries ADD COLUMN tags_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE db_saved_queries ADD COLUMN folder TEXT;
ALTER TABLE db_saved_queries ADD COLUMN favorite INTEGER NOT NULL DEFAULT 0;
ALTER TABLE db_saved_queries ADD COLUMN updated_at TEXT;
CREATE INDEX IF NOT EXISTS idx_db_saved_ws_folder ON db_saved_queries(workspace_id, folder);
CREATE INDEX IF NOT EXISTS idx_db_saved_ws_favorite ON db_saved_queries(workspace_id, favorite);
```

If SQLite/default compatibility in the migration runner requires a different
shape, use side tables rather than editing old migrations.

- [ ] **Step 3: Extend repo methods.**

Add create/update/list filters for saved queries and list filters for history.
Preserve owner-private behavior. History filters must include `connection_id`,
`search`, `ok`, `date_from`, `date_to`, `min_duration_ms`,
`max_duration_ms`, `min_rows`, `max_rows`, `limit`, and `cursor`.

- [ ] **Step 4: Add PATCH and filtered GET routes.**

Add `PATCH /db/saved-queries/{qid}` and query parameters on saved/history list
routes. Add:

- `GET /workspaces/{wid}/db/history?...`
- `DELETE /db/history/{hid}`
- `DELETE /connections/{id}/db/history`
- `DELETE /workspaces/{wid}/db/history`

Update contracts and TS types.

- [ ] **Step 5: Build QueryLibraryPanel.**

Replace the current thin Saved/History sidebar content with:

- search field.
- saved/history segmented control or side tabs.
- favorite, folder, tag filters.
- connection, status, date, duration, and row-count filters for history.
- row actions: open, run, duplicate, promote, rename/edit, delete.
- clear own history with confirmation.

- [ ] **Step 6: Add e2e.**

Test save -> favorite -> search -> open -> promote history -> filter history by
status/connection/date -> delete one history row -> clear own history.

- [ ] **Step 7: Run gates.**

```bash
cargo test -p otto-state db_query_library -- --nocapture
cargo test -p otto-dbviewer saved -- --nocapture
cd ui && npm run check
cd ui && npm run test:e2e -- db-workbench-ux
```

Expected: PASS.

- [ ] **Step 8: Commit.**

```bash
git add crates/otto-state/migrations/0076_db_query_library.sql crates/otto-state/src/db_explorer.rs crates/otto-dbviewer/src/http.rs crates/otto-dbviewer/src/service.rs docs/contracts/api.md ui/src/lib/api/types.ts ui/src/lib/stores/database.svelte.ts ui/src/modules/database/QueryLibraryPanel.svelte ui/src/modules/database/DatabasePage.svelte ui/e2e/db-workbench-ux.spec.ts
git commit -m "feat(db): add searchable query library and history manager"
```

### Task 1.4: Evidence Bundles

**Files:**
- Create: `crates/otto-state/migrations/0077_db_evidence_bundles.sql` or the next available migration number.
- Modify: `crates/otto-state/src/db_explorer.rs`
- Modify: `crates/otto-dbviewer/src/types.rs`
- Modify: `crates/otto-dbviewer/src/service.rs`
- Modify: `crates/otto-dbviewer/src/http.rs`
- Modify: `crates/otto-server/src/context_packet.rs` only if needed.
- Modify: `docs/contracts/api.md`
- Modify: `ui/src/lib/api/types.ts`
- Modify: `ui/src/lib/stores/database.svelte.ts`
- Create: `ui/src/modules/database/EvidenceBundleDialog.svelte`
- Modify: `ui/src/modules/database/ResultsGrid.svelte`
- Modify: `ui/src/modules/database/QueryEditor.svelte`
- Modify: `ui/e2e/db-workbench-ux.spec.ts`

- [ ] **Step 1: Write redaction tests.**

Add tests proving an evidence bundle records masking/truncation state and never
stores raw secret-bearing profile fields.

Run:

```bash
cargo test -p otto-dbviewer evidence_bundle -- --nocapture
```

Expected: FAIL.

- [ ] **Step 2: Add persistence.**

Create an append-only migration for `db_evidence_bundles`:

```sql
CREATE TABLE IF NOT EXISTS db_evidence_bundles (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    connection_id TEXT NOT NULL,
    created_by TEXT NOT NULL,
    title TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_db_evidence_ws ON db_evidence_bundles(workspace_id, created_at DESC);
```

- [ ] **Step 3: Add service builder.**

Given a query/result/explain context, produce a bundle with statement, engine,
environment, timing, row count, truncation, masking, and redacted sample rows.

- [ ] **Step 4: Add API route.**

`POST /connections/{id}/db/evidence-bundles` requires editor role because it can
persist and share operational context.

- [ ] **Step 5: Build dialog.**

`EvidenceBundleDialog.svelte` previews Markdown, redaction/masking status, and
actions: copy Markdown, save, send to agent, draft Jira/Slack when those modules
are available.

- [ ] **Step 6: Wire ResultsGrid and QueryEditor.**

Add an icon button near existing copy/export/agent actions. Do not crowd the
toolbar; use wrapping/menus where needed.

- [ ] **Step 7: Add e2e.**

Test query result -> evidence preview -> copy markdown -> send-to-agent preview.

- [ ] **Step 8: Run gates.**

```bash
cargo test -p otto-dbviewer evidence_bundle -- --nocapture
cargo test -p otto-state evidence -- --nocapture
cd ui && npm run check
cd ui && npm run test:e2e -- db-workbench-ux
```

Expected: PASS.

- [ ] **Step 9: Commit.**

```bash
git add crates/otto-state/migrations/0077_db_evidence_bundles.sql crates/otto-state/src/db_explorer.rs crates/otto-dbviewer/src/types.rs crates/otto-dbviewer/src/service.rs crates/otto-dbviewer/src/http.rs crates/otto-server/src/context_packet.rs docs/contracts/api.md ui/src/lib/api/types.ts ui/src/lib/stores/database.svelte.ts ui/src/modules/database/EvidenceBundleDialog.svelte ui/src/modules/database/ResultsGrid.svelte ui/src/modules/database/QueryEditor.svelte ui/e2e/db-workbench-ux.spec.ts
git commit -m "feat(db): create redacted evidence bundles from query results"
```

### Task 1.5: Connection Health Panel

**Files:**
- Modify: `crates/otto-dbviewer/src/types.rs`
- Modify: `crates/otto-dbviewer/src/driver.rs`
- Modify: `crates/otto-dbviewer/src/service.rs`
- Modify: `crates/otto-dbviewer/src/http.rs`
- Modify: `crates/otto-dbviewer/src/config.rs`
- Modify: `docs/contracts/api.md`
- Modify: `ui/src/lib/api/types.ts`
- Modify: `ui/src/lib/stores/database.svelte.ts`
- Create: `ui/src/modules/database/ConnectionHealthPanel.svelte`
- Modify: `ui/src/modules/database/DatabasePage.svelte`

- [ ] **Step 1: Write tests for redacted health output.**

Health must include version/latency/tunnel/TLS metadata but not passwords,
private key contents, or unredacted connection strings.

Run:

```bash
cargo test -p otto-dbviewer db_health -- --nocapture
```

Expected: FAIL.

- [ ] **Step 2: Add `DbHealth` type and driver method.**

Include:

- `ok`
- `latency_ms`
- `server_version`
- `engine`
- `environment`
- `read_only`
- `tls`
- `ssh_tunnel`
- `last_error`
- `redacted_cli_command`

- [ ] **Step 3: Add route.**

`GET /connections/{id}/db/health` requires viewer role.

- [ ] **Step 4: Build panel.**

Add a compact drawer/panel from the active connection row. Include retest,
copy redacted CLI command, tunnel status, and TLS/SNI facts.

- [ ] **Step 5: Run gates.**

```bash
cargo test -p otto-dbviewer db_health -- --nocapture
cd ui && npm run check
```

Expected: PASS.

- [ ] **Step 6: Commit.**

```bash
git add crates/otto-dbviewer/src/types.rs crates/otto-dbviewer/src/driver.rs crates/otto-dbviewer/src/service.rs crates/otto-dbviewer/src/http.rs crates/otto-dbviewer/src/config.rs docs/contracts/api.md ui/src/lib/api/types.ts ui/src/lib/stores/database.svelte.ts ui/src/modules/database/ConnectionHealthPanel.svelte ui/src/modules/database/DatabasePage.svelte
git commit -m "feat(db): add connection health and tunnel details panel"
```

## Milestone 2: Engine-Native Workbenches

### Task 2.1: Explain Workbench

**Files:**
- Modify: `crates/otto-dbviewer/src/types.rs`
- Modify: `crates/otto-dbviewer/src/driver.rs`
- Modify: `crates/otto-dbviewer/src/service.rs`
- Modify: `crates/otto-dbviewer/src/http.rs`
- Modify: `crates/otto-dbviewer/src/drivers/mysql.rs`
- Modify: `crates/otto-dbviewer/src/drivers/mongodb.rs`
- Modify: `crates/otto-dbviewer/src/drivers/clickhouse.rs`
- Modify: `docs/contracts/api.md`
- Modify: `ui/src/lib/api/types.ts`
- Modify: `ui/src/lib/stores/database.svelte.ts`
- Create: `ui/src/modules/database/ExplainWorkbench.svelte`
- Modify: `ui/src/modules/database/QueryEditor.svelte`
- Modify: `ui/src/modules/database/ResultsGrid.svelte`

- [ ] **Step 1: Write parser/warning tests.**

Cover at least:

- MySQL full table scan warning.
- Mongo collection scan warning.
- ClickHouse bytes/rows scanned warning.

Run:

```bash
cargo test -p otto-dbviewer explain_workbench -- --nocapture
```

Expected: FAIL.

- [ ] **Step 2: Add explain types and driver method.**

Use a normalized `ExplainResult` with raw payload preserved:

- `engine`
- `format`
- `summary`
- `warnings`
- `nodes`
- `raw`

- [ ] **Step 3: Add `POST /connections/{id}/db/explain`.**

The route should use the same role as query execution or viewer if it never runs
the statement. For `EXPLAIN ANALYZE`, require editor and guard it like a query.

- [ ] **Step 4: Implement first-pass engine explain.**

Start conservative. Parse only high-confidence fields. Preserve raw output for
agent review.

- [ ] **Step 5: Build workbench UI.**

Show summary, warnings, raw details, plan table/tree, and "Send to agent."

- [ ] **Step 6: Run gates.**

```bash
cargo test -p otto-dbviewer explain_workbench -- --nocapture
cd ui && npm run check
```

Expected: PASS.

- [ ] **Step 7: Commit.**

```bash
git add crates/otto-dbviewer/src/types.rs crates/otto-dbviewer/src/driver.rs crates/otto-dbviewer/src/service.rs crates/otto-dbviewer/src/http.rs crates/otto-dbviewer/src/drivers/mysql.rs crates/otto-dbviewer/src/drivers/mongodb.rs crates/otto-dbviewer/src/drivers/clickhouse.rs docs/contracts/api.md ui/src/lib/api/types.ts ui/src/lib/stores/database.svelte.ts ui/src/modules/database/ExplainWorkbench.svelte ui/src/modules/database/QueryEditor.svelte ui/src/modules/database/ResultsGrid.svelte
git commit -m "feat(db): add structured explain workbench"
```

### Task 2.2: Redis Safe Key Inspector

**Files:**
- Modify: `crates/otto-dbviewer/src/types.rs`
- Modify: `crates/otto-dbviewer/src/driver.rs`
- Modify: `crates/otto-dbviewer/src/service.rs`
- Modify: `crates/otto-dbviewer/src/http.rs`
- Modify: `crates/otto-dbviewer/src/drivers/redis.rs`
- Modify: `docs/contracts/api.md`
- Modify: `ui/src/lib/api/types.ts`
- Modify: `ui/src/lib/stores/database.svelte.ts`
- Create: `ui/src/modules/database/RedisInspector.svelte`
- Modify: `ui/src/modules/database/StructureView.svelte`
- Modify: `ui/e2e/db-sweep-redis.spec.ts`

- [ ] **Step 1: Write Redis driver tests.**

Cover string/hash/list/set/zset/stream detail mapping, TTL, memory usage fallback,
and safe mutation classification.

Run:

```bash
cargo test -p otto-dbviewer redis_inspector -- --nocapture
```

Expected: FAIL.

- [ ] **Step 2: Add typed Redis key routes.**

Add inspect and mutate routes. Mutations require editor role and existing guarded
write confirmation when production/read-only.

- [ ] **Step 3: Implement bounded inspection.**

Every collection-like key view must page or cap results. Never fetch an entire
large list/set/hash/stream by default.

- [ ] **Step 4: Build RedisInspector.**

Render type-specific views and actions: TTL, persist, expire, delete, edit field
where safe.

- [ ] **Step 5: Add e2e.**

Use seeded Redis data. Test prefix search, open key, view TTL/type, and a safe
read-only operation. Destructive mutation tests must use isolated seed data only.

- [ ] **Step 6: Run gates.**

```bash
cargo test -p otto-dbviewer redis_inspector -- --nocapture
cd ui && npm run check
cd ui && npm run test:e2e -- db-sweep-redis
```

Expected: PASS.

- [ ] **Step 7: Commit.**

```bash
git add crates/otto-dbviewer/src/types.rs crates/otto-dbviewer/src/driver.rs crates/otto-dbviewer/src/service.rs crates/otto-dbviewer/src/http.rs crates/otto-dbviewer/src/drivers/redis.rs docs/contracts/api.md ui/src/lib/api/types.ts ui/src/lib/stores/database.svelte.ts ui/src/modules/database/RedisInspector.svelte ui/src/modules/database/StructureView.svelte ui/e2e/db-sweep-redis.spec.ts
git commit -m "feat(db): add safe Redis key inspector"
```

### Task 2.3: Mongo Pipeline Lab

**Files:**
- Modify: `crates/otto-dbviewer/src/types.rs`
- Modify: `crates/otto-dbviewer/src/driver.rs`
- Modify: `crates/otto-dbviewer/src/service.rs`
- Modify: `crates/otto-dbviewer/src/http.rs`
- Modify: `crates/otto-dbviewer/src/drivers/mongodb.rs`
- Modify: `docs/contracts/api.md`
- Modify: `ui/src/lib/api/types.ts`
- Modify: `ui/src/lib/stores/database.svelte.ts`
- Create: `ui/src/modules/database/MongoPipelineLab.svelte`
- Modify: `ui/src/modules/database/DatabasePage.svelte`
- Modify: `ui/e2e/db-sweep-mongodb.spec.ts`

- [ ] **Step 1: Write Mongo pipeline tests.**

Test JSON validation, stage preview, `maxTimeMS`, limit, and explain request
construction.

Run:

```bash
cargo test -p otto-dbviewer mongo_pipeline_lab -- --nocapture
```

Expected: FAIL.

- [ ] **Step 2: Add preview route and types.**

`POST /connections/{id}/db/mongo/pipeline-preview` requires editor role because
it executes a read.

- [ ] **Step 3: Implement stage-by-stage preview.**

Run the pipeline incrementally with a cap. Preserve raw errors with stage index.

- [ ] **Step 4: Build UI.**

Add stage list, JSON editor, preview panel, explain panel, save pipeline action,
and send-to-agent action.

- [ ] **Step 5: Add e2e.**

Test a seeded collection pipeline with two stages and preview results.

- [ ] **Step 6: Run gates.**

```bash
cargo test -p otto-dbviewer mongo_pipeline_lab -- --nocapture
cd ui && npm run check
cd ui && npm run test:e2e -- db-sweep-mongodb
```

Expected: PASS.

- [ ] **Step 7: Commit.**

```bash
git add crates/otto-dbviewer/src/types.rs crates/otto-dbviewer/src/driver.rs crates/otto-dbviewer/src/service.rs crates/otto-dbviewer/src/http.rs crates/otto-dbviewer/src/drivers/mongodb.rs docs/contracts/api.md ui/src/lib/api/types.ts ui/src/lib/stores/database.svelte.ts ui/src/modules/database/MongoPipelineLab.svelte ui/src/modules/database/DatabasePage.svelte ui/e2e/db-sweep-mongodb.spec.ts
git commit -m "feat(db): add Mongo pipeline lab"
```

### Task 2.4: ClickHouse Analytics Loop

**Files:**
- Modify: `crates/otto-dbviewer/src/types.rs`
- Modify: `crates/otto-dbviewer/src/drivers/clickhouse.rs`
- Modify: `crates/otto-dbviewer/src/service.rs`
- Modify: `crates/otto-dbviewer/src/http.rs`
- Modify: `docs/contracts/api.md`
- Modify: `ui/src/lib/api/types.ts`
- Modify: `ui/src/lib/stores/database.svelte.ts`
- Create: `ui/src/modules/database/ClickHouseAnalyticsPanel.svelte`
- Modify: `ui/src/modules/database/QueryEditor.svelte`
- Modify: `ui/src/modules/database/ResultsGrid.svelte`
- Modify: `ui/e2e/db-sweep-clickhouse.spec.ts`

- [ ] **Step 1: Write ClickHouse stats tests.**

Test parsing and propagation of rows read, bytes read, elapsed time, and explain
index data where available.

Run:

```bash
cargo test -p otto-dbviewer clickhouse_analytics -- --nocapture
```

Expected: FAIL.

- [ ] **Step 2: Extend query stats.**

Add optional stats without breaking existing clients:

- `rows_read`
- `bytes_read`
- `memory_usage`
- `query_id`

- [ ] **Step 3: Implement driver propagation.**

Preserve current query behavior. Add stats only when the transport exposes them.

- [ ] **Step 4: Build analytics panel.**

Show scanned rows/bytes, duration, query id, explain link, chart action, and
compare-to-previous action.

- [ ] **Step 5: Add e2e.**

Test that ClickHouse results display analytics metadata when available and do not
break when unavailable.

- [ ] **Step 6: Run gates.**

```bash
cargo test -p otto-dbviewer clickhouse_analytics -- --nocapture
cd ui && npm run check
cd ui && npm run test:e2e -- db-sweep-clickhouse
```

Expected: PASS.

- [ ] **Step 7: Commit.**

```bash
git add crates/otto-dbviewer/src/types.rs crates/otto-dbviewer/src/drivers/clickhouse.rs crates/otto-dbviewer/src/service.rs crates/otto-dbviewer/src/http.rs docs/contracts/api.md ui/src/lib/api/types.ts ui/src/lib/stores/database.svelte.ts ui/src/modules/database/ClickHouseAnalyticsPanel.svelte ui/src/modules/database/QueryEditor.svelte ui/src/modules/database/ResultsGrid.svelte ui/e2e/db-sweep-clickhouse.spec.ts
git commit -m "feat(db): surface ClickHouse analytics loop"
```

## Milestone 3: Dashboard, Export, and Otto Integrations

### Task 3.1: Dashboard Editor and Export Job History

**Files:**
- Create: `crates/otto-state/migrations/0078_db_export_jobs.sql` or the next available migration number.
- Modify: `crates/otto-state/src/db_explorer.rs`
- Modify: `crates/otto-dbviewer/src/types.rs`
- Modify: `crates/otto-dbviewer/src/service.rs`
- Modify: `crates/otto-dbviewer/src/http.rs`
- Modify: `docs/contracts/api.md`
- Modify: `ui/src/lib/api/types.ts`
- Modify: `ui/src/lib/stores/database.svelte.ts`
- Create: `ui/src/modules/database/DashboardEditor.svelte`
- Create: `ui/src/modules/database/ExportJobsPanel.svelte`
- Modify: `ui/src/modules/database/Dashboards.svelte`
- Modify: `ui/src/modules/database/WidgetCard.svelte`
- Modify: `ui/src/modules/database/ResultsGrid.svelte`
- Modify: `ui/e2e/db-export.spec.ts`

- [ ] **Step 1: Write persistence tests for export jobs.**

Export job rows must record owner, connection, local path, format, state,
rows/bytes, errors, and timestamps.

Run:

```bash
cargo test -p otto-state db_export_jobs -- --nocapture
```

Expected: FAIL.

- [ ] **Step 2: Create migration with `db_export_jobs`.**

Use append-only SQL. Do not alter old migration files.

- [ ] **Step 3: Add list/cancel routes and contract.**

Add and document:

- `GET /connections/{id}/db/export-jobs`
- `POST /connections/{id}/db/export-jobs/{job_id}/cancel`

Cancellation should stop the tracked job when the driver/export path supports
it, or mark the request as cancelled with clear "best effort" messaging when
only the client stream can be aborted.

- [ ] **Step 4: Persist export job progress.**

Update `export_to_path` flow to create/update job records while preserving the
current NDJSON progress stream.

- [ ] **Step 5: Build export jobs panel.**

Show running/recent jobs, progress, final local path, errors, and rerun/reveal
actions where supported. Include a Cancel button for running jobs and make the
resulting state explicit: completed, failed, cancelled, or cancel requested.

- [ ] **Step 6: Build dashboard editor.**

Expose existing layout and widget patch endpoints with edit/duplicate/delete,
mapping, title, refresh, and open-query actions.

- [ ] **Step 7: Run gates.**

```bash
cargo test -p otto-state db_export_jobs -- --nocapture
cargo test -p otto-dbviewer export -- --nocapture
cd ui && npm run check
cd ui && npm run test:e2e -- db-export
```

Expected: PASS.

- [ ] **Step 8: Commit.**

```bash
git add crates/otto-state/migrations/0078_db_export_jobs.sql crates/otto-state/src/db_explorer.rs crates/otto-dbviewer/src/types.rs crates/otto-dbviewer/src/service.rs crates/otto-dbviewer/src/http.rs docs/contracts/api.md ui/src/lib/api/types.ts ui/src/lib/stores/database.svelte.ts ui/src/modules/database/DashboardEditor.svelte ui/src/modules/database/ExportJobsPanel.svelte ui/src/modules/database/Dashboards.svelte ui/src/modules/database/WidgetCard.svelte ui/src/modules/database/ResultsGrid.svelte ui/e2e/db-export.spec.ts
git commit -m "feat(db): add dashboard editor and export job history"
```

### Task 3.2: Agentic DB Investigation Workspace

**Files:**
- Modify: `crates/otto-server/src/context_packet.rs`
- Modify: `crates/otto-dbviewer/src/types.rs`
- Modify: `crates/otto-dbviewer/src/service.rs`
- Modify: `docs/contracts/api.md`
- Modify: `ui/src/lib/api/types.ts`
- Modify: `ui/src/lib/stores/database.svelte.ts`
- Modify: `ui/src/modules/database/EvidenceBundleDialog.svelte`
- Modify: `ui/src/modules/database/ExplainWorkbench.svelte`
- Modify: `ui/src/modules/database/ResultsGrid.svelte`
- Modify: `docs/features/database-explorer.md`

- [ ] **Step 1: Write packet formatting tests.**

Context packets must include engine, environment, masking/truncation status,
query/result/explain summary, and no secrets.

Run:

```bash
cargo test -p otto-server context_packet -- --nocapture
```

Expected: FAIL if new packet shape is not implemented.

- [ ] **Step 2: Define DB investigation packet schema.**

Prefer using the existing context packet endpoints. Only add new fields if
current payloads cannot represent evidence/explain context safely.

- [ ] **Step 3: Wire UI entry points.**

Every relevant DB surface should have one consistent "Send to agent" path:
schema object, query result, evidence bundle, explain workbench, Redis key,
Mongo pipeline, ClickHouse analytics.

- [ ] **Step 4: Add docs.**

Update `docs/features/database-explorer.md` with the investigation flow.

- [ ] **Step 5: Run gates.**

```bash
cargo test -p otto-server context_packet -- --nocapture
cd ui && npm run check
```

Expected: PASS.

- [ ] **Step 6: Commit.**

```bash
git add crates/otto-server/src/context_packet.rs crates/otto-dbviewer/src/types.rs crates/otto-dbviewer/src/service.rs docs/contracts/api.md ui/src/lib/api/types.ts ui/src/lib/stores/database.svelte.ts ui/src/modules/database/EvidenceBundleDialog.svelte ui/src/modules/database/ExplainWorkbench.svelte ui/src/modules/database/ResultsGrid.svelte docs/features/database-explorer.md
git commit -m "feat(db): standardize agentic database investigation packets"
```

### Task 3.3: Workflow, API, Jira/Slack, and Vault Hooks

**Files:**
- Modify: `crates/otto-server/src/workflow_engine.rs`
- Modify: `crates/otto-server/src/context_packet.rs`
- Modify: `crates/otto-dbviewer/src/service.rs`
- Modify: `docs/features/workflows.md`
- Modify: `docs/features/api-client.md`
- Modify: `docs/features/database-explorer.md`
- Modify: `ui/src/lib/api/types.ts`
- Modify: `ui/src/modules/database/EvidenceBundleDialog.svelte`
- Modify: relevant Product/Slack/Vault UI only if existing APIs require UI glue.

- [ ] **Step 1: Add workflow test for DB evidence flow.**

Use existing `db_query`, `agent_prompt`, `api_run`, `channel_notify`, and
`human_approval` nodes. Prove DB results can be summarized without write access.

Run:

```bash
cargo test -p otto-server workflow_db_evidence -- --nocapture
```

Expected: FAIL until glue exists.

- [ ] **Step 2: Add evidence bundle destinations.**

From `EvidenceBundleDialog.svelte`, expose only destinations with available
permissions/configuration:

- copy Markdown.
- send to agent.
- draft Jira/Confluence comment.
- draft Slack/Telegram message.
- remember in Vault.
- use in workflow.

- [ ] **Step 3: Keep outward-facing actions approval-gated.**

Draft by default. Do not post to Jira/Confluence/Slack/Telegram without current
explicit approval.

- [ ] **Step 4: Update docs.**

Document DB evidence as the common artifact across API client, DB Explorer,
workflows, Slack/Telegram, Product, and Vault.

- [ ] **Step 5: Run gates.**

```bash
cargo test -p otto-server workflow_db_evidence -- --nocapture
cd ui && npm run check
```

Expected: PASS.

- [ ] **Step 6: Commit.**

```bash
git add crates/otto-server/src/workflow_engine.rs crates/otto-server/src/context_packet.rs crates/otto-dbviewer/src/service.rs docs/features/workflows.md docs/features/api-client.md docs/features/database-explorer.md ui/src/lib/api/types.ts ui/src/modules/database/EvidenceBundleDialog.svelte
git commit -m "feat(db): connect evidence bundles to Otto workflows"
```

## Milestone 4: UX Fit, Responsiveness, and Release Docs

### Task 4.1: Responsive and Accessibility Pass

**Files:**
- Modify: `ui/e2e/db-mobile.spec.ts`
- Modify: `ui/e2e/db-workbench-ux.spec.ts`
- Modify: `ui/src/modules/database/*.svelte` touched by previous tasks.

- [ ] **Step 1: Add e2e assertions.**

Cover desktop, tablet, mobile, dark/light, and RTL for:

- quick open.
- query library.
- evidence dialog.
- health panel.
- Redis inspector.
- Mongo pipeline lab.
- Explain workbench.

- [ ] **Step 2: Fix overflow and keyboard issues.**

All new dialogs/panels must fit mobile/tablet, have reachable buttons, and avoid
text overlap.

- [ ] **Step 3: Run e2e suite.**

```bash
cd ui && npm run test:e2e
```

Expected: PASS.

- [ ] **Step 4: Commit.**

```bash
git add ui/e2e/db-mobile.spec.ts ui/e2e/db-workbench-ux.spec.ts ui/src/modules/database
git commit -m "test(db): cover database workbench responsive workflows"
```

### Task 4.2: Final Documentation and Release Notes

**Files:**
- Modify: `docs/features/database-explorer.md`
- Modify: `docs/features/README.md`
- Modify: `docs/contracts/api.md`
- Modify: `docs/db/features/2026-06-25-database-workbench-design.md` if implementation changed the design.
- Modify: `docs/db/features/2026-06-25-database-workbench-implementation-plan.md` if task status needs updating.

- [ ] **Step 1: Update feature docs.**

Document the shipped behavior honestly. Do not document planned features as
shipped.

- [ ] **Step 2: Update contracts.**

Ensure every new route/type is in `docs/contracts/api.md`.

- [ ] **Step 3: Run full gates.**

```bash
cargo test --workspace
cd ui && npm run check
cd ui && npm run build
cd ui && npm run test:e2e
```

Expected: PASS. If a gate cannot run locally, capture the reason and the
smallest command that did run.

- [ ] **Step 4: Commit.**

```bash
git add docs/features/database-explorer.md docs/features/README.md docs/contracts/api.md docs/db/features/2026-06-25-database-workbench-design.md docs/db/features/2026-06-25-database-workbench-implementation-plan.md
git commit -m "docs(db): document database workbench experience"
```

## Final Verification Checklist

- [ ] `cargo test -p otto-dbviewer`
- [ ] `cargo test -p otto-state`
- [ ] `cargo test -p otto-server`
- [ ] `cargo test --workspace`
- [ ] `cd ui && npm run check`
- [ ] `cd ui && npm run build`
- [ ] `cd ui && npm run test:e2e`
- [ ] Manual: MySQL connection, quick open, query library, explain, evidence.
- [ ] Manual: Redis key inspector on seeded data.
- [ ] Manual: Mongo pipeline lab on seeded data.
- [ ] Manual: ClickHouse analytics loop on seeded data.
- [ ] Manual: production/read-only guardrails.
- [ ] Manual: masked evidence bundle sent to agent.
- [ ] Manual: docs/contracts match TypeScript API types.

## Follow-Up Splits

If this still feels too large during execution, split into these separate plans:

1. `production-read-mode-and-quick-open`
2. `query-library-and-evidence-bundles`
3. `connection-health-and-export-jobs`
4. `explain-workbench`
5. `redis-key-inspector`
6. `mongo-pipeline-lab`
7. `clickhouse-analytics-loop`
8. `agentic-db-investigation-and-workflows`
