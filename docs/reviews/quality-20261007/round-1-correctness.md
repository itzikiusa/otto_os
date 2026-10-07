# Correctness review — round 1

**Verdict:** Block. **Counts:** blocker 1 · major 3 · minor 0 · question 0.

Baseline: `922ae483`. Review date: 2026-10-07. Review only; no source changes or runtime tests. Evidence below is confirmed by explicit source traces, not an executed reproduction.

## Scope inventory and intended behavior

This is risk-based sampling across application areas, not a claim that every file was reviewed. Newly inspected paths were selected from source rather than prior review reports.

| Area | Inspected paths and branches | Intended behavior / result |
| --- | --- | --- |
| Scheduled automation | `scheduled_tasks_scheduler.rs`, selected execution/completion/recovery/report paths in `scheduled_tasks_engine.rs`, admission/settlement/reaping in `otto-state/src/scheduled_tasks.rs`, `otto-core/src/cadence.rs` one-shot branch | Immutable captured run definition; no overlapping admission; old completion cannot consume a retimed occurrence. Admission's transaction prevents a suspected boot overlap; recovery loses the captured definition (C2). Report identity is not unique (C3). |
| Workflow durability | `otto-workflows/src/checkpoint.rs` success adoption, unknown interrupted external operation, retry exhaustion; workflow handoff/wait in automation | Completed operations are adopted; unknown outcomes fail closed; stopped workflows do not report success. Those traced branches match intent. General workflow executor was inventoried but not exhaustively traced. |
| Session lifecycle/recent change | `otto-sessions/src/lifecycle.rs` absent/unreadable transcript and fallback lookup; `manager.rs` shutdown_all/shutdown_ids, respawn shutdown guards, shutdown_for_restart | Only positively absent transcripts permit pruning; app-close sweep must allow later resume; persistent restart detaches holders. No confirmed finding in these sampled branches. Race coverage remains a validation gap. |
| Database editing | `edit-sql.ts`, `edit-mongo.ts`, `edit-redis.ts`, selected target/review/delete paths in `EditFlow.svelte.ts` | Edits/deletes identify exactly the selected underlying records. SQL direct-column/PK guard and Mongo ID preservation traced; Redis list marker violates selected-row deletion (C1). |
| Reviewed database changes | `otto-dbviewer/src/service/changes.rs` approval binding, preflight, policy/fingerprint recheck, revocation polling | Only the persisted claimed artifact runs, with target binding rechecked after preflight. No confirmed defect in this sample; native driver execution was not exercised. |
| Authentication and API contracts | `routes/auth_routes.rs` login throttle/busy/denied/success branches; telemetry client/runtime/server ingress and response header | Login failure/busy are distinct. UI telemetry emitted by the actual API client must satisfy ingest validation; ordinary client spans fail it (C4). This is not a full security audit. |

## Findings

### C1 [blocker] Deleting a Redis list row also deletes unrelated marker-valued elements — `ui/src/modules/database/edit-redis.ts:350`

**What:** The list-delete builder overwrites selected indices with a fixed `__otto_deleted__` marker, then executes `LREM key 0 marker`, deleting every existing occurrence of that valid string.

**Intended:** “Delete selected” removes only the selected elements. The generated review itself says it deletes the selected number of elements.

**Why it matters:** This permanently removes unselected database values. A partial/capped result can hide the victim from the user entirely.

**Evidence — confirmed, hand trace:** Let `LRANGE k 0 -1` return `['__otto_deleted__', 'remove-me', 'keep-me']`. `redisTarget` accepts the command and the one-column result selects `list` layout. `EditFlow` sets the editable target at lines 208–213 and calls `buildDelete([1])` at lines 807–817. Lines 350–353 generate:

```redis
LSET k 1 "__otto_deleted__"
LREM k 0 "__otto_deleted__"
```

After LSET, the list is `[marker, marker, 'keep-me']`; count-zero LREM removes both markers. Actual result is `['keep-me']`; expected is `[marker, 'keep-me']`. The review execution calls `database.runManagedStatement` (`EditFlow.svelte.ts:714`), so this is a real mutation path.

