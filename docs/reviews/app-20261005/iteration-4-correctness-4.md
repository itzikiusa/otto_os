# Iteration 4 correctness — partition 4

**Verdict: Block.** Counts: blocker 1 · major 2 · minor 0 · nit 0 · question 0.

Baseline: `03f2bc3e`, reviewed in `/Users/itziklavon/claude_ade-review` on `fix/app-review-20261005`. This is a bounded source-only review of workflows, goal loops, swarm, scheduled tasks, Mission Control/workgraph and MCP. Findings are confirmed by concrete hand traces; no tests, builds, servers or runtime acceptance checks were executed. Prior app-20261004 findings are treated as closed; the defects below are adjacent paths, not restatements of their original findings.

Intended behavior is anchored in the immutable workflow-definition contract (`WorkflowsRepo::definition_for_run`), append-only restore comments, cadence one-shot/rearm semantics, and scheduled-task lifecycle ownership comments. No source files were edited. Claude's visual/accessibility/copy, URL-selection, workflow keyboard and swarm bulk-operation ownership was respected.

## Findings

### R4-C4-01 — blocker: concurrent saves can make a workflow execute a graph different from its current saved graph

**Location:** `crates/otto-server/src/routes/workflows.rs:100`, `:114`, `:116`; `crates/otto-state/src/workflows.rs:858`. Related execution: `crates/otto-state/src/workflows.rs:445`, `:870`; `crates/otto-server/src/workflow_engine.rs:1306`. Restore has the same publication boundary at `crates/otto-server/src/routes/workflows.rs:175` and `:188`.

**Intended:** Each published workflow version must describe the exact graph/instructions saved with that version. A run pins that immutable version, so saving and then running must execute the saved definition.

**Evidence — confirmed by hand trace:** Start with workflow W at version 1. Two authorized requests PATCH W with distinguishable valid graphs X and Y.

1. Request A finishes `repo.update` and receives `updated.graph = X`, then pauses before line 114.
2. Request B writes Y, obtains its updated row, bumps to version 2 and snapshots Y. B completes.
3. A resumes, bumps the live row from version 2 to 3, and snapshots its earlier `updated.graph = X` under version 3. A's final GET returns the current live row, whose graph is Y and version is 3.
4. A later Run inserts `workflow_version = 3` from the live row. The engine calls `definition_for_run`, which replaces the current graph with snapshot 3, X.

**Actual / expected:** The editor/API says Y/version 3, but the run executes X. Expected: live version 3 and snapshot 3 agree, and a new run executes that definition. This is persistent misassociation of versioned data with possible automation side effects, not merely stale response text. No overlapping execution or rare database failure is required; overlapping saves from two clients or an agent and UI suffice.

There are two further manifestations of this same publication defect: `bump_version` performs UPDATE then a separate SELECT, so competing callers can receive the same incremented value; `snapshot_version` silently ignores a conflicting version key. Also, a run created after the version bump but before its snapshot can fail with “workflow version … is missing.” These reinforce the same required transaction boundary.

**Repair:** Move applying a workflow patch, allocating its new version and inserting the corresponding snapshot into one repository transaction. Obtain the row/version within that transaction, using `UPDATE … RETURNING` where appropriate. Use the same operation for restore, preserving restore's deliberate live name/description behavior. A per-route lock alone is insufficient unless every mutation and run-admission path participates; transactional publication lets `create_run` observe only committed version/snapshot pairs.

**Regression proposal (not executed):** Use an isolated database and barrier hooks to pause A after the graph write/read and let B publish Y before A continues; assert the live version's stored snapshot always matches its graph/instructions/restart policy and `definition_for_run(create_run(...))` matches the final saved graph. Add a save-versus-restore race and a run-admission barrier during publication: a run must pin either the complete previous version or complete next version, never a missing or mismatched snapshot. Assert all allocated history versions are unique and retained rather than silently conflict-dropped.

### R4-C4-02 — major: finishing an old scheduled run consumes a newly retimed one-shot schedule

**Location:** `crates/otto-server/src/scheduled_tasks_engine.rs:442`–`:446` (success) and `:489`–`:494` (error/cancel); edit/rearm at `crates/otto-server/src/routes/scheduled_tasks.rs:503`, `:528`; unconditional persistence at `crates/otto-state/src/scheduled_tasks.rs:307`–`:327`.

**Intended:** Changing a one-shot's `run_at` rearms it; the old occurrence must not count as completion of the newly scheduled occurrence. The explicit oracle is `cadence::rearms_once` at `crates/otto-server/src/cadence.rs:202`–`:205` and the route's rearm comments.

**Evidence — confirmed by hand trace:** Task T has `{cadence:"once", run_at:"2026-10-05T10:00:00Z"}`. Its scheduled run begins at 10:00 and remains active. At 10:01, PATCH T to one-shot 11:00. The route accepts the edit without an active-run restriction, identifies `reset_once = true`, and `rearm` clears `last_run_at`. At 10:02 the original run finishes. `complete_run` still holds the task snapshot for 10:00; it unconditionally writes `last_run_at = 10:02` and computes `next_run_at` from the obsolete 10:00 schedule, yielding None. At 11:00 and every later scheduler tick, `effective_cursor` returns the non-null last run for a one-shot, and `is_due_since` at `cadence.rs:159` returns false.

**Actual / expected:** The new 11:00 occurrence never runs, and its displayed next-run value is erased. Expected: completion records the old run's outcome while preserving the new schedule's eligibility and next-run value. The error/cancellation settlement branch has the same effect.

**Repair:** Associate schedule completion with the schedule generation captured at dispatch. Commit cursor advancement only when that generation still matches; update run history/outcome independently. Make schedule mutation, generation/rearm and next-run updates atomic as well, so the completion comparison cannot race a partially published edit. A schedule revision or equivalent compare-and-update predicate must distinguish rearming even if a user edits away and back; a plain reread followed by unconditional write is still racy.

