# Database Transfer and Mongo Power Tools Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an Otto-native Workbench-style data transfer workflow and NoSQLBooster-style MongoDB power tools to Database Explorer, with guarded writes, previews, dry runs, persisted transfer history, saved profiles, Mongo pipeline building, visual explain, schema analysis, snippets, and agent handoff.

**Architecture:** Extend the existing Database Explorer rather than creating a separate product area. Add transfer and Mongo-tools modules to `otto-dbviewer`, append state migrations in `otto-state`, wire new Axum routes through the existing DB route layer, mirror contracts in `docs/contracts/api.md` and `ui/src/lib/api/types.ts`, and add Svelte feature surfaces under `ui/src/modules/database/`. Keep transfer writers separate from the existing query/browse `Driver` trait.

**Tech Stack:** Rust workspace (`otto-dbviewer`, `otto-state`, `otto-server`/route wiring as needed), SQLite migrations, Axum HTTP, Svelte 5 + TypeScript + Vite UI, existing Database Explorer stores/components, existing SSH/connection/keychain infrastructure, existing DB driver crates for MySQL/MongoDB/ClickHouse/Redis.

---

## Source Documents

- Design: `docs/db/features/data-transfer-and-mongo-power-tools-design.md`
- Current Database Explorer feature doc: `docs/features/database-explorer.md`
- API contract: `docs/contracts/api.md`
- Existing UI API types: `ui/src/lib/api/types.ts`
- Existing DB store: `ui/src/lib/stores/database.svelte.ts`
- Existing DB HTTP routes: `crates/otto-dbviewer/src/http.rs`
- Existing DB service/types/export: `crates/otto-dbviewer/src/{service.rs,types.rs,export.rs}`
- Existing state DB module: `crates/otto-state/src/db_explorer.rs`

## Implementation Rules

- Do not weaken existing production/read-only/write-confirmation guards.
- Do not edit or renumber existing migrations. Add new numbered migrations only.
- Do not store full imported files or full transferred row/document payloads in
  SQLite.
- Do not put secrets in logs, state, events, or agent handoff payloads.
- Keep API contract, Rust types, and TypeScript types in lockstep.
- Keep tasks small enough that each can be reviewed independently.
- After each milestone, run the listed verification commands and fix failures
  before moving to the next milestone.

## Milestone 0: Baseline Audit

- [ ] Read the design doc completely:
  - `docs/db/features/data-transfer-and-mongo-power-tools-design.md`
- [ ] Re-read the current Database Explorer docs and contracts:
  - `docs/features/database-explorer.md`
  - `docs/contracts/api.md`, Database Explorer section
- [ ] Inspect current DB modules:
  - `crates/otto-dbviewer/src/http.rs`
  - `crates/otto-dbviewer/src/service.rs`
  - `crates/otto-dbviewer/src/types.rs`
  - `crates/otto-dbviewer/src/export.rs`
  - `crates/otto-dbviewer/src/drivers/mongodb.rs`
  - `crates/otto-dbviewer/src/drivers/mongo_parse.rs`
  - `crates/otto-dbviewer/src/drivers/mongo_sql.rs`
  - `crates/otto-state/src/db_explorer.rs`
  - `ui/src/modules/database/DatabasePage.svelte`
  - `ui/src/lib/stores/database.svelte.ts`
  - `ui/src/lib/api/types.ts`
- [ ] Identify the next migration number:
  - `ls crates/otto-state/migrations`
- [ ] Confirm route wiring location for DB routes in `otto-server` or the crate
  that mounts `otto-dbviewer::http`.
- [ ] Confirm whether desktop file picker commands already exist:
  - `rg -n "pick.*file|open.*dialog|save.*dialog|tauri.*dialog|dialog" apps ui crates`
- [ ] Decide whether transfer store state stays in
  `ui/src/lib/stores/database.svelte.ts` or a new
  `ui/src/lib/stores/database-transfer.svelte.ts`.
- [ ] Suggested commit after audit, if any docs-only notes are added:
  - `docs(db): document transfer implementation audit`

Verification:

```bash
cargo check -p otto-dbviewer
cd ui && npm run check
```

Expected result: no new failures introduced by the audit.

## Milestone 1: Contracts and Shared Types

### 1.1 Add Rust API Types

- [ ] In `crates/otto-dbviewer/src/types.rs`, add serializable public types:
  - `TransferKind`
  - `TransferFormat`
  - `TransferEndpoint`
  - `TransferFieldMapping`
  - `TransferConflictMode`
  - `TransferCreateTargetMode`
  - `TransferOptions`
  - `TransferPreviewRequest`
  - `TransferPreviewResponse`
  - `TransferDryRunRequest`
  - `TransferDryRunResponse`
  - `TransferRunRequest`
  - `TransferRunResponse`
  - `TransferRunSummary`
  - `TransferRunCounters`
  - `TransferRunStatus`
  - `TransferRunEvent`
  - `TransferProfile`
  - `TransferProfileUpsert`
  - `MongoPipelineStage`
  - `MongoPipelinePreviewRequest`
  - `MongoPipelinePreviewResponse`
  - `MongoSqlTranslateRequest`
  - `MongoSqlTranslateResponse`
  - `MongoExplainSummary`
  - `MongoSchemaAnalysisRequest`
  - `MongoSchemaAnalysisResponse`
  - `MongoSnippet`
  - `MongoSnippetUpsert`
