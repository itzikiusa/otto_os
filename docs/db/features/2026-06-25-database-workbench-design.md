# Database Workbench Competitive UX Design

**Date:** 2026-06-25
**Status:** design
**Location:** `docs/db/features`
**Scope:** Database Explorer improvements for MySQL, Redis, MongoDB, and ClickHouse.

## Summary

Otto should not compete by becoming a broader copy of DBeaver, Navicat, TablePlus,
NoSQLBooster, or a shell. The stronger product is a safe, agent-aware database
workbench for developers who are already inside a repo, branch, ticket, API
debugging flow, workflow, or incident thread.

The Database Explorer already has a substantial baseline: multi-engine
connections, lazy schema browsing, query tabs, autocomplete, guarded inline
editing, exports, visual SQL builder, ERD, dashboards, and agent handoff. The
next step is to make the experience faster, safer, more engine-native, and more
connected to Otto's agent/workflow/product surfaces.

This design recommends three layers:

1. **Trust and speed foundation:** production read mode, quick open/search,
   saved query/history library, evidence bundles, connection health, and dialect
   guardrails.
2. **Engine-native workbenches:** Redis inspector, Mongo pipeline lab,
   ClickHouse analytics loop, and structured explain/performance review.
3. **Otto-specific moat:** agentic DB investigations, API+DB correlation,
   ticket-to-data-to-PR flows, Vault memory, workflow checks, and ChatOps/mobile
   evidence sharing.

## Inputs

- Current implementation docs: `docs/features/database-explorer.md`
- API contract: `docs/contracts/api.md`
- UI: `ui/src/modules/database/`, `ui/src/lib/stores/database.svelte.ts`
- Backend: `crates/otto-dbviewer/`, `crates/otto-state/src/db_explorer.rs`
- Existing DB e2e coverage: `ui/e2e/db-*.spec.ts`
- Competitor research performed on 2026-06-25:
  - DBeaver: AI assistance, visual query builder, data transfer, ERD, task scheduling.
  - TablePlus: fast native UX, safe mode, history/favorites, inline editing.
  - Navicat: visual explain, profiling, data dictionary, modeling, sync, BI.
  - NoSQLBooster/MongoDB Compass: Mongo-native aggregation/explain/query assistance.
  - Redis Insight: Redis-native key browser, profiling, memory/key tools, Copilot.
  - ClickHouse Cloud SQL Console: query console, visualization, shared queries,
    dashboards, progress-oriented analytics.
  - CLI tools: `mysql`, `redis-cli`, `mongosh`, `mongoexport`,
    `clickhouse-client`.

## Assumptions

The user explicitly asked not to ask questions, so the design assumes:

- Stay inside the current supported DB engines: MySQL, Redis, MongoDB,
  ClickHouse.
- Preserve the loopback/local-daemon security posture.
- Keep secrets in macOS Keychain; never persist secret values in SQLite,
  docs, logs, query history, evidence bundles, or agent packets.
- Make writes possible only through existing approval/guardrail patterns.
- Prefer incremental PRs over a single rewrite.
- Prefer existing Otto surfaces and code patterns over new product areas.
- Update `docs/contracts/api.md` and `ui/src/lib/api/types.ts` whenever an API
  shape changes.

## Product Positioning

**Position:** "The database workbench for agent-assisted engineering work."

Otto should win when the user needs to:

- Inspect data through a tunnel while working on a ticket.
- Understand why an API response differs from database state.
- Ask an agent to explain a schema/result safely.
- Turn a DB finding into a Jira comment, Slack update, workflow check, migration,
  or PR context.
- Operate against prod-like data without relying on shell muscle memory.

Otto does not need to win every standalone DBA/admin workflow. It should cover
the core daily developer flows well enough that users stop context-switching for
routine work, then differentiate with safety, agents, workflow automation, and
code/ticket awareness.

## Approaches Considered

