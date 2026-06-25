# Database Transfer and Mongo Power Tools Design

## Summary

Otto can offer a Workbench-style data transfer experience and a NoSQLBooster-style
MongoDB workspace, but the strongest product shape is not a one-for-one clone. The
winning angle is an Otto-native database workbench: guarded data movement,
repeatable transfer profiles, Mongo-specific query/build/explain tools, and direct
handoff to agents, sessions, PRs, Jira stories, saved evidence, and workflows.

This document defines the feature set, architecture, API shape, state model, UI
flows, safety model, rollout plan, and acceptance criteria for:

- **Otto Data Transfer**: guided export, import, and connection-to-connection copy
  across MySQL, MongoDB, ClickHouse, and later Redis.
- **Mongo Power Tools**: Mongo-focused aggregation building, visual explain,
  document editing, schema analysis, SQL-to-Mongo assistance, snippets, and
  import/export integration.

The companion implementation plan is
[`data-transfer-and-mongo-power-tools-plan.md`](./data-transfer-and-mongo-power-tools-plan.md).

## Context

Otto already has a strong Database Explorer foundation:

- Connection-backed browsing for **MySQL, Redis, MongoDB, and ClickHouse**.
- Lazy schema tree and object details.
- Query tabs with row limits, query history, cancellation where supported, and
  per-engine completion.
- Virtualized results grid with guarded inline edits.
- Visual SQL join builder and schema diagram.
- ClickHouse dashboard widgets.
- Agent handoff with context packets.
- Streaming export through the existing DB export endpoints.

Relevant existing modules:

| Area | Current location |
| --- | --- |
| User-facing DB feature doc | `docs/features/database-explorer.md` |
| API contract | `docs/contracts/api.md` |
| UI page shell | `ui/src/modules/database/DatabasePage.svelte` |
| DB store and API calls | `ui/src/lib/stores/database.svelte.ts` |
| API types | `ui/src/lib/api/types.ts` |
| DB HTTP routes | `crates/otto-dbviewer/src/http.rs` |
| DB service | `crates/otto-dbviewer/src/service.rs` |
| DB domain types | `crates/otto-dbviewer/src/types.rs` |
| Export engine | `crates/otto-dbviewer/src/export.rs` |
| State tables | `crates/otto-state/src/db_explorer.rs` and migrations |
| Mongo driver/parser | `crates/otto-dbviewer/src/drivers/mongodb.rs`, `mongo_parse.rs`, `mongo_sql.rs` |

The gap is workflow depth. Competitors win when users need to move data, map
fields, reason about Mongo pipelines, debug query plans, or repeat work safely.
Otto should close those gaps in a way that uses its own strengths.

## Reference Research

Public product references reviewed for feature shape:

- MySQL Workbench data export/import and migration:
  - https://dev.mysql.com/doc/workbench/en/wb-admin-export-import.html
  - https://dev.mysql.com/doc/workbench/en/wb-admin-export-import-table.html
  - https://dev.mysql.com/doc/workbench/en/wb-admin-export-import-management.html
  - https://dev.mysql.com/doc/workbench/en/wb-migration.html
- DBeaver data transfer, ER diagrams, data editing, task management, query
  manager, and data compare:
  - https://dbeaver.com/docs/dbeaver/Data-transfer/
  - https://dbeaver.com/docs/dbeaver/ER-Diagrams/
  - https://dbeaver.com/docs/dbeaver/Data-View-and-Format/
- Navicat Premium all-in-one database tooling:
  - https://www.navicat.com/en/products/navicat-premium
- TablePlus connection, filtering, editing, and safe mode docs:
  - https://docs.tableplus.com/
- NoSQLBooster Mongo-focused features and AI helper:
  - https://nosqlbooster.com/features
  - https://nosqlbooster.com/blog/ai-helper-for-mongodb/

The resulting design is an inference from those products plus Otto's current
codebase. It should be treated as a product design, not a claim that every
competitor supports every item in exactly this form.

## Product Positioning

### What Otto Should Offer