- [ ] Use `serde(rename_all = "snake_case")` for enum JSON stability, unless the
  existing DB API uses another convention nearby.
- [ ] Keep `connection_id`, `workspace_id`, `database`, `schema`, and `object`
  fields explicit. Do not overload one string field for multiple concepts.
- [ ] Add validation helper methods only for pure validation. Do not do I/O in
  `types.rs`.

### 1.2 Mirror TypeScript Types

- [ ] In `ui/src/lib/api/types.ts`, add matching TypeScript types in the Database
  Explorer section.
- [ ] Keep field names exactly aligned with Rust JSON.
- [ ] Export all types that stores/components will consume.
- [ ] Add narrow union types for enum values.

### 1.3 Update API Contract

- [ ] In `docs/contracts/api.md`, add a Database Transfer subsection under the
  Database Explorer API area.
- [ ] Document endpoints:
  - `POST /connections/{id}/db/import/preview`
  - `POST /connections/{id}/db/import/dry-run`
  - `POST /connections/{id}/db/import/run`
  - `POST /workspaces/{workspace_id}/db-transfer/preview`
  - `POST /workspaces/{workspace_id}/db-transfer/dry-run`
  - `POST /workspaces/{workspace_id}/db-transfer/runs`
  - `GET /workspaces/{workspace_id}/db-transfer/runs`
  - `GET /workspaces/{workspace_id}/db-transfer/runs/{run_id}`
  - `POST /workspaces/{workspace_id}/db-transfer/runs/{run_id}/cancel`
  - `GET /workspaces/{workspace_id}/db-transfer/profiles`
  - `POST /workspaces/{workspace_id}/db-transfer/profiles`
  - `PATCH /workspaces/{workspace_id}/db-transfer/profiles/{profile_id}`
  - `DELETE /workspaces/{workspace_id}/db-transfer/profiles/{profile_id}`
- [ ] Add a Mongo Tools subsection documenting:
  - `POST /connections/{id}/db/mongo/pipeline/preview`
  - `POST /connections/{id}/db/mongo/sql/translate`
  - `POST /connections/{id}/db/mongo/explain`
  - `POST /connections/{id}/db/mongo/schema-analysis`
  - `GET /workspaces/{workspace_id}/db-mongo/snippets`
  - `POST /workspaces/{workspace_id}/db-mongo/snippets`
  - `PATCH /workspaces/{workspace_id}/db-mongo/snippets/{snippet_id}`
  - `DELETE /workspaces/{workspace_id}/db-mongo/snippets/{snippet_id}`
- [ ] For every endpoint, document:
  - permission (`ws viewer` or `ws editor`)
  - request shape
  - response shape
  - destructive/write confirmation behavior
  - local path semantics
  - masking behavior

Verification:

```bash
cargo check -p otto-dbviewer
cd ui && npm run check
rg -n "TransferPreviewRequest|MongoExplainSummary" docs/contracts/api.md ui/src/lib/api/types.ts crates/otto-dbviewer/src/types.rs
```

Suggested commit:

```bash
git add docs/contracts/api.md ui/src/lib/api/types.ts crates/otto-dbviewer/src/types.rs
git commit -m "feat(db): add transfer and Mongo tool contracts"
```

## Milestone 2: Persistence for Profiles, Runs, Events, and Snippets

### 2.1 Add Migration

- [ ] Add a new migration under `crates/otto-state/migrations/` with the next
  numeric prefix.
- [ ] Create `db_transfer_profiles`.
- [ ] Create `db_transfer_runs`.
- [ ] Create `db_transfer_run_events`.
- [ ] Create `db_mongo_snippets`.
- [ ] Add indexes:
  - `db_transfer_profiles(workspace_id, updated_at)`
  - `db_transfer_runs(workspace_id, created_at)`
  - `db_transfer_runs(status, created_at)`
  - `db_transfer_run_events(run_id, ts)`
  - `db_mongo_snippets(workspace_id, updated_at)`
  - `db_mongo_snippets(connection_id, database_name, collection_name)`
- [ ] Store structured fields as JSON text consistently with existing state
  patterns.

### 2.2 Add State Repository Methods

- [ ] In `crates/otto-state/src/db_explorer.rs`, add profile methods:
  - `list_transfer_profiles(workspace_id, filters)`
  - `create_transfer_profile(profile)`
  - `update_transfer_profile(profile_id, patch)`
  - `delete_transfer_profile(profile_id)`
- [ ] Add run methods:
  - `create_transfer_run(run)`
  - `update_transfer_run_status(run_id, status, counters, error)`
  - `get_transfer_run(run_id)`
  - `list_transfer_runs(workspace_id, filters)`
  - `append_transfer_run_event(event)`
  - `list_transfer_run_events(run_id, limit)`
- [ ] Add Mongo snippet methods:
  - `list_mongo_snippets(workspace_id, filters)`
  - `create_mongo_snippet(snippet)`
  - `update_mongo_snippet(snippet_id, patch)`
  - `delete_mongo_snippet(snippet_id)`
- [ ] If `db_explorer.rs` becomes too large, extract to
  `crates/otto-state/src/db_transfer.rs` and re-export from the state crate
  module root.

### 2.3 Add State Tests

- [ ] Add tests covering:
  - profile create/list/update/delete
  - run create/status update/event append/list
  - snippets create/list/update/delete
  - workspace scoping
  - JSON round trip for endpoints/mappings/options