**Fix:** Remove the fixed marker algorithm. Implement an atomic exact-index deletion operation with a collision-safe marker checked against the entire list in the same atomic operation, or refuse list-row deletion until such an operation is available. A check against loaded rows alone is insufficient because results can be capped; changing only LREM's count can still remove the wrong occurrence. Keep existing list editing supported where its target is provable.

**Regression:** Extend `ui/unit/dbRedisLineSafety.test.ts` or add a focused Redis edit test with an unselected marker before and after the selected row, a partial LRANGE result, multiple selections, and duplicate list values. The assertion must prove resulting data, or prove the unsafe action is refused, rather than merely checking command words.

### C2 [major] Restarted workflow handoffs settle and deliver using the edited task definition — `crates/otto-automation/src/scheduled_tasks_engine.rs:1177`

**What:** Recovery reloads the current task and passes it to `complete_run_with`, losing the definition captured when the run was admitted. Both delivery destination and schedule generation can change underneath the run.

**Intended:** `open_run`/`admit_run` document that captured prompt and destination stay together; `settle_generation` explicitly states old runs cannot advance a newly edited occurrence.

**Why it matters:** Restarting the daemon changes the semantics of a pending run: its report may go to the newly configured destination, and a future one-shot execution can be silently lost.

**Evidence — confirmed, hand trace:**

1. A one-shot task at 10:00, generation 0, admits a scheduled run and hands off to a long-running workflow. Admission persists run identity but no captured task definition (`otto-state/src/scheduled_tasks.rs:491`).
2. At 10:01 the user retimes the task to 11:00. Edit increments schedule generation to 1 and resets `last_run_at` (`scheduled_tasks.rs:358`). The original in-memory task correctly remains generation 0.
3. Restart the daemon before workflow completion. Reaping preserves running workflow handoffs and invokes `resume_workflow_handoff`. Line 1177 loads generation 1; line 1209 uses it to finish the old run.
4. Workflow completes at 10:05. `settle_schedule` at lines 488–505 passes generation 1 to `settle_generation`. The update now matches and sets `last_run_at=10:05` for the new 11:00 occurrence.
5. At 11:00, `otto-core/src/cadence.rs:159` requires `last_run.is_none()` for `once`; it is false, so the new occurrence never fires. Without restart the generation-0 settlement would not match and the 11:00 occurrence would remain eligible.

Separately, editing destination A→B during the workflow then restarting causes the recovered old report to use B, because the same reloaded `task` reaches `deliver` in `complete_run_with`.

**Fix:** Persist the admitted run's immutable execution/completion snapshot, including schedule generation, schedule, timezone, destination, and report/proof/notification settings needed after restart; recover from that snapshot. Add a nullable append-only schema addition for rollback compatibility. For pre-migration in-flight runs lacking a snapshot, use an explicit conservative legacy recovery policy rather than silently substituting the edited definition (do not consume an unproven generation or deliver to an unproven destination).

**Regression:** Add a scheduled handoff recovery integration test: admit at generation 0, retime once to generation 1, recover waiter, settle workflow, assert the new occurrence is still due. Add a fake delivery sink proving A remains the destination after an A→B edit and recovery. Existing generation-settlement tests near `scheduled_tasks_engine.rs:1707` exercise the in-memory captured task but miss recovery.

### C3 [major] Rapid successive scheduled-task runs overwrite previous reports — `crates/otto-automation/src/scheduled_tasks_engine.rs:145`

**What:** Report identity includes task ID and a timestamp truncated to a whole second, but excludes the unique run ID.

**Intended:** Every run history row must retain the report that particular execution produced; the report endpoint looks up a file through that row's `report_rel`.

**Why it matters:** A previous run can display another run's report, and later history pruning can delete a report still referenced by a retained run.

**Evidence — confirmed, hand trace:** Run a fast shell task (`printf first`) manually, let it finish at `10:00:00.100`, then run it again after changing its prompt to `printf second`, finishing at `10:00:00.800`. The per-task in-flight guard permits these sequential, completed runs. Both calls to `report_rel` yield the same `task/reports/YYYYMMDDT100000Z.md`. `report_delivery.rs:113` uses `tokio::fs::write`, so the second completion overwrites the first file. Both rows store this same relative path (`scheduled_tasks_engine.rs:332–335`), and the run report endpoint (`routes/scheduled_tasks.rs:658`) reads it. Looking up the first run now returns “second.” Failure-report saving at line 415 has the same collision. `prune` at line 1509 unconditionally removes paths returned for old runs, so two rows sharing a path also violate retention ownership.

