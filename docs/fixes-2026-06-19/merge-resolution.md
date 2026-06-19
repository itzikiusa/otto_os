# Merge resolution — `feat/roadmap-batch-1` → `feat/consolidated`

Resolved the in-progress `git merge feat/roadmap-batch-1` (run while on
`feat/consolidated`, the tip of `feat/roadmap-batch-1-integrated`).

- `<<<<<<< HEAD` ("ours") = the integrated P2-feature branch — DB Explorer ERD
  relationship diagram + connection environment labels/prod write-gate; swarm
  budget guardrails / attempt ceiling / crash recovery (migration
  `0034_swarm_budgets`) + Run Inspector + Plan→Swarm.
- `>>>>>>> feat/roadmap-batch-1` ("theirs") = batch-2/3 hardening — swarm D1
  (worktree reuse), D2 (hand-added tasks default to `todo`), D3 (budget —
  DUPLICATE of the integrated work, migration `0032_swarm_budget`), and
  server-side DB query cancel.

8 conflicted files + 1 duplicate migration deleted. Left **uncommitted**
(staged) for the orchestrator to review and commit.

## Swarm cluster — integrated budget impl WINS; theirs D3 duplicate dropped

The integrated swarm BUDGET implementation (migration `0034_swarm_budgets`,
which is a superset of theirs `0032_swarm_budget`) was kept everywhere. Batch-2
fixes (D1, D2) were checked and are already present — no regression.

### Deleted duplicate migration
- **`crates/otto-state/migrations/0032_swarm_budget.sql`** — `git rm -f`.
  It both (a) collided with the integrated `0032_connection_environment.sql`
  (same `0032` prefix) and (b) duplicated `0034_swarm_budgets.sql`. `0034` is a
  strict superset: it adds `run_started_at`, `pause_reason`,
  `swarm_tasks.attempts`, and `max_attempts NOT NULL DEFAULT 3`.

### `crates/otto-state/src/swarm.rs` (9 hunks → all HEAD)
- Kept HEAD for `Swarm` (`max_attempts: i64` non-optional + `run_started_at` +
  `pause_reason`), `NewSwarm`, `SwarmPatch` (with `max_attempts: Option<i64>` —
  a plain "set this value" patch field, not double-Option), `SwarmSpend`,
  `row_to_swarm` (reads `run_started_at`/`pause_reason`), and the INSERT/UPDATE
  column order + `.bind()` order. HEAD's column order
  (`max_total_runs, max_cost_usd, max_runtime_secs, max_attempts`) and bind
  order are internally consistent, so taking HEAD for every hunk keeps the SQL
  correct.
- **Test cleanup (auto-merge collision):** both sides appended a `new_swarm`
  helper and a `swarm_budget_columns_roundtrip` test with the same names →
  duplicate-definition errors under `--all-targets`. Removed theirs' duplicate
  `new_swarm((tuple))` helper and theirs' duplicate
  `swarm_budget_columns_roundtrip`; kept HEAD's `new_swarm(ws: &Id)` +
  `swarm_budget_columns_roundtrip`. Retargeted the surviving theirs test
  `budget_run_queries_and_attempt_numbering` (which exercises
  `total_run_count`/`total_cost`/`task_run_count` — methods that DO exist on the
  integrated repo) to use HEAD's `new_swarm(&ws)` signature.

### `crates/otto-swarm/src/types.rs` (2 hunks → all HEAD)
- `CreateSwarmReq`: HEAD's plain `Option<i64>` budget fields (request passes
  through; defaults are applied by presets/`NewSwarm`).
- `UpdateSwarmReq`: HEAD's `de_double_option` for the three caps +
  `max_attempts: Option<i64>`. Dropped theirs' duplicate `double_option` helper
  (HEAD already defines `de_double_option` at the top of the file).

### `crates/otto-swarm/src/service.rs` (2 hunks → all HEAD)
- `create_swarm`: pass `req.max_*` straight into `NewSwarm` (HEAD); defaults flow
  from presets, not the request. `update_swarm`: HEAD field order.
- Theirs applied `DEFAULT_MAX_*` here; HEAD applies them in `presets.rs`
  (`default_max_total_runs`/`default_max_runtime_secs`/`default_max_attempts`),
  which still reference the `DEFAULT_MAX_*` consts in `service.rs`, so the consts
  are NOT orphaned (no dead-code warning).

### `crates/otto-server/src/swarm_runtime.rs` (4 hunks → all HEAD)
- Kept HEAD's `budget_exceeded` (returns `Option<String>`, async I/O via
  `swarm_spend`, runtime measured from `run_started_at`), `pause_for_budget`
  (persists `pause_reason`, suspends idle sessions), and the `route_result` `_`
  arm using `attempt_ceiling_reached` + `block_for_attempts` (re-queues to
  `todo`, else blocks).