- [ ] Follow existing `otto-state` test setup patterns.

Verification:

```bash
cargo test -p otto-state db_transfer
cargo test -p otto-state db_explorer
```

Suggested commit:

```bash
git add crates/otto-state
git commit -m "feat(db): persist transfer runs and Mongo snippets"
```

## Milestone 3: Transfer Preview and Mapping Engine

### 3.1 Add Transfer Module Skeleton

- [ ] Add `crates/otto-dbviewer/src/transfer/mod.rs`.
- [ ] Add `crates/otto-dbviewer/src/transfer/preview.rs`.
- [ ] Add `crates/otto-dbviewer/src/transfer/mapping.rs`.
- [ ] Re-export the module from `crates/otto-dbviewer/src/lib.rs`.
- [ ] Keep module APIs internal until HTTP/service wiring needs them.

### 3.2 Implement File Format Detection

- [ ] Implement detection for:
  - CSV
  - TSV
  - JSON array
  - NDJSON
- [ ] Accept explicit format override from `TransferPreviewRequest`.
- [ ] Return a clear error for unsupported formats.
- [ ] Cap preview by:
  - rows/documents
  - bytes read
  - field count

### 3.3 Implement Parsers

- [ ] CSV/TSV:
  - header row on/off
  - delimiter
  - quote character
  - null token
  - trim whitespace option
- [ ] JSON array:
  - require top-level array for `json_array`
  - preview first N objects
  - reject non-object rows for object-target imports unless the user maps a
    value field explicitly
- [ ] NDJSON:
  - one JSON document per line
  - collect first N parse errors with line number
- [ ] Ensure all parsers stream or cap reads; do not load large files fully.

### 3.4 Implement Type Inference

- [ ] Infer field names from sample rows.
- [ ] Infer observed source types:
  - string
  - integer
  - float
  - decimal-like string
  - boolean
  - null
  - object
  - array
  - date-like string
  - ObjectId-like string
- [ ] Suggest target types per engine:
  - MySQL common scalar types
  - Mongo BSON-friendly field shapes
  - ClickHouse common scalar/nullable types
- [ ] Mark lossy mappings explicitly.

### 3.5 Implement Mapping Validation

- [ ] Validate duplicate target fields.
- [ ] Validate required target fields without source/default.
- [ ] Validate skipped fields.
- [ ] Validate target type compatibility.
- [ ] Validate create-target mode vs existing-target mode.
- [ ] Return structured validation errors with field path, code, and message.

### 3.6 Add Unit Tests

- [ ] Add tests for:
  - CSV with header
  - CSV without header
  - TSV
  - JSON array
  - NDJSON
  - invalid JSON line reporting
  - field type inference
  - duplicate target mapping
  - required target field missing
  - preview caps

Verification:

```bash
cargo test -p otto-dbviewer transfer::preview
cargo test -p otto-dbviewer transfer::mapping
```

Suggested commit:

```bash
git add crates/otto-dbviewer/src/transfer crates/otto-dbviewer/src/lib.rs
git commit -m "feat(db): add transfer preview and mapping engine"
```

## Milestone 4: Transfer Run Coordinator and Writers

### 4.1 Add Run Coordinator

- [ ] Add `crates/otto-dbviewer/src/transfer/run.rs`.
- [ ] Implement a `TransferRunManager` or similarly named coordinator.
- [ ] Responsibilities:
  - create persisted run
  - append events
  - stream input rows/documents
  - batch writes
  - update counters
  - observe cancellation flag before each batch
  - mark succeeded/failed/cancelled
- [ ] Do not hold UI request futures open for the whole run unless the existing
  app pattern requires it. Prefer creating a run and polling status.

### 4.2 Add Cancellation State

- [ ] Add in-memory cancellation registry keyed by `run_id`.
- [ ] Persist cancellation request as an event.
- [ ] If daemon restarts, incomplete `running` runs should be recovered as
  `failed` or `cancelled` on startup with a clear event. Choose the existing
  repo pattern for abandoned work.

### 4.3 Add Writer Adapters

- [ ] Add `crates/otto-dbviewer/src/transfer/writers.rs`.
- [ ] Define internal writer behavior:
  - inspect target
  - dry run
  - write batch
- [ ] Before any dry run or write that targets a database, inspect target
  connection metadata and return a structured refusal for read-only
  connections.
- [ ] Include source/target environment labels, production flags, and read-only
  flags in dry-run and run summaries so the UI can surface them before the user
  confirms a write.
- [ ] MongoDB writer:
  - use existing connection resolution/tunnel behavior
  - convert JSON/EJSON to BSON using existing helpers where possible
  - implement insert-only into existing collection or an implicit new collection
    when the target collection does not exist and create-target mode allows it
  - support ordered/unordered insert option
  - report duplicate key errors as structured failed rows where available
- [ ] MySQL writer:
  - use parameterized batched `INSERT`
  - quote identifiers using the existing SQL helper style; do not interpolate
    unescaped identifiers
  - append-only first
  - respect current database/schema context
- [ ] ClickHouse writer:
  - append-only into existing table
  - use the current driver/client capability that best supports batches
  - document unsupported options clearly in dry-run errors
- [ ] Redis:
  - return unsupported for import/copy writes in this milestone.

### 4.4 Implement Dry Run

- [ ] For existing target:
  - reject read-only target connections before parsing large inputs
  - inspect target fields/types
  - validate mapping
  - parse capped input
  - return estimated row count where cheap
  - return sample conversion errors
  - return source/target environment labels and production/read-only flags