Otto should offer a database workbench that is safer and more workflow-aware than
CLI-only work, and more agent-native than DBeaver, Navicat, TablePlus, Workbench,
or NoSQLBooster.

The feature should push users toward Otto by making common DB work:

- Easier to start: guided flows from existing connections, schema tree, result
  grids, and query tabs.
- Safer to execute: dry runs, row counts, sampled previews, write confirmation,
  read-only/production guards, and auditable run history.
- Easier to repeat: saved transfer profiles, reusable Mongo snippets, query
  history, and workflow integration.
- Easier to explain: generated mapping summaries, validation reports, query
  explain summaries, and agent-ready evidence packets.
- Easier to connect to development work: attach transfer runs or Mongo explain
  summaries to sessions, PRs, Jira stories, and investigation notes.

### Non-goals

Phase 1 is not a full enterprise ETL platform, CDC engine, or background
scheduler. It should not:

- Attempt live replication, bidirectional sync, or conflict resolution.
- Run destructive migrations without explicit confirmation and dry-run output.
- Replace specialist DBA tools for every administration task.
- Auto-convert every engine-specific type perfectly across all engines.
- Require cloud services or remote execution.

## Feature 1: Otto Data Transfer

### User Jobs

Users should be able to:

1. Export a table, collection, query result, or filtered result set to CSV, TSV,
   JSON array, or NDJSON.
2. Import CSV, TSV, JSON array, or NDJSON into an existing table/collection.
3. Create a new table/collection from a file with inferred schema and editable
   mapping.
4. Copy data between two Otto DB connections with preview, mapping, dry run,
   progress, cancellation, and validation.
5. Save a transfer profile and rerun it later with the same mapping and options.
6. Review past transfer runs with row counts, duration, errors, sample rejects,
   and validation evidence.
7. Hand a transfer run to an agent as structured context.

### Phase 1 Scope

Phase 1 should ship a useful and safe Workbench-style equivalent:

- Import preview for local files:
  - CSV and TSV with delimiter/header/quote/null handling.
  - JSON array and NDJSON.
  - Local daemon-host path input in web/dev mode.
  - Desktop file picker integration where practical.
- Mapping editor:
  - Source fields to target columns/fields.
  - Target type inference with user override.
  - Required/nullable indicators.
  - Default values and skip-field support.
  - Duplicate key behavior for Mongo and conflict behavior for SQL where
    supported.
- Dry run:
  - Parse row count.
  - Sample rows.
  - Target compatibility checks.
  - Estimated writes and first N validation errors.
- Import run:
  - Batches.
  - Progress and cancellation.
  - Failed-row sample report.
  - Persisted run history.
- Export polish:
  - Existing export endpoints remain the execution engine.
  - UI gets a wizard and saved presets.
  - Export can start from schema tree, results grid, or query tab.
- Saved profiles:
  - Import/export/copy profile metadata and options.
  - Rerun from the Database Explorer.

### Phase 2 Scope

Phase 2 adds connection-to-connection copy:

- Same-engine table/collection copy:
  - MySQL -> MySQL.
  - MongoDB -> MongoDB.
  - ClickHouse -> ClickHouse.
- Cross-engine copy with explicit lossy mapping warnings:
  - MySQL <-> MongoDB for common scalar types.
  - MySQL/CSV/NDJSON -> ClickHouse append.
- Create target object:
  - SQL `CREATE TABLE` preview.
  - Mongo collection creation is optional because collections are implicit.
  - ClickHouse DDL preview requires engine/order key selection.
- Validation:
  - Source count vs inserted count.
  - Optional sampled checksum for stable scalar columns.
  - Target sample preview.

### Later Scope

Later work can add:

- Scheduled transfer profiles via Otto workflows.
- SQL dump import/export for MySQL-compatible databases.
- Data compare after transfer.
- Data masking profiles.
- Redis key import/export with stronger safety prompts.
- Template library for common Mongo/SQL transformations.
- Agent-generated transfer profiles from Jira or PR context.

## Feature 2: Mongo Power Tools

### User Jobs

MongoDB users should be able to:

1. Build aggregation pipelines visually without losing access to raw code.
2. Preview output after each stage.
3. Explain a query or pipeline and see the plan in a readable form.
4. Edit documents with Extended JSON awareness and guarded write review.
5. Understand a collection's inferred schema, common fields, indexes, and data
   shape.
6. Write SQL-like queries when that is faster, and see the generated Mongo
   command.
7. Save reusable snippets and pipelines.
8. Import/export collection data through the shared transfer system.
9. Send a query, sample output, schema, and explain summary to an agent.

### Phase 1 Scope

Mongo Power Tools Phase 1 should ship:

- Aggregation Pipeline Builder:
  - Stage cards for `$match`, `$project`, `$group`, `$sort`, `$limit`, `$skip`,
    `$unwind`, `$lookup`, `$addFields`, `$count`, and raw stage.
  - Reorder, duplicate, disable, and delete stages.
  - Raw JSON editor per stage.
  - Full generated `db.collection.aggregate([...])` command.
  - Preview after selected stage using the current row limit.
- Visual Explain:
  - Runs existing Mongo explain path.
  - Extracts `executionStats`, `queryPlanner`, winning plan, index usage,
    collection scan warnings, docs examined, docs returned, duration, and
    rejected plans where present.
  - Displays readable summary plus raw JSON.
- Collection Schema Analyzer:
  - Samples documents.
  - Shows inferred fields, observed BSON types, missing/null rate, example value,
    nesting, and whether a field appears in an index.
  - Builds on existing sampled schema tree behavior.
- Snippets:
  - Saved Mongo snippets and pipelines scoped to workspace and optionally
    collection.
  - Insert snippet into editor or open in builder.
- SQL-to-Mongo assistant:
  - Reuses existing `mongo_sql` support.
  - Makes the translated command visible and copyable.
  - Explains unsupported SQL constructs in product language.

### Phase 2 Scope

Phase 2 can add:

- Profiler/currentOp panels for authorized users.
- Index advisor from explain results and sampled filters.
- Schema drift snapshots.
- Reference following for ObjectId-like fields.
- GridFS browser.
- Mock data generator.
- Script playground for multi-statement Mongo sessions, with explicit write
  review and no hidden writes.

## Shared Experience Model

### Navigation

Add two first-class surfaces inside the existing Database Explorer:

- **Transfer** tab or toolbar action:
  - Available for all DB connections.
  - Entry points from schema tree, results grid, query tab, and connection list.
- **Mongo Tools** tab:
  - Visible when the active connection is MongoDB.
  - Empty state for non-Mongo engines should point to Transfer, Query, Structure,
    and Diagram instead of showing irrelevant Mongo controls.

### Transfer Wizard Flow

The data transfer flow should be:

1. **Choose operation**
   - Export.
   - Import from file.
   - Copy from connection/query to connection/object.
2. **Choose source**
   - Table/collection.
   - Query result.
   - Local file.
3. **Choose target**
   - Existing table/collection.
   - New table/collection.
   - Local file for export.
4. **Preview**
   - First rows/documents.
   - Inferred columns/fields.
   - Target object metadata.
5. **Map**
   - Source -> target field mapping.
   - Type conversion.
   - Defaults/skips.
   - Conflict behavior.
6. **Dry run**
   - Validate file parse.
   - Validate target compatibility.
   - Show estimated impact.
   - Require confirmation for writes.
7. **Run**
   - Progress by rows, bytes, batches, elapsed time.
   - Cancel.
   - Show errors without dumping secrets or full sensitive rows.
8. **Validate**
   - Inserted/updated/skipped/failed counts.
   - Sample rejects.
   - Optional target sample.
9. **Save / hand off**
   - Save as profile.
   - Copy summary.
   - Send to agent.
   - Open result in query tab.

### Mongo Builder Flow

The Mongo builder flow should be:

1. User selects a collection and opens **Mongo Tools -> Pipeline**.
2. Builder seeds `db.<collection>.aggregate([])`.
3. User adds stage cards and edits JSON.
4. Each stage validates locally for JSON shape.
5. User clicks **Preview stage** or **Run pipeline**.
6. Otto sends generated command through the existing Mongo query runner.
7. Results appear in the normal results grid.
8. Explain opens the Visual Explain panel.
9. User can save the pipeline as a snippet/profile or hand it to an agent.

## Backend Architecture

### Module Layout

Add a transfer subsystem to `otto-dbviewer` without turning the existing driver
trait into an ETL framework.

Proposed files:

| File | Responsibility |
| --- | --- |
| `crates/otto-dbviewer/src/transfer/mod.rs` | Public transfer service module wiring |
| `crates/otto-dbviewer/src/transfer/types.rs` | Internal transfer domain types if they grow too large for `types.rs` |
| `crates/otto-dbviewer/src/transfer/preview.rs` | File sniffing, format detection, sample parse, inferred schema |
| `crates/otto-dbviewer/src/transfer/mapping.rs` | Mapping validation and type conversion planning |
| `crates/otto-dbviewer/src/transfer/run.rs` | Run coordinator, batching, progress, cancellation |
| `crates/otto-dbviewer/src/transfer/writers.rs` | Engine-specific write adapters |
| `crates/otto-dbviewer/src/mongo_tools.rs` | Mongo explain shaping, schema analyzer, pipeline validation |

Keep the existing `Driver` trait focused on query/browse/export. Transfer writes
should use small engine adapters that call existing service resolution and driver
helpers where possible.

### Domain Types

API-visible types should live in `crates/otto-dbviewer/src/types.rs` and mirror
to `ui/src/lib/api/types.ts`.

Core transfer types:

```rust
pub enum TransferKind {
    Export,
    Import,
    Copy,
}

pub enum TransferFormat {
    Csv,
    Tsv,
    JsonArray,
    Ndjson,
}

pub enum TransferEndpoint {
    LocalFile {
        path: String,
        format: TransferFormat,
    },
    DbObject {
        connection_id: String,
        engine: Engine,
        database: Option<String>,
        schema: Option<String>,
        object: String,
    },
    Query {
        connection_id: String,
        engine: Engine,
        database: Option<String>,
        statement: String,
        max_rows: Option<u32>,
    },
}

pub struct TransferFieldMapping {
    pub source: String,
    pub target: String,
    pub target_type: Option<String>,
    pub nullable: Option<bool>,
    pub default_value: Option<serde_json::Value>,
    pub skip: bool,
}

pub enum TransferRunStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}
```

Request/response types:

- `TransferPreviewRequest`
- `TransferPreviewResponse`
- `TransferDryRunRequest`
- `TransferDryRunResponse`
- `TransferRunRequest`
- `TransferRunResponse`
- `TransferRunSummary`
- `TransferRunEvent`
- `TransferProfile`
- `TransferProfileUpsert`
- `MongoPipelinePreviewRequest`
- `MongoPipelinePreviewResponse`
- `MongoExplainSummary`
- `MongoSchemaAnalysisRequest`
- `MongoSchemaAnalysisResponse`
- `MongoSnippet`

### Engine Writers

Each engine writer should expose a small internal contract:

```rust
#[async_trait]
trait TransferWriter {
    async fn inspect_target(&self, target: &TransferEndpoint) -> Result<TargetShape>;
    async fn dry_run(&self, plan: &TransferPlan) -> Result<TransferDryRunReport>;
    async fn write_batch(&self, batch: TransferBatch) -> Result<TransferBatchReport>;
}
```

Initial implementation order:

1. **MongoDB writer**
   - Converts JSON values to BSON using the existing Extended JSON conversion
     helpers where possible.
   - Supports insert-only first.
   - Adds upsert/replace later.
2. **MySQL writer**
   - Uses parameterized batched `INSERT`.
   - Supports append first.
   - Adds truncate/replace/upsert later.
3. **ClickHouse writer**
   - Uses `INSERT INTO ... FORMAT JSONEachRow` or parameterized batches depending
     on current driver support.
   - Append-only first.
