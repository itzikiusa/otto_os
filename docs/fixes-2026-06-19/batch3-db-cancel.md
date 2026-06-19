# Batch-3 — Server-side DB query cancellation (audit Database §5 must-have)

## Problem

DB-Explorer query cancellation was **client-only**: `abortQuery`
(`ui/src/lib/stores/database.svelte.ts`) merely aborted the `fetch`. The database
kept executing the heavy query, and the driver's cached connection stayed pinned
running it — so "Stop" freed the UI but not the server.

This adds **server-side, engine-native cancellation** and wires the UI to it.

## Query-id / in-flight registry design

- The **client generates a per-run `query_id`** (UUID) and sends it on
  `POST …/db/query`. The *same* id is later sent to `…/db/cancel`. A per-run id
  (not the tab id) is the right key: a stale tab id could otherwise cancel a
  newer run.
- `QueryRequest` gained an optional `query_id: Option<String>`
  (`#[serde(default)]` — old clients still deserialize, just without tracking).
- `DbViewerService` keeps an in-flight map `query_id → InFlightQuery { conn_id,
  token }` (a plain `std::sync::Mutex`, held only for the brief insert / lookup /
  remove — never across an await).
- Registration is **RAII** via `InFlightGuard`: inserted at the top of `run`,
  removed on `Drop` — so the entry is cleaned up on success, error, **or**
  future-cancellation (the UI dropping the request). No stale entry a later
  cancel could wrongly target.
- A `CancelToken` (`Arc<Mutex<Option<QueryHandle>>>`, cheaply cloneable) is the
  shared slot: the service hands one clone to the driver and keeps the other in
  the registry, so a concurrent cancel reads the engine-native handle the moment
  the driver captures it.

`QueryHandle` is the engine-native reference:
`MysqlConnId(u64)` | `ClickhouseQueryId(String)`.

## Driver trait additions (`driver.rs`)

- `run_tracked(cfg, req, &CancelToken)` — default delegates to `run` (ignores the
  token). Engines with a native cancel override it to capture their handle.
- `cancel(cfg, &QueryHandle)` — default **no-op success**. Engines override to
  issue the KILL. Cancelling an unknown/finished query is always a no-op success,
  never an error/panic.

`DbViewerService::run` now calls `run_tracked`; the bare `run` on each driver
delegates to `run_tracked` with a throwaway token (so the plain trait method is
unchanged for non-cancel callers like `run_widget`).

## Per-engine cancellation

- **MySQL** (`drivers/mysql.rs`): after acquiring the single session connection
  in `run_read`/`run_write`, capture `CONNECTION_ID()` into the token
  (`capture_conn_id`). `cancel` runs `KILL QUERY <connid>` on a **separate**
  pooled connection — cancels only the current statement, leaving the pooled
  session reusable. An already-finished id yields "Unknown thread id", swallowed
  as a successful no-op.
- **ClickHouse** (`drivers/clickhouse.rs`): generate a fresh `query_id`
  (`otto-<ULID>`), tag the HTTP request with the `query_id` request param, and
  record it in the token (HTTP transport only). `cancel` runs `KILL QUERY WHERE
  query_id = '<id>'` over a separate HTTP request. The **native** transport
  doesn't expose a per-query id here, so a native-transport run isn't
  server-cancellable → cancel is a documented no-op there (most connections use
  HTTP).
- **MongoDB** & **Redis**: use the trait-default **no-op** cancel (per the task —
  no reliable per-op handle through the high-level drivers; Redis console
  commands here aren't long-blocking). Robust and explicit.

## REST endpoint (`http.rs`)

`POST /connections/{id}/db/cancel` with body `{ "query_id": "<id>" }`. Gated at
the **same role as `run_query`** — `WorkspaceRole::Editor` (global connections:
root only) via the existing `check_conn_role` — issuing a `KILL` is privileged.
Returns `204 No Content`; unknown/finished/cross-connection ids are a no-op
success. The service additionally requires the registry entry's `conn_id` to
match the route's connection, so a cancel can't reach across connections.

## UI wiring (`database.svelte.ts`)

- `runQuery` generates a `query_id` (`newQueryId()` → `crypto.randomUUID` with a
  non-crypto fallback) and sends it on the query body; it's stored alongside the
  `AbortController` + `connId` per tab.
- `abortQuery` now (1) fires `POST …/db/cancel` with the run's `query_id`
  (fire-and-forget, best-effort — failures don't block stopping the UI), then
  (2) aborts the fetch and clears the tab's running state. Starting a new run for
  a tab also calls `abortQuery` first, so a prior heavy query is cancelled
  server-side too.

## Docs

`docs/contracts/api.md` — added the `…/db/cancel` row and a paragraph describing
the optional `query_id` on `RunQueryReq`, the engine-native KILL behaviour, the
role gate, and the no-op cases.

## Tests (all unit, no Docker required)

- `types.rs`: `CancelToken` round-trip, shared-across-clones, last-set-wins;
  `query_id` serde default.
- `driver.rs`: default `run_tracked` delegates to `run` (ignoring the token);
  default `cancel` is a no-op success; an overriding `cancel` is dispatched with
  the captured handle (via `MinimalDriver` / `CancellingDriver` stubs).
- `service.rs`: `cancel_handle_for` decision logic — returns the captured handle
  on a match, `None` for unknown id / wrong connection / no captured handle; and
  `InFlightGuard` removes its entry on drop.

## Verification

- `cargo check -p otto-dbviewer` — clean.
- `cargo clippy -p otto-dbviewer --all-targets -- -D warnings` — clean (0).
- `cargo clippy --workspace --all-targets -- -D warnings` — clean (0), so
  downstream consumers (otto-server / ottod) still build.
- `cargo test -p otto-dbviewer --lib` — **87 passed**, 1 ignored (E2E suites
  remain env-gated/ignored — no Docker needed).
- `cd ui && npm run check` — **0 errors, 0 warnings** (480 files).

## Files changed

- `crates/otto-dbviewer/src/types.rs` — `QueryRequest.query_id`, `QueryHandle`,
  `CancelToken`.
- `crates/otto-dbviewer/src/driver.rs` — `run_tracked` + `cancel` trait methods +
  tests.
- `crates/otto-dbviewer/src/service.rs` — in-flight registry, `InFlightGuard`,
  `cancel`, `cancel_handle_for` + tests.
- `crates/otto-dbviewer/src/drivers/mysql.rs` — `run_tracked`, `cancel`,
  `capture_conn_id`.
- `crates/otto-dbviewer/src/drivers/clickhouse.rs` — `run_tracked`, `cancel`,
  `query_id` plumbing, `new_query_id`.
- `crates/otto-dbviewer/src/http.rs` — `…/db/cancel` route + handler.
- `ui/src/lib/stores/database.svelte.ts` — `query_id` on run, server cancel in
  `abortQuery`, `newQueryId`.
- `docs/contracts/api.md` — cancel route + semantics.