- [ ] For new target:
  - reject read-only target connections
  - allow MongoDB implicit collection creation when the target collection does
    not exist and create-target mode explicitly allows it
  - produce DDL preview for MySQL only if implemented
  - otherwise return `unsupported_create_target` with a clear message
- [ ] Dry run must not write.

### 4.5 Add Backend Tests

- [ ] Unit-test coordinator status transitions.
- [ ] Unit-test cancellation before first batch.
- [ ] Unit-test cancellation between batches.
- [ ] Unit-test dry-run unsupported mode errors.
- [ ] Unit-test dry-run refuses read-only target connections.
- [ ] Unit-test run refuses read-only target connections.
- [ ] Unit-test dry-run/run summaries include environment labels and
  production/read-only flags.
- [ ] Add integration tests for Mongo/MySQL writers only if the repo already has
  testcontainer patterns that can run in CI.

Verification:

```bash
cargo test -p otto-dbviewer transfer::run
cargo test -p otto-dbviewer transfer::writers
cargo test -p otto-dbviewer
```

Suggested commit:

```bash
git add crates/otto-dbviewer/src/transfer
git commit -m "feat(db): run guarded transfer imports"
```

## Milestone 5: HTTP Routes for Transfer

### 5.1 Wire Import Routes

- [ ] In `crates/otto-dbviewer/src/http.rs`, add handlers:
  - `import_preview`
  - `import_dry_run`
  - `import_run`
- [ ] Enforce permissions:
  - preview: viewer
  - dry run: editor
  - run: editor
- [ ] Validate path semantics and return safe errors.
- [ ] Refuse import dry-run/run for read-only target connections.
- [ ] Include source/target environment labels and production/read-only flags in
  dry-run/run responses.
- [ ] Respect existing masking/write confirmation conventions.

### 5.2 Wire Workspace Transfer Routes

- [ ] Add handlers for:
  - transfer preview
  - transfer dry run
  - create run
  - list runs
  - get run
  - cancel run
  - list profiles
  - create profile
  - update profile
  - delete profile
- [ ] If these routes are mounted outside `otto-dbviewer::http`, add the route
  wiring in the server crate that owns workspace routes.
- [ ] Keep route path names exactly matching `docs/contracts/api.md`.

### 5.3 Add API-Level Tests

- [ ] Add handler tests following existing route test patterns.
- [ ] Cover:
  - viewer can preview
  - viewer cannot dry run
  - viewer cannot run import
  - editor can start dry run/run
  - read-only target connection dry-run is refused
  - read-only target connection run is refused
  - dry-run/run response includes environment labels and production/read-only flags
  - missing file returns safe error
  - run listing is workspace-scoped
  - cancel missing run returns expected status

Verification:

```bash
cargo test -p otto-dbviewer http
cargo test --workspace db_transfer
```

Suggested commit:

```bash
git add crates/otto-dbviewer/src/http.rs crates/otto-server docs/contracts/api.md
git commit -m "feat(db): expose transfer API routes"
```

## Milestone 6: UI API Client and Transfer Store

### 6.1 Add API Methods

- [ ] Locate existing DB API helper methods used by
  `ui/src/lib/stores/database.svelte.ts`.
- [ ] Add client methods for all transfer endpoints.
- [ ] Keep request/response types imported from `ui/src/lib/api/types.ts`.
- [ ] Use existing error handling conventions.

### 6.2 Add Transfer Store

- [ ] Add `ui/src/lib/stores/database-transfer.svelte.ts` if transfer state would
  make the existing database store too large.
- [ ] Store state:
  - current operation
  - source endpoint
  - target endpoint
  - preview response
  - mapping
  - options
  - dry-run report
  - active run
  - run polling timer
  - profiles list
  - recent runs list
- [ ] Actions:
  - reset wizard
  - load preview
  - update mapping
  - run dry run
  - start run
  - poll run
  - cancel run
  - save profile
  - rename profile
  - update profile mapping/options
  - load profile
  - delete profile
- [ ] Ensure polling is stopped when component unmounts or active run completes.

### 6.3 Add Store Tests if Pattern Exists

- [ ] If the repo has store unit tests, add tests for:
  - preview success
  - preview failure
  - mapping update
  - run polling completion
  - cancel action
- [ ] If no store test pattern exists, cover these flows in Playwright later.

Verification:

```bash
cd ui && npm run check
rg -n "startTransferRun|TransferRunSummary|database-transfer" ui/src
```

Suggested commit:

```bash
git add ui/src/lib/api ui/src/lib/stores ui/src/lib/api/types.ts
git commit -m "feat(db): add transfer UI client state"
```

## Milestone 7: Transfer UI

### 7.1 Add Transfer Components

- [ ] Create `ui/src/modules/database/transfer/TransferWizard.svelte`.
- [ ] Create `TransferSourceStep.svelte`.
- [ ] Create `TransferTargetStep.svelte`.
- [ ] Create `TransferPreviewStep.svelte`.
- [ ] Create `TransferMappingStep.svelte`.
- [ ] Create `TransferRunPanel.svelte`.
- [ ] Create `TransferProfiles.svelte`.
- [ ] Follow existing Database Explorer visual density and component style.
- [ ] Avoid nested cards; use panels, tabs, tables, and constrained sections.

### 7.2 Add Entry Points