4. **Redis writer**
   - Defer writes until safety model is tighter.
   - Phase 1 may export Redis keys only.

### File Handling

Otto must be explicit that import/export paths are on the daemon host, which is
usually the user's Mac.

MVP behavior:

- Import preview/run accepts a local daemon-host path.
- Export-to-path keeps using the existing path-based export endpoint.
- UI labels the path as "Local path on this Mac".
- Desktop shell can add native open/save file picker commands as a UX
  enhancement. Browser/dev mode keeps a manual path input.

Do not add browser multipart uploads as the first path. Large DB imports should
stream from disk through the daemon to avoid loading whole files into the
webview.

### Progress and Cancellation

Transfer runs should be persisted because they can outlive a modal and should be
auditable.

Progress model:

- `POST .../transfer/runs` creates and starts a run.
- `GET .../transfer/runs/{run_id}` returns current status, counters, and recent
  events.
- `POST .../transfer/runs/{run_id}/cancel` requests cancellation.
- UI polls initially; WebSocket events can be added later if the app already has
  an appropriate DB event channel.

Run counters:

- `bytes_read`
- `rows_read`
- `rows_written`
- `rows_skipped`
- `rows_failed`
- `batches_completed`
- `started_at`
- `finished_at`
- `elapsed_ms`

### Persistence

Add append-only migrations under `crates/otto-state/migrations/`.

Proposed tables:

```sql
CREATE TABLE db_transfer_profiles (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  source_json TEXT NOT NULL,
  target_json TEXT NOT NULL,
  mapping_json TEXT NOT NULL,
  options_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE db_transfer_runs (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  profile_id TEXT,
  kind TEXT NOT NULL,
  status TEXT NOT NULL,
  source_json TEXT NOT NULL,
  target_json TEXT NOT NULL,
  mapping_json TEXT NOT NULL,
  options_json TEXT NOT NULL,
  counters_json TEXT NOT NULL,
  error TEXT,
  created_at TEXT NOT NULL,
  started_at TEXT,
  finished_at TEXT
);

CREATE TABLE db_transfer_run_events (
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  ts TEXT NOT NULL,
  level TEXT NOT NULL,
  code TEXT NOT NULL,
  message TEXT NOT NULL,
  details_json TEXT,
  FOREIGN KEY(run_id) REFERENCES db_transfer_runs(id) ON DELETE CASCADE
);

CREATE TABLE db_mongo_snippets (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  connection_id TEXT,
  database_name TEXT,
  collection_name TEXT,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  body TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
```

The repository layer should sit in `crates/otto-state/src/db_explorer.rs` unless
that file is already too large; if it is, split to `db_transfer.rs` and re-export
from state initialization.

### API Contract

Add endpoints to `docs/contracts/api.md` and mirror all types in
`ui/src/lib/api/types.ts`.

Connection-scoped file import:

| Endpoint | Permission | Request | Response |
| --- | --- | --- | --- |
| `POST /connections/{id}/db/import/preview` | `ws viewer` | `TransferPreviewRequest` | `TransferPreviewResponse` |
| `POST /connections/{id}/db/import/dry-run` | `ws editor` | `TransferDryRunRequest` | `TransferDryRunResponse` |
| `POST /connections/{id}/db/import/run` | `ws editor` | `TransferRunRequest` | `TransferRunResponse` |

Workspace-scoped transfer runs:

| Endpoint | Permission | Request | Response |
| --- | --- | --- | --- |
| `POST /workspaces/{workspace_id}/db-transfer/preview` | `ws viewer` | `TransferPreviewRequest` | `TransferPreviewResponse` |
| `POST /workspaces/{workspace_id}/db-transfer/dry-run` | `ws editor` | `TransferDryRunRequest` | `TransferDryRunResponse` |
| `POST /workspaces/{workspace_id}/db-transfer/runs` | `ws editor` | `TransferRunRequest` | `TransferRunResponse` |
| `GET /workspaces/{workspace_id}/db-transfer/runs` | `ws viewer` | query filters | `TransferRunSummary[]` |
| `GET /workspaces/{workspace_id}/db-transfer/runs/{run_id}` | `ws viewer` | none | `TransferRunSummary` |
| `POST /workspaces/{workspace_id}/db-transfer/runs/{run_id}/cancel` | `ws editor` | none | `204` |
| `GET /workspaces/{workspace_id}/db-transfer/profiles` | `ws viewer` | query filters | `TransferProfile[]` |
| `POST /workspaces/{workspace_id}/db-transfer/profiles` | `ws editor` | `TransferProfileUpsert` | `TransferProfile` |
| `PATCH /workspaces/{workspace_id}/db-transfer/profiles/{profile_id}` | `ws editor` | `TransferProfileUpsert` | `TransferProfile` |
| `DELETE /workspaces/{workspace_id}/db-transfer/profiles/{profile_id}` | `ws editor` | none | `204` |

Mongo tools:

| Endpoint | Permission | Request | Response |
| --- | --- | --- | --- |
| `POST /connections/{id}/db/mongo/pipeline/preview` | `ws viewer` | `MongoPipelinePreviewRequest` | `MongoPipelinePreviewResponse` |
| `POST /connections/{id}/db/mongo/sql/translate` | `ws viewer` | `MongoSqlTranslateRequest` | `MongoSqlTranslateResponse` |
| `POST /connections/{id}/db/mongo/explain` | `ws viewer` | `MongoPipelinePreviewRequest` or `QueryRequest` | `MongoExplainSummary` |
| `POST /connections/{id}/db/mongo/schema-analysis` | `ws viewer` | `MongoSchemaAnalysisRequest` | `MongoSchemaAnalysisResponse` |
| `GET /workspaces/{workspace_id}/db-mongo/snippets` | `ws viewer` | query filters | `MongoSnippet[]` |
| `POST /workspaces/{workspace_id}/db-mongo/snippets` | `ws editor` | snippet upsert | `MongoSnippet` |
| `PATCH /workspaces/{workspace_id}/db-mongo/snippets/{snippet_id}` | `ws editor` | snippet upsert | `MongoSnippet` |
| `DELETE /workspaces/{workspace_id}/db-mongo/snippets/{snippet_id}` | `ws editor` | none | `204` |

## Frontend Architecture

### Module Layout

Add focused modules under the existing DB module:

| File or directory | Responsibility |
| --- | --- |
| `ui/src/modules/database/transfer/TransferWizard.svelte` | Operation/source/target/preview/map/dry-run/run shell |
| `ui/src/modules/database/transfer/TransferSourceStep.svelte` | Source selection |
| `ui/src/modules/database/transfer/TransferTargetStep.svelte` | Target selection |
| `ui/src/modules/database/transfer/TransferPreviewStep.svelte` | File/query/object preview |
| `ui/src/modules/database/transfer/TransferMappingStep.svelte` | Field mapping grid |
| `ui/src/modules/database/transfer/TransferRunPanel.svelte` | Progress, events, result summary |
| `ui/src/modules/database/transfer/TransferProfiles.svelte` | Saved profiles and run history |
| `ui/src/modules/database/mongo/MongoTools.svelte` | Mongo tools tab shell |
| `ui/src/modules/database/mongo/PipelineBuilder.svelte` | Stage card editor |
| `ui/src/modules/database/mongo/MongoExplainPanel.svelte` | Visual explain summary and raw JSON |
| `ui/src/modules/database/mongo/MongoSchemaAnalyzer.svelte` | Sampled schema and index coverage |
| `ui/src/modules/database/mongo/MongoSnippets.svelte` | Snippet list and save/open controls |
| `ui/src/lib/stores/database-transfer.svelte.ts` | Transfer store if current DB store becomes too large |
| `ui/src/lib/stores/mongo-tools.svelte.ts` | Mongo tools state if needed |

Keep common app-wide widgets in `ui/src/lib/components/` only when they are
clearly reusable outside Database Explorer.

### UX Details

Transfer:

- Toolbar entries should be icon+text where space allows:
  - `Export`
  - `Import`
  - `Transfer`