**Fix:** Include the immutable unique run ID in report filenames in both success and failure branches. Do not rely solely on greater timestamp precision. Preserve existing stored paths for historical reads.

**Regression:** Replace/extend `report_rel_uses_task_id_and_stamp` with two distinct run IDs and the same instant; paths must differ. Write distinct content through the real helper and read each back. Exercise pruning one run without removing the other run's report.

### C4 [major] Normal API client spans invalidate every mixed UI telemetry batch — `crates/otto-server/src/routes/telemetry.rs:157`

**What:** Client spans are renamed to `http.client.<method>.<route>`, but the ingest allowlist accepts only exact `http.client`.

**Intended:** The daemon intentionally echoes a static route template so endpoint-specific client latency can be collected (`otto-server/src/telemetry.rs:190`). UI batches must be accepted when generated by the application's own telemetry runtime.

**Why it matters:** With telemetry enabled, normal API activity drops the batch's navigation/render/client measurements and makes the diagnostics incomplete. This also weakens evidence from performance investigations.

**Evidence — confirmed, end-to-end source trace; initial suspect supplied by performance reviewer:** A GET `/api/v1/repos` receives `x-otto-route: /api/v1/repos` from middleware. `ui/src/lib/api/client.ts:338` calls `finish(..., clientSpanName(...))`; `ui/src/lib/telemetry.ts:52` produces `http.client.get.repos`. This passes runtime `SAFE_NAME` and is queued (`telemetryRuntime.ts:123–140`). The batch reaches ingest, whose exact-name match at lines 157–172 rejects the operation and returns before ingestion. `flushTelemetry` has already spliced the batch out and increments `dropped` for its entire length on failure (`telemetryRuntime.ts:97–106`), including otherwise valid spans.

**Fix:** Align the producer and ingest contract while retaining bounded safe names and prohibitions on raw URLs/identifiers. Either allow the documented bounded endpoint-name grammar or return to the exact accepted name and represent approved route identity through an explicitly validated contract. Update contract documentation and tests together.

**Regression:** Add a backend ingress test for an actual client-produced route span mixed with navigation/render spans; assert the entire valid batch is accepted. Keep negative cases for arbitrary span names, oversized names, raw URL/route attributes, and invalid components. Existing `ui/unit/telemetry.test.ts` and `telemetryHarness.test.ts` need a shared client/server contract fixture to catch this mismatch.

## Score and validation gaps

**Correctness score: 7.8/10 for this sampled scope; not an app-wide certification.** Rubric: preservation/targeting of user data 1.2/2; persisted lifecycle and recovery 1.4/2; interface contract agreement 1.5/2; routine guards and error branches 1.9/2; evidence/test confidence 1.8/2. A 9.8+ assessment requires all confirmed defects fixed, regression checks that fail against the original behavior, affected-consumer gates passing, and explicit residual gaps. Four clean-looking code paths do not compensate for one reachable destructive edit.

No builds, tests, database commands, browser sessions, live API calls, or external mutations were run: the coordinator reserved the exclusive heavy-command slot. All confirmation above is hand-traced. Native Redis execution, daemon restart with a waiting workflow, runtime telemetry ingestion, and rapid successive task execution remain to be reproduced against isolated fixtures. Session races, all provider resume formats, general workflow graph execution, and full auth coverage were not exhausted.

Suggested next commands after the new regressions exist and the coordinator grants the slot:

```bash
# From ui/: targeted existing harnesses plus new regression files.
node --test unit/dbRedisLineSafety.test.ts unit/telemetry.test.ts unit/telemetryHarness.test.ts
# From repository root: add the appropriate new test filter(s).
cargo test -p otto-automation --lib scheduled_tasks_engine
cargo test -p otto-state --lib scheduled_tasks
cargo test -p otto-server --lib telemetry
# Final affected-consumer gate after implementation.
scripts/check.sh --base 922ae483 --check
```

Off-lens: performance observations were delegated to the performance reviewer; no additional off-lens claims are asserted here.