- [ ] In `DatabasePage.svelte`, add Transfer as a primary DB surface.
- [ ] In `SchemaTree.svelte`, add actions:
  - export data
  - import into this object
  - copy to... disabled behind a clear "planned" state until the later
    connection-to-connection copy milestone is implemented
- [ ] In `ResultsGrid.svelte`, add actions:
  - export result
  - transfer result to file/export; connection-to-connection copy remains
    disabled until the later connection-to-connection copy milestone
- [ ] In query tab UI, add action:
  - use result as transfer source
- [ ] Keep existing export behavior reachable.

### 7.3 Implement Wizard Steps

- [ ] Operation step:
  - export
  - import
  - copy shown as disabled/planned until the later connection-to-connection copy
    milestone, or hidden entirely if the product shell has no disabled-operation
    pattern
- [ ] Source step:
  - active table/collection
  - active query result
  - local file path
- [ ] Target step:
  - existing table/collection
  - MongoDB implicit new collection when create-target mode is enabled
  - SQL/ClickHouse new target disabled or clearly marked unsupported until DDL
    creation is implemented
  - export local path
- [ ] Preview step:
  - show sample rows/documents
  - show inferred fields/types
  - show parse errors
- [ ] Mapping step:
  - field grid
  - source field selector
  - target field input/select
  - target type select
  - nullable/default/skip controls
  - validation messages per row
- [ ] Dry-run step:
  - show source/target environment labels and production/read-only flags
  - block run action when dry-run reports a read-only target refusal
  - show estimated writes
  - show incompatible fields
  - show failed sample conversions
  - require confirmation before run
- [ ] Run step:
  - progress counters
  - event list
  - cancel button
  - final result summary
  - save profile action
  - send to agent action if the existing DB page exposes that helper

### 7.4 Accessibility and Responsive Behavior

- [ ] Keyboard navigable wizard controls.
- [ ] Fixed-size icon buttons with tooltips.
- [ ] No text overlap at mobile/tablet widths.
- [ ] Mapping grid scrolls horizontally rather than compressing controls into
  unreadable cells.
- [ ] Long paths/object names truncate with title tooltip.

Verification:

```bash
cd ui && npm run check
cd ui && npm run build
```

Manual verification:

- Open DB page.
- Start import from toolbar.
- Enter a sample local path.
- Preview.
- Change mapping.
- Dry run.
- Start a run against a scratch DB only.
- Cancel a run.
- Save and reload a profile.

Suggested commit:

```bash
git add ui/src/modules/database ui/src/lib/stores
git commit -m "feat(db): add transfer wizard"
```

## Milestone 8: Export Wizard Polish and Saved Profiles

### 8.1 Wrap Existing Export

- [ ] Reuse existing `POST /connections/{id}/db/export` and
  `POST /connections/{id}/db/export-to-path`.
- [ ] Add export wizard flow for:
  - current table/collection
  - current query result
  - selected format
  - target path
  - row limit/mask options
- [ ] Persist export profile using the shared transfer profile table.

### 8.2 Add Run History View

- [ ] In `TransferProfiles.svelte`, add tabs or segmented controls:
  - profiles
  - runs
- [ ] List run status, source, target, row counts, duration, error.
- [ ] Add actions:
  - view details
  - rerun profile
  - rename profile
  - edit profile mapping/options
  - copy summary
  - hand off to agent
  - delete profile with confirmation
- [ ] Keep destructive actions confirmed.

Verification:

```bash
cd ui && npm run check
cargo test -p otto-state db_transfer
```

Suggested commit:

```bash
git add ui/src/modules/database/transfer crates/otto-state
git commit -m "feat(db): save transfer profiles and history"
```

## Milestone 9: Mongo Tools Backend

### 9.1 Add Mongo Tools Module

- [ ] Add `crates/otto-dbviewer/src/mongo_tools.rs`.
- [ ] Re-export or wire from `lib.rs`.
- [ ] Keep the module focused on Mongo-specific shaping and validation.

### 9.2 Pipeline Preview

- [ ] Define helper to convert `MongoPipelineStage[]` into
  `db.<collection>.aggregate([...])`.
- [ ] Validate every stage:
  - exactly one top-level operator per stage unless raw mode is used
  - stage body is JSON object where Mongo expects an object
  - disabled stages are omitted
- [ ] Run through the existing Mongo query path with row limit.
- [ ] Return:
  - generated command
  - preview rows/documents
  - stage index
  - warnings

### 9.3 SQL-to-Mongo Assistant Backend

- [ ] Reuse the existing `mongo_sql` translator rather than creating a second
  translation engine.
- [ ] Implement `POST /connections/{id}/db/mongo/sql/translate` as a viewer
  endpoint that translates without executing.
- [ ] Return:
  - input SQL
  - generated Mongo command
  - unsupported construct errors
  - warnings about translation limits
- [ ] Keep generated commands runnable through the normal Mongo query runner.
- [ ] Add tests for:
  - supported `SELECT` translation
  - unsupported join/subquery/SQL construct message
  - generated command returned without executing when requested

### 9.4 Visual Explain Summary

- [ ] Use existing explain query path where possible.
- [ ] Extract fields from Mongo explain JSON:
  - execution time
  - docs examined
  - docs returned
  - keys examined
  - winning plan stage
  - index names
  - collection scan boolean
  - rejected plans count
  - raw explain JSON