- Schema tree context menu:
  - `Export data`
  - `Import into this table/collection`
  - `Copy to...`
- Results grid toolbar:
  - `Export result`
  - `Transfer result`
- Query tab:
  - `Export result`
  - `Use as transfer source`
- Run history:
  - status chip, duration, source, target, row counts, last error.

Mongo:

- Show Mongo Tools only for MongoDB connections.
- Pipeline builder should be split-pane:
  - left: stage list.
  - right: generated command, preview, explain.
- Stage cards should not resize unpredictably; use fixed action button sizes and
  constrained editors.
- Explain should be readable first, raw JSON second.
- No visible instructional text that explains basic UI mechanics; use concise
  labels and tooltips.

### Agent Handoff

Add structured payload builders for:

- Transfer dry-run report.
- Transfer run summary.
- Transfer failed-row sample.
- Mongo explain summary.
- Mongo schema analysis.
- Pipeline/snippet body.

The handoff should include enough context for an agent to reason without
re-querying by default:

- Connection display name, engine, database/schema/object names.
- Query or pipeline text.
- Mapping.
- Counts and errors.
- Sample rows/documents capped and masked.
- Relevant schema/index metadata.

## Safety and Security

### RBAC

Use existing workspace permission semantics:

- Preview, schema analysis, explain, history read: `ws viewer`.
- Import, copy, saved profile mutation, snippet mutation: `ws editor`.
- Export-to-path remains a data egress operation and should keep existing export
  permission behavior; if current export is `ws viewer`, document that clearly
  and consider an admin setting later.

### Write Guards

All write-capable transfer runs must:

- Reuse existing `confirm_write` semantics where possible.
- Refuse to run against read-only connections.
- Surface production/environment labels from connection metadata.
- Show dry-run impact before enabling run.
- Require explicit confirmation text for high-impact options:
  - create target object.
  - truncate target.
  - replace/upsert many rows.
  - Redis writes if/when added.

### Data Exposure

Transfer run history must avoid storing full imported data. Persist:

- Metadata.
- Counters.
- Mapping.
- Error messages.
- Small capped reject samples with masking applied where the request asks for
  masking.

Do not store:

- Secrets.
- Full files.
- Full transferred row/document payloads.

### Local Paths

Any endpoint accepting a local path must:

- Treat it as daemon-host local.
- Reject directory traversal only when resolving relative paths against an
  allowed base. Absolute paths are expected in desktop usage.
- Never expand shell syntax.
- Never execute user-provided paths.
- Return clear errors for unreadable files.

## Engine Behavior

### MySQL

Phase 1:

- Export: existing streaming export.
- Import: append into existing table with batched parameterized insert.
- Create target table: generated DDL preview, optional in Phase 2.
- Types: infer common scalar types from file samples; allow user override.
- Validation: count rows read/written; sample failed row errors.

Later:

- `LOAD DATA LOCAL INFILE` behind an explicit performance mode.
- Replace/upsert modes.
- Foreign-key disable is not in initial scope.
- SQL dump import/export.

### MongoDB

Phase 1:

- Export: existing streaming cursor export.
- Import: insertMany into existing or implicit collection.
- Extended JSON decode for `$oid`, `$date`, numeric wrappers, UUID where current
  helpers support it.
- Duplicate key behavior: fail batch, skip duplicates, or unordered insert with
  failed-row reporting.
- Pipeline builder and visual explain.

Later:

- Upsert by selected key.
- Schema drift snapshots.
- Profiler/currentOp.
- GridFS.

### ClickHouse

Phase 1:

- Export: existing streaming export.
- Import: append-only into existing table.
- Prefer native bulk format support when it fits the current driver shape.
- Avoid auto-create table until the UI can collect engine/order key choices.

Later:

- Create table flow.
- Partition-aware validation.
- Insert settings profile.

### Redis

Phase 1:

- Export key/value samples only.
- No Redis writes in the first import/copy release.

Later:

- Import key/value NDJSON with TTL/type metadata.
- Strong confirmation and namespace prefix guard.
- Dry-run collision report.