### Approach A: Broad DB Client Parity

Replicate the largest feature sets from DBeaver/Navicat: advanced import/export,
data modeling, sync, task scheduling, diagram editing, BI dashboards, and every
engine-specific panel.

**Pros:** Clear parity story.
**Cons:** Huge scope, slow to ship, and competes directly against mature suites.
It does not use Otto's strongest assets.

### Approach B: Fast Native Developer Client

Prioritize TablePlus-style speed: quick open, query history, favorites, simple
editing, safe mode, and light schema navigation.

**Pros:** Good everyday UX and achievable.
**Cons:** Easy to become a smaller TablePlus unless combined with Otto-specific
workflows.

### Approach C: Safe Agentic Workbench

Build TablePlus-grade daily speed, then add production-safe guardrails,
engine-native inspectors, evidence bundles, and agent/workflow integration.

**Pros:** Uses Otto's existing agents, Jira/Confluence, Slack/Telegram, Vault,
workflows, API client, and local daemon. Creates a reason to switch.
**Cons:** Requires careful boundaries so DB Explorer does not become a vague
"AI everything" surface.

**Recommendation:** Approach C. Implement the speed/trust basics first, then
ship engine-native panels and Otto-specific investigation flows.

## Current Baseline

The existing Database Explorer already has these important foundations:

- Connection profiles for MySQL, Redis, MongoDB, and ClickHouse.
- TLS/SSH support through the daemon and connection profile resolver.
- macOS Keychain secret storage.
- Lazy schema tree with engine-shaped nodes.
- Query tabs persisted by workspace/connection.
- Per-engine autocomplete.
- Row limits, timeout flags, masking, cancel, explain, and save affordances.
- Virtualized result grid with search/filter/sort and CSV/JSON export.
- Approval-gated inline editing for supported result shapes.
- Local-file streaming export for large results.
- Structure view, indexes, DDL, table designer, visual SQL builder, ERD.
- Dashboards/widgets.
- Context packet handoff to agents.
- Existing e2e specs per DB engine.

Important improvement areas found during code review:

- Schema search is not a true server-backed quick open across unloaded objects.
- Saved queries and history are too thin for daily use.
- Export and dashboard infrastructure is stronger than the UI exposed around it.
- Dialect-specific affordances need stricter gating.
- Redis, MongoDB, and ClickHouse deserve more native workflows than a generic
  query editor and grid.
- Agent handoff exists, but the "investigation" flow is not yet a first-class
  workspace.

## Design Principles

1. **Safe by default:** production surfaces must show the environment, enforce
   limits/timeouts, preserve masking, and require approval for risky writes.
2. **Fast from keyboard:** open connection, schema object, saved query, or recent
   query from one quick-open surface.
3. **Engine-native when it matters:** Redis keys, Mongo pipelines, and ClickHouse
   analytics should not feel like SQL tables with JSON bolted on.
4. **Transparent and reproducible:** every generated query and significant DB
   action should expose the equivalent CLI command or audit/evidence summary.
5. **Agent-aware, not agent-dependent:** the user can work manually, then send a
   precise redacted packet to an agent when useful.
6. **No hidden data leakage:** samples, exports, context packets, history, and
   evidence bundles must mark masking/truncation state and avoid secrets.
7. **Incremental adoption:** every feature slice should stand alone and improve
   the current Database Explorer without requiring the full program to land.

## Feature Set

### 1. Production Read Mode

Production Read Mode is a connection/workspace runtime mode that makes dangerous
operations harder and routine reads safer.

**Behavior**

- Shows a persistent production/read-only banner near the active connection.
- Defaults read queries to conservative row limits and timeouts.
- Forces masking on by default for production unless the user explicitly disables
  it with an audited action.
- Rejects obvious unbounded/keyspace-wide Redis patterns such as `KEYS *` on
  production; guides the user to `SCAN`.
