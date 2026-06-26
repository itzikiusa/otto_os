# Mission Control / Work Graph — Implementation Plan

Ordered, independently-verifiable tasks. Each maps to requirements in
[workgraph-requirements.md](./workgraph-requirements.md) and the
[design](./workgraph-mission-control-design.md). Verifications are concrete gates.

> Convention: Rust repo idioms — `otto_core::{new_id, Error, Id, Result}`,
> `crate::convert::{dberr, fmt, ts}`, `sqlx::migrate!()`. Match surrounding code.

## Stage A — Foundations (backend types & storage)

### Task A1 — Register the `otto-workgraph` crate
- Add `crates/otto-workgraph` to root `Cargo.toml` `members` + `workspace.dependencies`.
- Create crate skeleton (Cargo.toml mirroring `otto-canvas` + `lib.rs`).
- **Verify:** `cargo build -p otto-workgraph` compiles (empty lib).

### Task A2 — Migration `0077_workgraph.sql` (R5.1–R5.3, R4.*)
- Append-only new file with `work_items`, `work_edges`, `work_events`,
  `work_artifacts`, `work_approvals` + indexes (per design §3).
- **Verify:** a unit test that runs `sqlx::migrate!()` on an in-memory pool passes
  (the existing `name_themes` test pattern already exercises migrate at compile time).

### Task A3 — `otto-state/src/workgraph.rs`: types + `WorkGraphRepo` (R5.*)
- Enums: `WorkKind, WorkStatus, EdgeRelation, WorkActor, ArtifactKind,
  ApprovalStatus, RiskLevel` with `parse`/`as_str` + `WorkStatus::from_source`.
- Structs: `WorkItem, WorkEdge, WorkEvent, WorkArtifact, WorkApproval` +
  `WorkItemUpsert`, `WorkItemDetail`, `MissionSummary`, `MissionFilter`, `GraphView`.
- Repo methods: `upsert_item` (by `(ws,kind,source_id)` → returns (item, created:bool)),
  `get_item`, `list_items(filter)`, `append_event`, `add_edge`, `edges_for`,
  `add_artifact`, `artifacts_for`, `request_approval`, `decide_approval`,
  `approvals_for`, `pending_approval_count`, `patch_item` (risk/goal/result),
  `summary`, `graph`, `set_cost`, `set_status`, `touch`.
- Register in `otto-state/src/lib.rs` (`pub mod workgraph;` + re-exports).
- Unit tests: enum roundtrips, `from_source` normalization for all kinds, upsert
  idempotency, approval flow, summary counts.
- **Verify:** `cargo test -p otto-state workgraph` green.

### Task A4 — `otto-workgraph` crate: normalize + `WorkGraphService`
- `normalize.rs`: pure `status(kind, raw)`, `risk(kind, title)` (uses A3 enums).
- `service.rs`: `WorkGraphService { repo: WorkGraphRepo, events: broadcast::Sender<Event> }`
  with `record(upsert)` (persist + append created/status_changed event + emit
  `WorkGraphUpdated`), mutation wrappers (add_edge/add_artifact/request_approval/
  decide_approval/patch_item/append_event) that emit, and read passthroughs.
- Depends on A3 (`otto-state`) + `otto-core` (Event) — needs A6 first for the event.
- Unit tests for normalize tables + a service test against an in-memory pool +
  a throwaway broadcast channel.
- **Verify:** `cargo test -p otto-workgraph` green.

## Stage B — Core wiring (events, RBAC)

### Task A5 — `Feature::MissionControl` (otto-core domain.rs)
- Add variant + `parse`/`as_str`; bump doc "19" → "20".
- **Verify:** `cargo test -p otto-core` green (the capability roundtrip test still passes).

### Task A6 — `Event::WorkGraphUpdated` (otto-core event.rs) + WS scope
- Add `Event::WorkGraphUpdated { workspace_id, item_id, kind: String, status: String }`.
- Add it to the **Workspace** arm of `ws_events::scope_of` (exhaustive match → compiler-forced).
- **Verify:** `cargo build -p otto-server` (match exhaustiveness) + `cargo test -p otto-server ws_events` green.

