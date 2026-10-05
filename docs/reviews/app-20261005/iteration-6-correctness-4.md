# Iteration 6 correctness — partition 4

**Approve the bounded backend repair: 0 blockers, 0 majors, 0 minors.** Runtime checkpoint `64a850e6`. R5-C4-01 is repaired and supported by executed regressions. Mounted acceptance gaps below remain open; this is a provisional score, not final partition or whole-app acceptance. The iteration-5 report and its original 9.1 score remain unchanged.

## R5-C4-01 — confirmed fixed

Reviewed only the scheduled-task stale-admission repair, its separate generation ownership, migrations 0173/0174, associated regressions and current `VERIFICATION.md`. No broad discovery, tests, builds, browser execution, production edits or git mutations were performed by this reviewer.

`crates/otto-state/src/scheduled_tasks.rs:454` now admits a captured task under `BEGIN IMMEDIATE`. Its conditional UPDATE at `:466` requires the same task/workspace, enabled state for scheduled dispatch, captured admission generation, occurrence generation and last-run cursor. It rejects a currently running task for both scheduled and manual admission. The claim increments admission generation and inserts the running history row before committing at `:505`. Failure to claim returns before insertion; insertion/decoding failure drops the transaction and rolls back the generation change.

The actual engine boundary, `crates/otto-server/src/scheduled_tasks_engine.rs:351`, calls this repository method before broadcasting the run or executing its provider. The scheduler still captures tasks at `crates/otto-server/src/scheduled_tasks_scheduler.rs:56` and spawns at `:80`, but its old captured value can no longer bypass durable admission.

Hand-traced acceptance cases:

- **Disable/retime before admission:** the edit commits first; enabled/generation comparison rejects the obsolete capture, so there is no run row, running event or execution.
- **Disable/re-enable and retime-away/back:** `scheduled_tasks.rs:294` advances admission generation on every timing or eligibility change. Equality of the eventual schedule/enabled value cannot revive the stale capture.
- **Concurrent duplicate claims:** write transactions serialize; the first increments admission generation and opens the running row, making the second ineligible. Incrementing at claim also blocks reuse of the same capture after history completion but before cursor settlement.
- **Admission before pause:** it is an already admitted run and may complete. `schedule_generation` still advances only for timing edits (`:293`), while settlement compares that occurrence generation (`:378`). Pause/resume therefore does not recreate the earlier duplicate-one-shot defect.
- **Ordinary content edit:** preserves eligibility; the captured prompt and destination remain paired as documented. A fresh resumed capture is admitted. Manual runs intentionally allow a disabled task while retaining the overlap guard.
- **Failed history insertion:** the claim rolls back with it, leaving the same occurrence available. Repository regressions at `scheduled_tasks.rs:702` and `:729` assert rollback and consumed-capture replay rejection.

The barrier regression at `scheduled_tasks_engine.rs:1458` releases the actual `open_run` only after edits commit, and asserts no history row or event plus an error. Controls at `:1580`, `:1598` and `:1624` cover content edits, manual disabled runs and competing claims. These are production-boundary tests; they do not launch an external agent.

Migration `0173_scheduled_task_admission.sql` adds a non-null default-zero admission generation, separate from occurrence settlement. Migration 0174 adds the non-unique partial index `idx_str_running_task` on `scheduled_task_runs(task_id) WHERE status = 'running'`; its predicate matches the admission overlap subquery. It changes the lookup access path without changing which rows qualify or introducing uniqueness behavior. Current migrated-schema regression execution is recorded below.

## Executed evidence and outstanding acceptance

Root's current `VERIFICATION.md` records **five intended RED failures** (with three passing controls) before the repair, followed by **57/57 GREEN** for admission and prior workflow/task controls. The same **57/57 passed again after migration 0174** at runtime checkpoint `64a850e6` (9.79 seconds, `/tmp/otto-review05-schedule-index-green.log`). This supplies the missing R5-C4-01 acceptance evidence; source checks above establish that the repaired boundary is the engine's real entry path.

Current inherited integration evidence: workspace formatting and strict all-target clippy green; workspace doc-test command green with zero cases in its 33 library targets; fresh daemon build green; merged UI check 0 errors/0 warnings, 1333 unit passes, production build and unchanged bundle budgets green. All 15 distinct merged desktop cases have passing executions, plus the additional API Automation journey. The original failed full nextest run remains historical evidence; later focused passes are not represented as a new full-suite execution.

Two concrete mounted acceptance gaps remain **queued and unrun**: (1) Scheduled Tasks deep-link opening, failed save, retry while typing, Cancel/Leave and final persistence as one real UI journey; (2) workflow-version history failure/retry in the mounted drawer. Execute those authored journeys against the final merged runtime and verify retained draft/selection and persisted results before the final evidence rescore. No defect is inferred solely from those missing runs.

## Fixed PLAN dimensions — provisional

| Dimension | /2 | Evidence / deduction |
|---|---:|---|
| Contract/data integrity | 2.0 | Existing publication/pinning/history acceptance retained; admitted task/run identity and rollback now have current migrated-schema execution. |
| State/concurrency ownership | 2.0 | R5-C4-01's stale and concurrent admission cases are red-to-green; occurrence settlement remains distinct from dispatch eligibility, with prior controls passing. |
| Boundary/error behavior | 2.0 | Rejected admission cannot emit/start; insertion rollback, manual-disabled positive control, content edits and success/error/canceled prior controls pass. |
| Persistence/recovery | 2.0 | Admission/history commit together; failed claim insertion rolls back; consumed captures cannot replay before settlement; migration 0174 was included in the 57-case rerun. |
| Executed regression coverage | 1.8 | Current affected backend repairs/failure paths pass. Deduction is the two specifically queued mounted journeys above; neither is substituted by generic UI unit/check success. |
| **Total** | **9.8/10** | **Bounded provisional evidence score; final mounted acceptance and final evidence rescore pending.** |

No arbitrary score ceiling or whole-app unknown penalty is applied. Full loop/swarm/Mission Control/MCP behavior was not re-reviewed, and this report does not extend its backend approval beyond the stated acceptance matrix. Only this report was written. Read-shell slot released.


## Final evidence calibration — 2026-10-05

The same correctness reviewer supplied this final calibration after the centrally reported 18/18 mounted acceptance run: **2.0 / 2.0 / 2.0 / 2.0 / 1.9 = 9.9/10**. The coordinator transcribed that response without changing its judgment. The original provisional 9.8 above is preserved. Scheduled Tasks deep-link, failed save, newer typing, Keep/leave and final persistence are now executed GREEN. Final UI check 0 errors/0 warnings, 1,335 units and build also pass. The sole remaining deduction is the unrun mounted workflow-version failed-page Retry and oldest-version restore journey. The reviewer ran no additional commands for this calibration.
