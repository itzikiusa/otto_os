# Batch-3 D3 — Agent Swarm spend / run / time budget guardrails

**Audit finding (D3):** the autonomous swarm coordinator had no spend/run/time
budget — only `max_parallel_sessions`, a *concurrency* cap. `route_result`'s
`_` arm re-queued `in_progress`/unknown turn results to `todo` forever, and
handoff-of-handoff / delegation-of-delegation loops were unbounded. Result:
uncapped token spend with no off-switch.

This change adds real, persisted budget guardrails plus a per-task attempt
ceiling, and wires them into the coordinator's `tick` and `route_result`.

## What was added

### 1. Swarm-level budgets (persisted, nullable = unlimited)

New nullable columns on the `swarms` row (migration **0032_swarm_budget.sql**):

| Column             | Meaning                                                      |
|--------------------|-------------------------------------------------------------|
| `max_total_runs`   | Lifetime run count for the swarm (counts all `swarm_runs`). |
| `max_runtime_secs` | Wall-clock budget, measured from the swarm's `created_at`.  |
| `max_cost_usd`     | Summed `SUM(cost_usd)` across runs — **soft** cap.          |
| `max_attempts`     | Per-task attempt ceiling before a task is marked `blocked`. |

`NULL` on any column = unlimited for that dimension (power-user opt-out).

These surface as fields on `otto_state::Swarm`, `NewSwarm`, and `SwarmPatch`
(`crates/otto-state/src/swarm.rs`) and round-trip through
`create_swarm` / `get_swarm` / `update_swarm`. `SwarmPatch` uses
`Option<Option<...>>` so a budget can be explicitly *cleared* to unlimited.

The DTOs `CreateSwarmReq` / `UpdateSwarmReq` (`crates/otto-swarm/src/types.rs`)
gained the four budget fields with a `double_option` deserializer that
distinguishes "field absent" (use service default) from "field present and
null" (unlimited). `SwarmPreset` also carries them.

### Defaults (so a runaway swarm self-stops)

`crates/otto-swarm/src/service.rs`:

```
DEFAULT_MAX_TOTAL_RUNS   = 300
DEFAULT_MAX_RUNTIME_SECS = 4 * 60 * 60   // 4 hours
DEFAULT_MAX_ATTEMPTS     = 3
// max_cost_usd default = None (unlimited) — cost is best-effort/soft and is
// often 0 until usage attribution lands, so it would otherwise never trip or
// trip spuriously. It IS enforced when a user sets it.
```

`create_swarm` applies these when the request omits a field; an explicit
`null` opts out. Presets supply the same defaults via YAML-overridable fields
(`default_max_total_runs` / `default_max_runtime_secs` / `default_max_attempts`
in `crates/otto-swarm/src/presets.rs`), written onto the swarm row in
`instantiate`. Existing preset YAML files need no change — the defaults apply.

### 2. Enforcement in `tick` (`crates/otto-server/src/swarm_runtime.rs`)

Before scheduling any new run, `tick` calls `budget_exceeded(ctx, &swarm)`:

- counts lifetime runs (`SwarmRepo::total_run_count`),
- computes elapsed wall-clock from `swarm.created_at`,
- sums cost (`SwarmRepo::total_cost`, `COALESCE(SUM(cost_usd),0)`),

and delegates the decision to a **pure** `eval_budget(...)` (unit-testable, no
I/O) which returns the reason for the first exhausted dimension (runs →
runtime → cost). When a budget is exceeded, `stop_for_budget(...)`:

1. sets the swarm status to `paused` (so subsequent ticks no-op),
2. flips the coordinator's `paused` flag (`set_paused`) so it stops immediately,
3. emits the existing `SwarmStatus` event (UI shows the pause),
4. posts the reason to the board (`system_post`, kind `status`), and
5. raises a `Notice` ("Swarm budget reached"). It never silently stops.

Resume/abort/pause lifecycle is unchanged — a budget-paused swarm is a normal
paused swarm; raising the budget and resuming continues it.

### 3. Per-task attempt ceiling — fixes the `route_result` `_` arm

The `_` (in_progress / unknown) arm previously always re-queued the task to
`todo`. Now it first checks `attempts_exhausted(ctx, task)`:

- reads the swarm's `max_attempts` (`None`/`<=0` = unlimited),
- counts the task's runs (`SwarmRepo::task_run_count`),
- delegates to pure `attempts_reached(max, runs)`.

When the ceiling is reached, the task is marked **`blocked`** with a board post
("blocked after exhausting its attempt budget") and a `Notice`, instead of
re-running forever. Below the ceiling, behavior is unchanged (re-queue to
`todo`).

`SwarmRepo::create_run` now sets the existing `swarm_runs.attempt` column to the
task's prior run count (0-based attempt index) instead of always `0`, so the
column is meaningful and aggregable.

## Files changed

- `crates/otto-state/migrations/0032_swarm_budget.sql` (new) — 4 nullable columns.
- `crates/otto-state/src/swarm.rs` — `Swarm`/`NewSwarm`/`SwarmPatch` budget
  fields; `row_to_swarm`, `create_swarm`, `update_swarm`; new repo queries
  `total_run_count`, `total_cost`, `task_run_count`; `create_run` attempt
  numbering; 2 new tests.
- `crates/otto-swarm/src/types.rs` — budget fields on `CreateSwarmReq`,
  `UpdateSwarmReq`, `SwarmPreset`; `double_option` deserializer.
- `crates/otto-swarm/src/service.rs` — default budget constants; wired into
  `create_swarm` / `update_swarm`.
- `crates/otto-swarm/src/presets.rs` — preset budget fields + defaults; applied
  in `instantiate`; `list_presets` populates them.
- `crates/otto-server/src/swarm_runtime.rs` — `budget_exceeded` + pure
  `eval_budget`, `stop_for_budget`, `attempts_exhausted` + pure
  `attempts_reached`; `tick` enforcement; `route_result` `_` arm fix; 4 tests.

## Tests

`crates/otto-state/src/swarm.rs`:
- `swarm_budget_columns_roundtrip` — budget columns persist through
  create/get/update (migration 0032), and `update_swarm` can clear to NULL.
- `budget_run_queries_and_attempt_numbering` — `total_run_count` /
  `task_run_count` / `total_cost` sums; `create_run` assigns attempt 0 then 1
  for the same task.

`crates/otto-server/src/swarm_runtime.rs`:
- `run_budget_stops_scheduling_at_cap` — `eval_budget` returns None under the
  run cap, a reason at/over it.
- `runtime_and_cost_budgets_stop_scheduling` — runtime + cost dimensions trip.
- `unlimited_budget_never_stops` — all-`None` budgets never stop.
- `attempt_ceiling_blocks_after_n_runs` — `attempts_reached(Some(3), …)` trips
  at the 3rd run; `None`/`Some(0)` = unlimited.

## Build / clippy / test status

All green:

- `cargo check --workspace` — Finished, no errors.
- `cargo clippy --workspace --all-targets -- -D warnings` — Finished, clean
  (the whole tree stays clippy-clean).
- `cargo test -p otto-server -p otto-swarm -p otto-state` —
  otto-server 126 passed, otto-state 30 passed, otto-swarm 3 passed; 0 failed.
  All 6 new budget tests pass.

Note: during the run, transient phantom "no `Swarm` in the root" errors appeared
in `otto-server` builds — a corrupt incremental artifact for `otto-state` left by
concurrent multi-agent builds (another agent was mid-edit on `otto-dbviewer` in
the same graph). `cargo clean -p otto-state -p otto-swarm` cleared it; no code
issue. No files outside this task's ownership were modified.