- Applies Mongo `maxTimeMS` and ClickHouse `max_execution_time` when a timeout is
  configured.
- Keeps existing write classification and typed confirmation, but upgrades the
  preview to show exactly why the statement is risky.
- Captures an audit/evidence record for confirmed writes, cancelled queries, and
  risky read overrides.

**Source of truth**

Production Read Mode is a computed backend view, not a UI preference. The
authoritative inputs are:

- `Connection.environment` (`prod` is guarded).
- `Connection.read_only`.
- Engine capabilities.
- Any future workspace-level DB safety defaults.

The UI receives this through `GET /connections/{id}/db/workbench-mode` and caches
it only for rendering. The UI must never be the source of truth for whether a
connection is guarded.

**Overrides and permissions**

- Viewers can see the mode.
- Editors can run read queries within the computed mode.
- Per-query overrides for mask/limit/timeout are tab-local, explicit, and
  auditable. They are not persisted as connection defaults.
- Only workspace admins/root can change the underlying connection environment or
  read-only flag through the existing connection management APIs.
- Confirmed guarded writes, risky read overrides, and query cancellations create
  audit/evidence records.

**Non-goal**

This does not create write automation. It makes the manual DB workflow safer.

### 2. Global DB Quick Open

Add a command-palette style surface scoped to the Database Explorer.

**Search targets**

- Connections and sections.
- Databases/keyspaces.
- Tables, views, collections, Redis keys/namespaces.
- Columns/fields when supported by the driver.
- Saved queries and recent history.
- Dashboards/widgets.

**Result actions**

- Open connection.
- Open structure.
- Insert starter query into editor.
- Run safe preview query.
- Set active database/keyspace.
- Open saved query/history in a new tab.
- Open dashboard/widget.
- Send object context to agent.

**Search architecture**

Global DB Quick Open is a workspace-level search surface made of two providers:

1. **Workspace provider:** searches local Otto records without touching a DB:
   connections, sections, saved queries, query history, dashboards, and widgets.
2. **Catalog provider:** searches live DB catalog objects for a selected/open
   connection through the engine driver.

The workspace endpoint owns result merging, ranking, and action metadata:
`POST /workspaces/{wid}/db/quick-open`. It may call the connection-scoped catalog
provider, `POST /connections/{id}/db/catalog-search`, for one active connection
or a bounded set of selected/open connections. Drivers implement only catalog
search; they do not know about saved queries, history, dashboards, or sections.

Drivers can use engine-native catalog queries and cap results. Redis should
require a prefix or use bounded `SCAN` sampling.

### 3. Query Library and History Manager

Replace the thin Saved/History lists with a real query library.

**Saved query fields**

- Name.
- Statement.
- Optional connection id.
- Tags.
- Folder/path.
- Favorite flag.
- Description/notes.
- Updated timestamp.
- Owner.

**History filters**

- Search statement text.
- Success/failure.
- Connection.
- Date range.
- Duration.
- Row count.
- "Promote to saved."

**History API shape**

History is exposed both connection-scoped and workspace-scoped:

- Connection-scoped history is optimized for the active DB tab.
- Workspace-scoped history powers the library manager and can filter by
  connection, status, date range, duration, row count, and text.

The API must support:

- `connection_id`
- `search`
- `ok`
- `date_from`
- `date_to`
- `min_duration_ms`
- `max_duration_ms`
- `min_rows`
- `max_rows`
- `limit`
- `cursor`

It must also support deleting one owned history row and clearing the caller's
own history for a connection or workspace. Root/workspace-admin access follows
the existing owner-private history model.

**UX**

- Saved and History tabs share a dense list component with search.
- Saved queries support rename, duplicate, edit metadata, delete, and run/open.
- History supports rerun, open in tab, copy, promote, and clear own history.
- Results can be saved with an optional evidence bundle.

### 4. Evidence Bundles

An evidence bundle is a redacted, shareable record of a DB investigation step.