- [ ] Return both machine-readable summary fields and raw JSON.
- [ ] Add defensive parsing for Mongo server version differences.

### 9.5 Schema Analysis

- [ ] Sample documents with a capped limit.
- [ ] Infer field paths recursively up to a sane depth.
- [ ] Record:
  - field path
  - observed types
  - count present
  - count null
  - example value
  - indexed boolean/name where known
- [ ] Use existing collection stats/index metadata if already available in
  `StructureView`/driver responses; otherwise add a targeted metadata call.

### 9.6 Wire Mongo Tool Routes

- [ ] In `crates/otto-dbviewer/src/http.rs`, add handlers for:
  - `mongo_pipeline_preview`
  - `mongo_sql_translate`
  - `mongo_explain`
  - `mongo_schema_analysis`
- [ ] Mount routes matching `docs/contracts/api.md` exactly:
  - `POST /connections/{id}/db/mongo/pipeline/preview`
  - `POST /connections/{id}/db/mongo/sql/translate`
  - `POST /connections/{id}/db/mongo/explain`
  - `POST /connections/{id}/db/mongo/schema-analysis`
- [ ] Enforce `ws viewer` on all four Mongo tool routes.
- [ ] Reject non-MongoDB connections with a structured unsupported-engine error.
- [ ] Ensure every route resolves the connection through existing connection
  resolution/tunnel paths.
- [ ] Keep `docs/contracts/api.md`, `crates/otto-dbviewer/src/types.rs`, and
  `ui/src/lib/api/types.ts` in lockstep for the request/response types.

### 9.7 Snippet Persistence Routes

- [ ] Reuse state methods from Milestone 2.
- [ ] Add HTTP handlers for snippet CRUD.
- [ ] Scope snippets by workspace and optionally connection/database/collection.

### 9.8 Backend Tests

- [ ] Unit-test pipeline command generation.
- [ ] Unit-test invalid stage validation.
- [ ] Unit-test disabled stage omission.
- [ ] Unit-test SQL-to-Mongo helper success and unsupported construct output.
- [ ] Unit-test explain summary extraction from fixture JSON.
- [ ] Unit-test schema analyzer on sample documents.
- [ ] Unit-test snippet CRUD route/state path.
- [ ] API-test viewer can call Mongo pipeline preview.
- [ ] API-test viewer can call SQL translate.
- [ ] API-test viewer can call Mongo explain.
- [ ] API-test viewer can call schema analysis.
- [ ] API-test non-MongoDB connections return unsupported-engine errors for
  Mongo tool routes.

Verification:

```bash
cargo test -p otto-dbviewer mongo_tools
cargo test -p otto-state db_mongo
cargo test -p otto-dbviewer
```

Suggested commit:

```bash
git add crates/otto-dbviewer/src/mongo_tools.rs crates/otto-dbviewer/src/http.rs crates/otto-state docs/contracts/api.md
git commit -m "feat(db): add Mongo power tool APIs"
```

## Milestone 10: Mongo Tools UI

### 10.1 Add Mongo UI API Client Methods

- [ ] Locate the existing DB API helper used by the Database Explorer store.
- [ ] Add client methods for:
  - `mongoPipelinePreview`
  - `mongoSqlTranslate`
  - `mongoExplain`
  - `mongoSchemaAnalysis`
  - `listMongoSnippets`
  - `createMongoSnippet`
  - `updateMongoSnippet`
  - `deleteMongoSnippet`
- [ ] Import request/response types from `ui/src/lib/api/types.ts`.
- [ ] Reuse existing API error handling and cancellation conventions.

### 10.2 Add Mongo UI Store

- [ ] Add `ui/src/lib/stores/mongo-tools.svelte.ts` if state does not fit cleanly
  in the main DB store.
- [ ] Store:
  - selected collection
  - pipeline stages
  - generated command
  - selected stage preview
  - explain summary
  - schema analysis
  - snippets
  - SQL-to-Mongo input
  - SQL-to-Mongo generated command
  - SQL-to-Mongo warnings/errors
- [ ] Actions:
  - add stage
  - update stage
  - reorder stage
  - duplicate stage
  - disable stage
  - delete stage
  - preview stage
  - run pipeline
  - explain pipeline
  - load schema analysis
  - save snippet
  - open snippet
  - translate SQL to Mongo
  - open generated Mongo command in query tab

### 10.3 Add Mongo Tools Components

- [ ] Create `ui/src/modules/database/mongo/MongoTools.svelte`.
- [ ] Create `PipelineBuilder.svelte`.
- [ ] Create `MongoStageCard.svelte` if stage cards need their own component.
- [ ] Create `MongoExplainPanel.svelte`.
- [ ] Create `MongoSchemaAnalyzer.svelte`.
- [ ] Create `SqlToMongoAssistant.svelte`.
- [ ] Create `MongoSnippets.svelte`.
- [ ] Mount Mongo Tools in `DatabasePage.svelte` only for MongoDB connections.

### 10.4 Pipeline Builder UX

- [ ] Provide stage add menu:
  - `$match`
  - `$project`
  - `$group`
  - `$sort`
  - `$limit`
  - `$skip`
  - `$unwind`
  - `$lookup`
  - `$addFields`
  - `$count`
  - raw
- [ ] Each stage card has:
  - enable toggle
  - operator label
  - code editor/textarea matching current DB UI patterns
  - duplicate
  - delete
  - move up/down or drag handle
  - preview action