## Observability

Record transfer events in a structured form:

- parse_started
- parse_failed
- preview_ready
- dry_run_started
- dry_run_failed
- run_started
- batch_written
- batch_failed
- cancel_requested
- run_cancelled
- run_failed
- run_succeeded

Log with connection IDs and run IDs, but not secrets or full row values.

## Testing Strategy

Backend:

- Unit tests for file format detection.
- Unit tests for CSV/TSV parser options.
- Unit tests for JSON array and NDJSON parser errors.
- Unit tests for type inference.
- Unit tests for mapping validation.
- Unit tests for Mongo pipeline command generation.
- Unit tests for Mongo explain summary extraction.
- State tests for transfer profiles/runs/events CRUD.
- Driver-level tests for write adapters where existing testcontainers are
  available.

Frontend:

- Svelte type check through `npm run check`.
- Component-level tests if this repo has an established Svelte component test
  runner; otherwise Playwright coverage.
- E2E paths:
  - open DB page, start export wizard from result grid.
  - import preview from sample file path.
  - mapping validation errors.
  - transfer run progress with mocked daemon.
  - Mongo pipeline builder generates command.
  - Mongo explain panel renders summary and raw JSON.

Manual verification:

- MySQL CSV import into a scratch database.
- Mongo NDJSON import into scratch collection.
- ClickHouse NDJSON import into scratch table if a local test instance exists.
- Cancel a long-running transfer and confirm persisted cancelled status.

## Acceptance Criteria

### Data Transfer MVP

- A user can start export/import from Database Explorer without leaving Otto.
- A user can preview a local CSV/TSV/JSON/NDJSON file before writing anything.
- A user can map fields to a MySQL table or Mongo collection.
- A dry run catches incompatible mappings and shows readable errors.
- A confirmed import writes batches and reports progress.
- A cancelled run stops before the next batch and persists `cancelled`.
- Run history shows status, source, target, counts, duration, and error details.
- Profiles can be saved, listed, rerun, renamed, and deleted.
- Contracts and TypeScript types match Rust API types.
- Existing query/export behavior is not regressed.

### Mongo Power Tools MVP

- Mongo connections show a Mongo Tools surface.
- Users can build an aggregation pipeline from stage cards and raw JSON.
- Generated Mongo command can be run in the normal query engine.
- Stage preview respects the current row limit.
- Visual Explain shows index usage, collection scan warnings, docs examined,
  docs returned, duration, and raw JSON.
- Schema Analyzer shows sampled field names, observed types, example values,
  null/missing rates, and index coverage.
- Snippets can be saved and reopened.
- SQL-to-Mongo translation remains visible and understandable when used.

## Risks and Decisions

| Risk | Decision |
| --- | --- |
| Large files can overwhelm memory | Stream from daemon-local paths; preview only capped samples. |
| Browser file upload is awkward for large imports | Use daemon path first; add desktop picker for UX. |
| Cross-engine type conversion can be lossy | Ship explicit mapping and warnings; avoid silent conversion. |
| Long-running jobs need visibility | Persist runs/events and poll; WebSocket can come later. |
| Redis writes are high risk | Defer Redis import/copy writes until a stronger guard exists. |
| Existing `Driver` trait could become too broad | Keep transfer writer adapters separate from browsing/query driver. |
| Mongo pipeline builder can become a second query engine | Generate normal Mongo commands and run through the existing runner. |
| Run history may leak data | Persist metadata, counters, mapping, and capped masked samples only. |

## Open Implementation Notes

These are not questions for product approval; they are implementation checks for
the engineer doing the work:

- Confirm whether the Tauri shell already exposes file picker commands. If not,
  implement manual path input first and add a desktop picker in a later patch.
- Confirm current state store initialization before choosing whether transfer
  repositories live in `db_explorer.rs` or a new state module.
- Confirm existing RBAC helper names in `otto-server` route wiring before adding
  endpoints.
- Confirm whether existing export path permission is viewer or editor and
  document the behavior exactly in `docs/contracts/api.md`.