- Dropped theirs' parallel helpers: `eval_budget`, `stop_for_budget`,
  `attempts_exhausted`, `attempts_reached`.
- **Test cleanup:** removed theirs' entire `#[cfg(test)] mod tests` block — every
  test in it called the dropped pure functions (`eval_budget`,
  `attempts_reached`) and built a `Swarm` missing the integrated
  `run_started_at`/`pause_reason` fields. The integrated budget behaviour is
  covered at the repo layer in `otto-state/src/swarm.rs`
  (`swarm_budget_columns_roundtrip`, `swarm_spend_sums_cost_and_counts_runs`,
  `status_manages_runtime_anchor_and_pause_reason`,
  `budget_run_queries_and_attempt_numbering`).

### D1 / D2 status (no regression)
- **D2:** `otto-swarm/service.rs::create_task` already defaults a new task's
  status to `"todo"` (so hand-added tasks are schedulable) — present on the
  integrated side. Left as-is.
- **D1:** worktree reuse lives in `swarm_workspace.rs` / `otto-git` (not in any
  conflicted file) — already merged, untouched.

### Auto-merge fix (`crates/otto-swarm/src/presets.rs`)
- `default_swarm_from_preset` builds a `SwarmPatch`. With the integrated
  `SwarmPatch.max_attempts: Option<i64>` (vs theirs' `Option<Option<i64>>`),
  `max_attempts: Some(preset.max_attempts)` double-wrapped (`preset.max_attempts`
  is already `Option<i64>`). Changed to `max_attempts: preset.max_attempts`.
  This was an auto-merged file, not one of the 8 conflicts, but the type
  interaction surfaced only after resolving the swarm types — fixed minimally.

## DB Explorer cluster — KEEP BOTH (ERD + write-gate AND server-side cancel)

Both feature sets are additive and now coexist.

### `crates/otto-dbviewer/src/types.rs` (1 hunk → merged both)
- `QueryRequest` now has BOTH `confirm_write: bool` (HEAD write-gate) AND
  `query_id: Option<String>` (theirs cancel). Kept theirs' `QueryHandle` enum +
  `CancelToken` additions. HEAD's ERD graph types (`SchemaGraph`, `GraphColumn`,
  `GraphEdge`, `GraphTable`, `NodeKind`, `statement_is_write`) are intact
  outside the conflict region.

### `crates/otto-dbviewer/src/http.rs` (1 hunk → merged both)
- Kept BOTH request structs: `SchemaGraphReq` (ERD) AND `CancelReq` (cancel).
  Both routes (`db/schema-graph`, `db/cancel`) and both handlers (`schema_graph`,
  `cancel_query`) were auto-merged and are registered.

### `crates/otto-dbviewer/src/service.rs` (1 hunk → merged imports)
- Combined the `crate::types` import list so it pulls in BOTH HEAD's ERD types
  (`statement_is_write`, `GraphColumn`, `GraphEdge`, `GraphTable`, `NodeKind`,
  `SchemaGraph`) and theirs' cancel types (`CancelToken`, `QueryHandle`). Both
  the `schema_graph` method and the in-flight registry / `cancel` method +
  `cancel_handle_for` were auto-merged and are present.

### `ui/src/lib/stores/database.svelte.ts` (1 hunk → merged both)
- Kept HEAD's `post(confirmWrite)` helper + the production write-gate retry
  (`isWriteBlocked` → `confirmGuardedWrite` → `post(true)`) AND added theirs'
  `query_id: queryId` to the POST body so the server can KILL the in-flight
  query. The `abortQuery` wiring (`POST …/cancel`), `newQueryId`,
  `runControllers` map were auto-merged and are present.

### Driver trait integrity
- `Driver::run_tracked` and `Driver::cancel` (theirs) have default impls in
  `driver.rs` (`run_tracked` delegates to `run`; `cancel` is a no-op), so every
  engine impl satisfies the trait whether or not it overrides them. MySQL and
  ClickHouse override `cancel` (KILL QUERY); Redis/Mongo use the no-op default.

## Verification (all gates pass on the resolved tree)

| Gate | Result |
|------|--------|
| `cargo check --workspace` | OK (Finished, no errors) |
| `cargo clippy --workspace --all-targets -- -D warnings` | **exit 0** (clean) |
| `cargo test --workspace` | **0 failed** across all suites |
| `cd ui && npm run check` | **0 errors / 0 warnings** (489 files) |

Ignored tests (≈31) are pre-existing env-gated E2E suites (e.g. DB Explorer E2E
behind `OTTO_DBV_E2E=1`) — ignored before the merge too, not a regression.

State: 8 conflict files + `presets.rs` staged; `0032_swarm_budget.sql` deletion
staged; merge left **uncommitted** (MERGE_HEAD present).