**Bundle contents**

- Statement or operation.
- Connection name, engine, environment, active database/keyspace.
- Timestamp, duration, row count, truncation, timeout, and masking state.
- Result sample with redaction already applied.
- Explain/performance summary when available.
- Optional user note.

**Destinations**

- Copy as Markdown.
- Send to agent via context packet.
- Draft Jira/Confluence comment.
- Draft Slack/Telegram message.
- Attach to Vault memory.
- Save alongside query history.

This is a strong Otto differentiator because it turns data inspection into a
portable artifact that preserves safety context.

### 5. Connection Health and Tunnel Command Center

Add a health panel for the active connection.

**Fields**

- Engine, server version, latency, current active database.
- TLS mode, verification state, SNI/server name when known.
- SSH tunnel type (`-L` local forward or SOCKS5), local endpoint, target host,
  last error, idle timeout, and reuse state.
- Environment/read-only/production guardrail state.
- Test history.
- Equivalent CLI command with secrets redacted.

**Actions**

- Retest.
- Reopen tunnel.
- Copy redacted CLI command.
- Open terminal context using the same connection/tunnel where feasible.

### 6. Explain Workbench

Turn raw explain output into an actionable performance review.

**Supported first**

- MySQL `EXPLAIN` / `EXPLAIN ANALYZE` when available.
- ClickHouse `EXPLAIN indexes = 1`, query stats, rows/bytes scanned.
- MongoDB `explain()` for find/aggregate.

**UI**

- Structured plan tree/table.
- Warnings for full scans, missing indexes, high rows read, high bytes read,
  poor row estimate, blocking sort, no shard/key usage when detectable.
- "Compare with previous run" for two variants.
- "Ask agent" packet that includes plan, statement, schema object detail, and
  result stats.
- Optional "draft index/migration" action that creates code context only; it
  must not apply migrations automatically.

### 7. Redis Safe Key Inspector

Redis needs a first-class key workflow.

**Capabilities**

- Prefix browser powered by bounded `SCAN`.
- Key detail: type, TTL, memory usage where available, encoding, length/cardinality.
- Type-specific viewers:
  - string: text/json/binary preview.
  - hash: field table with search.
  - list: paged range.
  - set: paged members.
  - zset: score/member table.
  - stream: entries and consumer groups when available.
- Safe actions with confirmation: set TTL, persist TTL, delete key, update field.
- Production warnings for destructive actions and broad scans.

### 8. Mongo Pipeline Lab

MongoDB should have a pipeline-first workflow.

**Capabilities**

- Stage editor with JSON validation.
- Stage-by-stage preview.
- `maxTimeMS`, limit, read preference, and active database/collection controls.
- Explain panel.
- Save pipeline variants.
- Export to language snippets where useful.
- Send pipeline/result/explain to agent.

### 9. ClickHouse Analytics Loop

ClickHouse users care about iteration speed and scanned data cost.

**Capabilities**

- Query progress: rows read, bytes read, elapsed, memory when available.
- Query variants/history comparison.
- One-click chart from result.
- Explain/index usage view.
- Export in ClickHouse-friendly formats.
- Dashboard widget promotion with editable mapping/layout.

### 10. Dashboard and Export Polish

The dashboard model already contains layout and widget update support. The UI
should expose it.

**Dashboard improvements**

- Edit mode with drag/resize/reorder.
- Widget settings: query, connection, viz, mapping, title, refresh cadence.
- Duplicate widget.
- Open widget query in editor.
- Add current explain/evidence note to widget metadata.

**Export improvements**

- Export job history for local-file exports.
- Progress survives navigation.
- Reveal local file on daemon host.
- Cancel long export when supported or abort client stream with clear messaging.
- Explicit "interactive export" vs "daemon local-file export" copy.

### 11. Agentic and Cross-Module DB Workflows

This is the Otto-specific layer that turns DB work into engineering output.