### Task A7 — Policy classification (otto-server policy.rs)
- Before the `Deny` default: `if p.starts_with("/workspaces/{id}/workgraph/") {
  return Require(MissionControl, if get { View } else { Edit }); }`.
- **Verify:** `cargo test -p otto-server policy` incl. `policy_coverage` green.

## Stage C — Server runtime & API

### Task A8 — `WorkGraphProjector` (otto-server)
- Build `WorkGraphService` into `ServerCtx` (new field `workgraph: Arc<WorkGraphService>`).
- `workgraph_projector.rs`: spawn task subscribing to `events.subscribe()`, mapping
  each relevant `Event::*` → fetch source row (swarm/goal_loops/reviews/workflows/
  product repos) + usage cost → `WorkItemUpsert` → `service.record`; create edges
  (review→pr reviews; external_trigger→session spawned; swarm→session spawned);
  ingest `TrailAppended{tool_call,action}` as work_events; 60 s reconcile sweep.
- `backfill(ctx)` fn: enumerate existing source rows → same mappers (idempotent).
- Spawn in `ottod/src/main.rs` next to the swarm scheduler; run one backfill at boot.
- **Verify:** `cargo build -p ottod`; a projector unit test mapping a sample
  `SwarmRunUpdated`/`GoalLoopUpdated`/`ReviewChanged` → expected work item.

### Task A9 — HTTP routes (otto-server `routes/workgraph.rs`)
- Handlers per design §6 (summary, items, item detail, graph, patch, edges,
  approvals request/decide, backfill). Each calls `require_ws_role` + uses the service/repo.
- Merge the router into `protected_routes()`; update `docs/contracts/api.md`.
- **Verify:** `cargo test -p otto-server` green; manual route smoke via curl after build.

## Stage D — UI (Mission Control)

### Task A10 — API client + types
- `ui/src/lib/api/types.ts`: mirror Rust DTOs (snake_case).
- `ui/src/lib/api/missionControl.ts`: typed methods.
- `ui/src/lib/events.svelte.ts`: `missionControlBus` + `work_graph_updated` handler.
- Add `'mission_control'` to the UI Feature union + capabilities handling.
- **Verify:** `npm run check` clean.

### Task A11 — Module + nav + routing
- `ui/src/modules/mission-control/`: `MissionControlPage.svelte`,
  `WorkItemList.svelte`, `WorkItemDetail.svelte`, `WorkGraphView.svelte`, `types.ts`.
- `sidebar.ts` entry; `App.svelte` import + route; e2e `helpers.ts` PAGES += 'mission-control'.
- Surfaces every traceable field (owner/goal/context/cost/result/status/risk/audit/
  evidence/approvals/relations) + live WS refresh. Tokens-only styling.
- **Verify:** `npm run check` + `npm run build` clean.

## Stage E — Tests & gates

### Task A12 — E2E specs (R7.4)
- `ui/e2e/mission-control.spec.ts`: render/overflow/a11y baseline; seed (sessions +
  swarm + goal loop + workflow via API) → `backfill` → assert summary/list show the
  kinds; open detail; request + decide an approval; switch to graph view; live update.
- Seed helpers in `ui/e2e/seed.ts` as needed.
- **Verify:** `npm run test:e2e` (mission-control specs) green on the isolated daemon.

### Task A13 — Full gate sweep + requirements re-verification (R7.6)
- `cargo build --workspace`, `cargo test --workspace`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `npm run check`, `npm run build`, `npm run test:e2e`.
- Walk the requirements matrix; mark each ☑ with impl+test refs.
- **Verify:** all gates green; matrix fully ☑.

## Stage F — Ship

### Task A14 — Merge + deploy (R7.6–R7.8)
- Commit on branch (no AI attribution), merge `feat/workgraph-mission-control` → local
  `main` (no push), rebuild+sign+install the Tauri app, replace the running daemon,
  leave it running on the new build.
- **Verify:** daemon serving new `/workgraph` routes (401 not 404 unauth); app relaunched.

## Dependency order
A1 → A2 → A3 → (A5, A6, A7 can interleave) → A4 (needs A6 for Event) → A8 (needs A4) →
A9 → A10 → A11 → A12 → A13 → A14.
</content>
