# Iteration 5 correctness — partition 4

**Approve with fixes: 0 blockers, 1 major, 0 minor.** Bounded source review at `418e963a`, covering repaired automation acceptance paths and adjacent admission/merged UI interactions. This is not a fresh exhaustive review of workflows, loops, swarm, Mission Control or MCP. No tests, builds, benchmarks, production edits or git mutations were performed.

## Confirmed finding

### R5-C4-01 — major: Scheduled Tasks admits an obsolete occurrence after pause or retiming commits

**Locations:** `crates/otto-server/src/scheduled_tasks_scheduler.rs:56`, `:74`, `:80`–`:83`; `crates/otto-server/src/scheduled_tasks_engine.rs:295`–`:297`, `:346`–`:360`, `:376`.

**Intent:** Pausing disables subsequent scheduled admission; retiming invalidates a captured old occurrence. An already admitted run can finish independently. The repaired settlement generation must continue distinguishing those cases.

**Confirmed by hand trace, not executed:** At 10:00 a scheduler tick obtains enabled one-shot T, due at 10:00, from `list_enabled`. Before its spawned future opens the run, PATCH disables T (or changes its occurrence to tomorrow) and commits. The scheduler only claims the process-local in-flight set and passes the original T to the spawned `run_task`. `run_task` calls `open_run`; that function unconditionally creates the history row using the captured IDs. `complete_run` then calls `execute` with the original T. There is no enabled-state or occurrence-generation validation at this boundary. Consequently a newly opened scheduled execution runs after the disable/retime has completed, using the obsolete task definition. The settlement fence can reject its eventual cursor write after retiming, but cannot undo executed agent/shell/workflow or delivery effects.

The interleaving is reachable at the spawned-task scheduling boundary and the asynchronous row insertion, without another scheduler or process. This is distinct from pausing an already admitted execution and from the now-correct transactional **workflow-trigger** admission path. The iteration-4 follow-up expressly limited its claim to settlement and did not certify this separate scheduler.

**Repair:** Give scheduled dispatch a durable admission boundary shared with task edits: validate current enabled state and captured scheduling identity while atomically opening the run. Preserve separate occurrence identity for completion so pause/resume cannot reintroduce the earlier duplicate-once defect. If disable/re-enable must invalidate a captured tick, use a separate eligibility epoch. A plain reload followed by an unrelated insert leaves the same race.

**Required verification:** Barrier the actual scheduled admission path after capturing a due T but before opening its history row. Commit disable, retime, and retime-away/back respectively, release the barrier, and assert no obsolete run row or execution starts. Add a current-due positive control and unchanged-content edit control. Retain all six pause/completion ordering cases and retiming settlement tests; run them through the production boundary. No proposed test here is claimed passing.

## Repair dispositions and inspected evidence

- **Atomic publication/pinning holds.** Read `crates/otto-state/src/workflows.rs:383`–`:435`, `:451`–`:498`. Publication obtains its write lock on the initial UPDATE, derives the snapshot from that transaction's live row and commits both together. Collision errors roll back. Shared run insertion pins the committed version and writes progress inside the same transaction. Engine lookup at `crates/otto-server/src/workflow_engine.rs:1303` resolves the admitted run's pinned definition. PATCH/PATCH and PATCH/Restore interleavings cannot expose the earlier split version/graph write.
- **One-shot settlement repair holds.** Read `crates/otto-state/src/scheduled_tasks.rs:265`–`:380` and `crates/otto-server/src/scheduled_tasks_engine.rs:504`–`:522`, `:1436`–`:1547`. Pause/resume leaves occurrence generation unchanged; completion records the consumed one-shot for success/error/canceled in either ordering. Timing/timezone edits increment identity; retime-away/back cannot restore old completion ownership. History remains independently settled. No regression asserted in these paths.
- **Workflow-trigger admission repair holds.** Read `crates/otto-state/src/workflow_triggers.rs:187`–`:285` and `crates/otto-server/src/workflow_trigger_scheduler.rs:119`–`:158`, `:587`–`:710`. Schedule/eligibility changes advance admission generation, claims check that generation and captured cursor under a write transaction, and overlap/queue/progress changes commit together. Traced two competing claims, different triggers for one workflow, replay after the first run finishes, and insertion rollback. Rejected claims leave cursor and queue untouched.
- **Merged adjacent UI sample:** inspected merge statistics plus source diff for `ui/src/modules/loops/LoopDetail.svelte`, `ui/src/modules/scheduled-tasks/ScheduledTasksPage.svelte`, and `ui/src/modules/workflows/TriggersPanel.svelte`. The scheduled-task merge adds route/expanded-task synchronization while retaining the pause API action and edit ownership fence. The loop detail changes inspected concern icon/animation. This limited source sample does not certify all 45 merged P4 UI files or their mounted behavior.

## Fixed PLAN dimensions

Scores apply to the stated bounded acceptance matrix, preserving the PLAN rubric. Existing executions count even though another agent ran them; no arbitrary global ceiling is applied.

| Dimension | /2 | Evidence / exact deduction |
|---|---:|---|
| Contract/data integrity | 2.0 | Atomic publication, immutable pinning, progress projection and history traversal repairs have named passing regression evidence and unchanged backend source across the inspected merge. No remaining defect in this bounded matrix. |
| State/concurrency ownership | 1.4 | R5-C4-01 permits obsolete scheduled admission after an accepted pause/retime. This significant concrete defect is the deduction. |
| Boundary/error behavior | 2.0 | Named success/error/canceled settlement cases and failed trigger insertion rollback passed; source recheck preserves their production boundaries. |
| Persistence/recovery | 2.0 | Published snapshot/run/progress commit boundaries and one-shot consumed-cursor persistence are supported by the focused executed cases and current traces. This score does not claim exhaustive restart/provider coverage. |
| Executed regression coverage | 1.7 | Substantial named repair matrix passed, but R5-C4-01 lacks its barrier regression and the newly merged scheduled-task route/expansion interaction lacks a current merged-revision mounted check. Add those exact cases and rerun the two repaired integration checks plus omitted gate phases. |
| **Total** | **9.1/10** | **Provisional; the major prevents acceptance regardless of arithmetic.** |

## Execution provenance and limits

`VERIFICATION.md:330` records 30/30 consolidated automation/publication tests passing, including all six pause/settlement cases, stale retime/disable/away-back admission, queue failure rollback, overlap, captured-cursor replay and version/pinning/history. This is inherited execution evidence, not a new run by this reviewer. `VERIFICATION.md:354` records preintegration UI check 0 errors/0 warnings and 1314 unit passes. Coordinator reports 12 desktop plus one WebKit pass before integration. `VERIFICATION.md:378` records Rust formatting/clippy green and 4694 passed / 2 failed / 86 skipped; the route fixture and MCP payload-budget failures were source-fixed but require rerun. The script stopped before doc-tests/UI. Full merged-source gates remain unverified.

No new full sweep of loop lifecycle, swarm rollup, Mission Control reconciliation, MCP transport/policy combinations, external providers, cron/DST or native UI was performed. Their prior review dispositions are not overwritten. Only this report was written. Read-shell slot released.