**Agentic DB Investigation Workspace**

- Every DB artifact can become an evidence-backed context packet: schema object,
  query result, explain result, Redis key detail, Mongo pipeline preview, and
  ClickHouse analytics summary.
- Packets include engine, connection label, environment, active database/keyspace,
  masking/truncation state, statement/operation, result sample, and timing.
- The packet preview is redacted before injection into an agent session.
- The agent can explain, propose next queries, draft a migration, or draft a PR
  plan, but any DB execution still routes through the human-visible editor and
  existing guardrails.

**API+DB correlation**

- An API-client response can be bundled with one or more DB evidence bundles.
- The bundle answers "request X returned Y; database state Z explains it."
- The shared artifact can be sent to an agent, copied to Markdown, or attached to
  product/debugging notes.

**Ticket-to-data-to-PR**

- Evidence bundles can be drafted into Jira/Confluence comments.
- Agents can use the bundle as input to a code/migration plan.
- Generated migrations or code changes are created through the normal repo/PR
  flow, not applied directly from DB Explorer.

**Workflow and ChatOps checks**

- Workflows can run read-only `db_query`, attach an evidence summary, send it to
  `agent_prompt`, then draft or send a `channel_notify` after approval.
- Slack/Telegram actions draft by default unless the user explicitly approves an
  outward-facing send.

**Vault memory**

- Users can save stable learnings such as schema conventions, dangerous tables,
  known indexes, incident notes, and verified query explanations to Vault.
- Vault entries store redacted summaries, not raw result sets.

**Mobile on-call view**

- Mobile support is limited to safe triage: view evidence, inspect masked result
  samples, send to agent, and draft notifications.
- Destructive DB actions remain guarded and should not become a casual mobile
  flow.

## Architecture

### Backend Boundaries

The existing `otto-dbviewer` layering remains right:

- `crates/otto-dbviewer/src/driver.rs`
  - Defines engine-specific behavior behind a trait.
  - Add optional methods with default unsupported results rather than new
    standalone subsystems.
- `crates/otto-dbviewer/src/service.rs`
  - Resolves profiles/secrets/tunnels.
  - Applies guardrails, masking, history, audit, and cross-engine orchestration.
  - Owns production read mode enforcement.
- `crates/otto-dbviewer/src/http.rs`
  - Exposes typed REST routes.
  - Keeps RBAC checks close to routes.
- `crates/otto-state/src/db_explorer.rs`
  - Persists saved queries, history, dashboards/widgets, export jobs, and
    evidence bundles.
- `crates/otto-server/src/context_packet.rs`
  - Reused for redacted packet preview/send. Evidence bundles can build on the
    same redaction rules.

### Proposed Driver Extensions

Add optional methods to `Driver` with default unsupported behavior:

```rust
async fn catalog_search(&self, cfg: &ResolvedConfig, req: &CatalogSearchReq)
    -> Result<Vec<CatalogSearchHit>>;

async fn health(&self, cfg: &ResolvedConfig) -> Result<DbHealth>;

async fn explain(&self, cfg: &ResolvedConfig, req: &ExplainRequest)
    -> Result<ExplainResult>;

async fn redis_key_detail(&self, cfg: &ResolvedConfig, req: &RedisKeyReq)
    -> Result<RedisKeyDetail>;

async fn mongo_pipeline_preview(&self, cfg: &ResolvedConfig, req: &MongoPipelinePreviewReq)
    -> Result<MongoPipelinePreview>;
```

Only engines that support a method override it. The UI should gate affordances
from `DbCapabilities` and route errors gracefully.

### Proposed Types

Add Rust types in `crates/otto-dbviewer/src/types.rs` and mirror them in
`ui/src/lib/api/types.ts`:

- `DbWorkbenchMode`
- `QuickOpenReq`, `QuickOpenHit`
- `CatalogSearchReq`, `CatalogSearchHit`
- `DbHealth`
- `ExplainRequest`, `ExplainResult`, `ExplainWarning`
- `EvidenceBundleReq`, `EvidenceBundle`
- `SavedQueryPatch`, extended `SavedQuery`
- `HistoryQuery`, `HistoryPage`
- `ExportJob`
- `RedisKeyReq`, `RedisKeyDetail`, `RedisKeyMutationReq`
- `MongoPipelinePreviewReq`, `MongoPipelinePreview`
- `ClickHouseQueryProgress`

### Persistence

Add an append-only migration after the current latest migration:

- Extend `db_saved_queries` with tags/folder/favorite/description/updated fields
  or create side tables if migration compatibility requires it.
- Add `db_query_folders` for saved query organization.
- Add `db_evidence_bundles`.
- Add `db_export_jobs`.
- Consider query history indexes for `user_id`, `connection_id`, `created_at`,
  `ok`, and text search. Keep SQLite-compatible search first; defer FTS unless
  plain `LIKE` becomes too slow.

### UI Boundaries

Keep `DatabasePage.svelte` as the shell, but move feature-specific complexity
into focused components:

- `QuickOpen.svelte`
- `QueryLibraryPanel.svelte`
- `ConnectionHealthPanel.svelte`
- `ProductionReadModeBanner.svelte`
- `EvidenceBundleDialog.svelte`
- `ExplainWorkbench.svelte`
- `RedisInspector.svelte`
- `MongoPipelineLab.svelte`
- `ClickHouseAnalyticsPanel.svelte`
- `DashboardEditor.svelte`
- `ExportJobsPanel.svelte`

`ui/src/lib/stores/database.svelte.ts` remains the orchestration store, but new
state should be grouped by feature and kept small. If a feature needs complex
local UI state, keep it in that component rather than expanding the central
store unnecessarily.

## API Sketch

All paths remain under `/api/v1`.

```text
GET  /connections/{id}/db/workbench-mode
GET  /connections/{id}/db/health
POST /workspaces/{wid}/db/quick-open
POST /connections/{id}/db/catalog-search
POST /connections/{id}/db/explain
POST /connections/{id}/db/evidence-bundles
GET  /connections/{id}/db/export-jobs
POST /connections/{id}/db/export-jobs/{job_id}/cancel

GET   /workspaces/{wid}/db/saved-queries?search=&tag=&folder=&favorite=&connection_id=
POST  /workspaces/{wid}/db/saved-queries
PATCH /db/saved-queries/{qid}
DELETE /db/saved-queries/{qid}

GET    /connections/{id}/db/history?search=&ok=&date_from=&date_to=&min_duration_ms=&max_duration_ms=&min_rows=&max_rows=&limit=&cursor=
GET    /workspaces/{wid}/db/history?connection_id=&search=&ok=&date_from=&date_to=&min_duration_ms=&max_duration_ms=&min_rows=&max_rows=&limit=&cursor=
DELETE /db/history/{hid}
DELETE /connections/{id}/db/history
DELETE /workspaces/{wid}/db/history

POST /connections/{id}/db/redis/key
POST /connections/{id}/db/redis/key/mutate

POST /connections/{id}/db/mongo/pipeline-preview
POST /connections/{id}/db/clickhouse/progress

POST /workspaces/{wid}/db/evidence-bundles/{bid}/destinations/agent
POST /workspaces/{wid}/db/evidence-bundles/{bid}/destinations/vault
POST /workspaces/{wid}/db/evidence-bundles/{bid}/destinations/workflow
```

The exact route count can be reduced during implementation if existing
`/db/query`, `/db/object`, and `/db/widgets` routes cover the behavior cleanly.
Do not hide engine-specific semantics behind generic routes when doing so would
make validation, safety, or typing worse.

## Safety Model

- RBAC follows existing Database rules: view for schema/search/history read,
  edit for query execution/export/saved query mutation, admin/root for global
  connection management.