- [ ] Generated command panel:
  - read-only command
  - copy button
  - open in query tab
  - run
  - explain
- [ ] Preview should reuse the normal result grid where practical.

### 10.5 Explain and Schema UX

- [ ] Explain panel shows:
  - collection scan warning
  - index used
  - docs returned
  - docs examined
  - keys examined
  - execution time
  - rejected plans
  - raw JSON toggle
- [ ] Schema analyzer shows:
  - field tree/table
  - observed types
  - present/null/missing rates
  - example value
  - index coverage
- [ ] Snippets panel:
  - list snippets
  - save current pipeline
  - save current query
  - open in builder/editor
  - delete with confirmation

### 10.6 SQL-to-Mongo Assistant UX

- [ ] Add a SQL-to-Mongo panel inside `MongoTools.svelte`.
- [ ] Let the user enter SQL against the active collection/database.
- [ ] Show:
  - translated Mongo command
  - copy action
  - open in query tab action
  - run action
  - unsupported construct errors in plain product language
  - current translation limits
- [ ] Ensure the assistant reuses existing query editor behavior where possible,
  including row limit and generated-command visibility.
- [ ] Make translated commands visible in query results when a user runs SQL
  through the normal Mongo query editor path.

Verification:

```bash
cd ui && npm run check
cd ui && npm run build
```

Manual verification:

- Select a Mongo connection.
- Open Mongo Tools.
- Build a pipeline with `$match`, `$group`, and `$sort`.
- Preview after `$match`.
- Run full pipeline.
- Open explain and confirm summary renders.
- Save pipeline as snippet and reopen it.
- Translate a supported SQL query and open the generated Mongo command.
- Try an unsupported SQL construct and confirm the message is actionable.

Suggested commit:

```bash
git add ui/src/modules/database/mongo ui/src/lib/stores/mongo-tools.svelte.ts ui/src/modules/database/DatabasePage.svelte
git commit -m "feat(db): add Mongo pipeline workspace"
```

## Later Milestone: Connection-to-Connection Copy

This milestone implements the copy operation that is intentionally disabled in
the MVP Transfer UI until this work lands.

### Copy 1: Add Source Readers

- [ ] Add transfer source readers for:
  - table/collection scan with filter and row limit
  - query result source
  - file source using the preview parser
- [ ] Readers must stream batches and reuse existing export/query paths where
  practical.
- [ ] Readers must preserve source column/field order where available.
- [ ] Readers must honor masking options when the target is a file or agent
  handoff, but must not mask data being copied into a database unless the user
  selected an explicit masking transform.

### Copy 2: Same-Engine Copy

- [ ] Implement MySQL -> MySQL append into existing table.
- [ ] Implement MongoDB -> MongoDB insert into existing/implicit collection.
- [ ] Implement ClickHouse -> ClickHouse append into existing table if the
  writer from Milestone 4 supports it.
- [ ] Return clear unsupported errors for Redis writes.
- [ ] Use the same dry-run, mapping validation, run history, progress, and cancel
  paths as file import.

### Copy 3: Cross-Engine Copy

- [ ] Add explicit mapping warnings for lossy conversion.
- [ ] Support only common scalar values initially:
  - string
  - integer
  - float
  - boolean
  - null
  - date-like string
- [ ] Reject nested Mongo documents into SQL unless the user maps them to JSON
  text or a JSON-capable target type.
- [ ] Reject SQL rows into Mongo only when field names cannot be represented
  safely.
- [ ] Document unsupported engine pairs in dry-run output.

### Copy 4: Enable Copy UI

- [ ] Enable `copy to...` in `SchemaTree.svelte`.
- [ ] Enable `transfer result` as connection-to-connection copy in
  `ResultsGrid.svelte`.
- [ ] Enable `copy` operation in `TransferWizard.svelte`.
- [ ] Add target connection/object picker.
- [ ] Add dry-run warnings for cross-engine copy.
- [ ] Add Playwright coverage for the disabled-to-enabled UI path.

Verification:

```bash
cargo test -p otto-dbviewer transfer::run
cargo test -p otto-dbviewer transfer::writers
cd ui && npm run check
```

Suggested commit:

```bash
git add crates/otto-dbviewer/src/transfer ui/src/modules/database/transfer docs/contracts/api.md ui/src/lib/api/types.ts
git commit -m "feat(db): copy data between connections"
```

## Milestone 11: Agent Handoff and Workflow Hooks

### 11.1 Transfer Handoff

- [ ] Locate existing agent handoff code for Database Explorer.
- [ ] Add payload builders for:
  - transfer dry-run report
  - transfer run summary
  - failed-row sample
  - saved transfer profile
- [ ] Payload must include:
  - engine
  - connection display name
  - database/schema/object
  - operation
  - mapping
  - options
  - counters
  - errors
  - capped masked sample
- [ ] Add UI action in transfer run details:
  - `Send to agent`

### 11.2 Mongo Handoff

- [ ] Add payload builders for:
  - pipeline
  - explain summary
  - schema analysis
  - snippet
- [ ] Add UI actions:
  - send pipeline to agent
  - send explain to agent
  - send schema analysis to agent

### 11.3 Optional Workflow Integration

- [ ] If workflow APIs already expose DB query actions, add a follow-up task
  document for scheduled transfer profiles rather than implementing scheduling
  in this milestone.
- [ ] Do not add scheduling until transfer runs are stable and auditable.

Verification:

```bash
cd ui && npm run check
cargo test --workspace db
```

Suggested commit:

```bash
git add ui/src/modules/database ui/src/lib/stores crates/otto-dbviewer
git commit -m "feat(db): hand off transfer and Mongo evidence to agents"
```

## Milestone 12: Documentation and E2E Coverage

### 12.1 Update Feature Docs

- [ ] Update `docs/features/database-explorer.md` with:
  - Transfer overview
  - Supported formats
  - Local path semantics
  - Safety guard behavior
  - Transfer run history
  - Mongo Tools overview
  - Mongo Pipeline Builder
  - Visual Explain
  - Schema Analyzer
  - Snippets
  - Limitations and troubleshooting
- [ ] Update `docs/features/README.md` Database Explorer row if needed.
- [ ] Ensure `docs/contracts/api.md` is still accurate after implementation.

### 12.2 Add E2E Tests

- [ ] Inspect existing DB E2E specs:
  - `rg -n "database|db" ui/e2e`
- [ ] Add Playwright coverage for:
  - Transfer wizard opens.
  - File preview happy path with mocked route or test fixture.
  - Mapping validation error.
  - Dry run report.
  - Run history row.
  - Mongo Tools visible only for Mongo connection.
  - Pipeline stage add/reorder/delete.
  - Generated command text.
  - Explain panel summary.
- [ ] Use isolated daemon/test data patterns from existing E2E specs.

### 12.3 Final Verification

- [ ] Run Rust formatting/checks:

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

- [ ] Run UI checks:

```bash
cd ui
npm run check
npm run build
npm run test:e2e
```

- [ ] If any command fails because of pre-existing unrelated failures, capture
  the exact failure and confirm the new feature-specific tests pass.

Suggested commit:

```bash
git add docs/features docs/contracts ui/e2e
git commit -m "docs(db): document transfer and Mongo tools"
```

## Release Slices

Slice 1 should be shippable independently:

- Contracts/types.
- State persistence.
- Import preview.
- Dry run.
- Mongo insert import.
- Transfer wizard preview/map/dry-run/run.
- Run history.

Slice 2:

- MySQL import.
- Export wizard polish.
- Saved profiles.
- Agent handoff.

Slice 3:

- Mongo pipeline builder.
- Visual explain.
- Schema analyzer.
- Snippets.

Slice 4:

- Connection-to-connection copy.
- ClickHouse import.
- Cross-engine mapping.
- Workflow scheduling follow-up.

## Definition of Done

- API contract, Rust types, and TypeScript types match.
- Transfer preview works for CSV, TSV, JSON array, and NDJSON.
- At least MongoDB and MySQL imports are implemented or unsupported engines show
  clear dry-run errors.
- Transfer writes require editor permission and explicit confirmation.
- Runs persist status, counters, events, and errors.
- Profiles persist, rerun, rename, update, and delete.
- Mongo Tools can build, preview, run, explain, save, and reopen a pipeline.
- SQL-to-Mongo translation remains visible, copyable, runnable, and clear about
  unsupported constructs.
- Mongo Schema Analyzer produces sampled field/type/index information.
- Connection-to-connection copy is either implemented by the later copy
  milestone or remains disabled/hidden in the UI with no active copy affordance.
- Feature docs and troubleshooting are updated.
- Rust and UI verification commands pass, or any unrelated pre-existing failures
  are documented with evidence.

## Acceptance Mapping

| Acceptance criterion | Implemented by |
| --- | --- |
| API contract, Rust types, and TypeScript types match | Milestone 1 |
| Transfer preview supports CSV, TSV, JSON array, and NDJSON | Milestone 3 |
| Mapping validation catches incompatible fields | Milestones 3 and 7 |
| Dry run shows estimated impact before writes | Milestones 4, 5, and 7 |
| Transfer writes require editor permission and explicit confirmation | Milestones 4, 5, and 7 |
| Runs persist status, counters, events, and errors | Milestones 2, 4, 5, and 8 |
| Runs can be cancelled | Milestones 4, 5, and 7 |
| Profiles can be saved, listed, rerun, renamed, edited, and deleted | Milestones 2, 6, and 8 |
| Export remains available and gains wizard/profile polish | Milestone 8 |
| Connection-to-connection copy works | Later Milestone: Connection-to-Connection Copy |
| Copy is not exposed as active before backend support exists | Milestone 7 |
| Mongo Tools are visible only for MongoDB connections | Milestone 10 |
| Pipeline builder can build, preview, run, explain, save, and reopen pipelines | Milestones 9 and 10 |
| Visual Explain summarizes index use and execution stats with raw JSON available | Milestones 9 and 10 |
| Schema Analyzer shows sampled fields, types, examples, null/missing rates, and index coverage | Milestones 9 and 10 |
| SQL-to-Mongo translation is visible and understandable | Milestones 9 and 10 |
| Snippets can be saved and reopened | Milestones 2, 9, and 10 |
| Agent handoff includes transfer and Mongo evidence | Milestone 11 |
| Feature docs and E2E tests cover the shipped surface | Milestone 12 |

## Known Follow-up Documents

Create separate plans before starting these larger extensions:

- `docs/db/features/db-transfer-scheduling-plan.md`
- `docs/db/features/db-transfer-cross-engine-mapping-plan.md`
- `docs/db/features/mongo-profiler-and-index-advisor-plan.md`
- `docs/db/features/redis-import-export-plan.md`