**Regression proposal (not executed):** Start an isolated scheduled run and barrier execution before settlement. PATCH its one-shot from 10:00 to 11:00, finish the old run at 10:02, then evaluate the real scheduler due calculation at 11:00. Assert it fires exactly once, retains the new next-run display, and preserves the first run's success/error/canceled history. Exercise both success and failure settlement, plus changing an interval run to a future one-shot. Conversely, unchanged-schedule completion must still advance its cursor and prevent duplicate dispatch.

### R4-C4-03 — major: retiming an already fired workflow one-shot trigger never rearms it

**Location:** `crates/otto-state/src/workflow_triggers.rs:199`–`:214`, especially `:211`. Consumers: `crates/otto-server/src/workflow_trigger_scheduler.rs:211`–`:221`; `crates/otto-server/src/cadence.rs:159`, `:176`–`:177`.

**Intended:** An accepted change to a scheduled trigger's `run_at` rearms the new occurrence. `SCHEDULE_KEYS` includes `run_at`, and the update computes a new `armed_at` for that change. The shared cadence engine explicitly distinguishes a one-shot's fired flag from the arming timestamp.

**Evidence — confirmed by hand trace:** Trigger W has fired once with spec `{cadence:"once", run_at:"2026-10-05T10:00:00Z", last_run:"2026-10-05T10:00:00Z"}`. PATCH its spec to `{cadence:"once", run_at:"2026-10-05T11:00:00Z"}`. The route accepts it through shared cadence validation. `TriggersRepo::update` detects the changed `run_at` and updates `armed_at`, but SQL preserves the existing `last_run` unconditionally even when the request omits it. At 11:00, `is_due_armed` loads that old cursor. For `once`, `effective_cursor` deliberately ignores the new arm instant and returns `last_run`; the due check requires `last_run.is_none()`, so it rejects the trigger forever.

**Actual / expected:** Saving a later one-shot trigger succeeds, but it never fires again. Expected: changing the scheduled occurrence clears the old fired flag. No concurrency is needed. Converting any previously fired recurring trigger into a one-shot also fails this way.

**Repair:** In the atomic trigger update, preserve the server-owned cursor for ordinary edits, but clear it when a change actually creates a new one-shot occurrence, including conversion from another cadence. Compare scheduling fields against the database row at the write boundary; do not trust a client-supplied cursor. Keep unchanged resaves and prompt/destination-only edits from causing duplicate fires. Check timezone changes affecting a local one-shot instant as part of the same rule.

**Regression proposal (not executed):** Extend `trigger_cursor_is_server_owned` with a fired once-to-retimed-once update and recurring-to-once conversion. After repository update, feed the returned trigger into `is_due_armed` before and at the new instant: false, then true; after recording its new cursor, false. Also assert unchanged one-shot saves and prompt-only edits keep the old cursor. Preserve the existing stale-client-cursor tests for recurring schedules.

## Fixed-rubric provisional score

| Correctness dimension | Score / 2 | Evidence and deduction |
|---|---:|---|
| Contract/data integrity | 1.4 | Run pinning and exact-argument MCP approval binding are present, but R4-C4-01 can persist and execute the wrong versioned graph. |
| State/concurrency ownership | 1.3 | Goal-loop operation locks, workflow retry CAS and swarm dispatch/lifecycle guards exist; concurrent workflow publication and scheduled completion ownership remain defective. |
| Boundary/error behavior | 1.8 | Traced terminal lifecycle guards, canceled workflow handling, approval consume races and scheduled error settlement; detailed node/provider failure matrix remains unverified. |
| Persistence/recovery | 1.3 | Immutable run definitions and startup scheduled-run reap are present; version publication and one-shot fired flags have confirmed persistent faults. |
| Executed regression coverage | 1.0 | No tests executed in this pass. Relevant existing tests were read and prior verification documented, but the three new interleavings/retiming cases lack executed evidence. |
| **Total** | **6.8 / 10** | **Provisional source judgment; blocker/majors prevent acceptance irrespective of arithmetic.** |

The executed-coverage score describes the evidence gap; it is not an estimate of test pass rate or reliability. Root execution may add separately identified evidence without replacing this original review score.

## Coverage and limits

Substantive reads/traces: workflow save/restore/version publication, create/run pinning and queue admission, retry and cancel ownership; schedule-trigger update/cursor preservation; scheduled-task manual/scheduled in-flight ownership, completion and schedule edit; goal-loop patch/start/pause/resume/stop/retry guards and detail/list loaders; swarm coordinator/scheduled dispatch guards and stopped/result routing; MCP invocation gates, exact-caller approval selection/consume, dry-run and final transport/audit settlement. Sampled Mission Control single-flight assembly and workgraph mutation/audit broadcasts.

Safeguards observed: goal-loop lifecycle uses the same per-loop operation lock; workflow retries use a transactional active-state reservation; cancellation does not blindly overwrite a stale node snapshot; swarm scheduler and lifecycle share operation guards; MCP consumes approvals atomically and binds them to the requesting caller; scheduled manual runs hold a task-level in-flight guard outside HTTP request lifetime. The UI goal-detail component filters stored detail by selected ID, so a stale store response alone does not prove a wrong-target action.

Omitted: exhaustive workflow node implementations, external transports/providers, full swarm verify/merge/roll-up matrix, every cron/DST boundary, complete Mission Control reconciliation under event loss, MCP policy combination matrix and rendered/native UI interactions. No browser, fixtures or runtime instrumentation were started. Regression suggestions are proposals, not passing tests. No new off-lens findings are asserted.