- Production/read-only connections default to read-only behavior.
- Writes and destructive Redis mutations require confirmation on guarded
  connections.
- Evidence bundles and agent packets store or transmit already-redacted content.
- Query history and saved queries remain owner-private unless an explicit sharing
  feature is introduced later.
- Export jobs write to the daemon host. UI copy must keep that clear.
- Equivalent CLI commands must redact secrets and private key paths where needed.

## Testing Strategy

### Rust

- Unit-test parsing/classification helpers.
- Add driver tests for catalog-search request normalization, explain parsing,
  Redis key detail parsing, and Mongo pipeline request validation.
- Add service tests for production read mode enforcement and evidence bundle
  redaction.
- Run:
  - `cargo test -p otto-dbviewer`
  - `cargo test -p otto-state`
  - `cargo test --workspace` before broad integration.

### UI

- Keep `npm run check` as the type gate.
- Extend existing DB e2e specs:
  - `ui/e2e/db-sweep-mysql.spec.ts`
  - `ui/e2e/db-sweep-redis.spec.ts`
  - `ui/e2e/db-sweep-mongodb.spec.ts`
  - `ui/e2e/db-sweep-clickhouse.spec.ts`
  - `ui/e2e/db-mobile.spec.ts`
  - `ui/e2e/db-export.spec.ts`
- Add `ui/e2e/db-workbench-ux.spec.ts` for cross-engine shell features:
  quick open, query library, evidence bundle dialog, health panel, and responsive
  fit.

### Manual QA

- Test with one connection per engine.
- Test production/read-only flags and guardrail confirmations.
- Test through SSH tunnels.
- Test masked/unmasked result bundles.
- Test long-running export progress.
- Test mobile/tablet layouts.

## Phased Delivery

### Phase 0: Stabilize and Polish

- Contract audit for DB export and current docs.
- Dialect guardrails for builder/designer.
- Better inline-edit diagnostics.
- Saved/history list search and promote actions.
- Clearer export copy and export job surface.

### Phase 1: Trust and Speed

- Production Read Mode.
- Global DB Quick Open.
- Query Library and History Manager.
- Connection Health Panel.
- Evidence Bundle Dialog.

### Phase 2: Engine-Native Workbenches

- Redis Safe Key Inspector.
- Explain Workbench for MySQL, MongoDB, ClickHouse.
- Mongo Pipeline Lab.
- ClickHouse Analytics Loop.
- Dashboard editor mode.

### Phase 3: Otto Moat

- Agentic DB Investigation Workspace.
- API+DB correlation workbench.
- Ticket-to-data-to-PR evidence flow.
- Vault-backed DB memory.
- Workflow DB checks and ChatOps triage.
- Mobile on-call DB evidence view.

## Success Metrics

- Time from opening DB page to running a safe query decreases.
- Users can find a table/collection/key/query from quick open without expanding
  the schema tree manually.
- More queries are run from saved/history than retyped from scratch.
- Production queries mostly run with masking/limits/timeouts enabled.
- Evidence bundles are used in Jira/Slack/agent flows.
- Redis/Mongo/ClickHouse users perform native workflows without switching tools.
- E2E DB specs stay green across desktop, tablet, mobile, light/dark, and RTL.

## Open Risks

- The feature set is large. It must be delivered in independent PRs.
- Adding many routes can bloat `http.rs`; keep request handling thin and push
  logic into service/driver methods.
- The Svelte store can grow too large. Feature-local UI state should stay in
  feature components.
- Explain parsing can become fragile. Start with structured raw output plus a
  few high-confidence warnings; do not overpromise optimizer advice.
- Redis mutation UX can become risky. Keep production defaults conservative and
  require confirmation for destructive operations.
- Cross-module agent/Jira/Slack/Vault integration can sprawl. Evidence bundles
  should be the common artifact.
