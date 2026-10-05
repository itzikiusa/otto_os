# Iteration 5 implementation — partition 4

Bounded repair of R5-C4-01, pending central verification. No test/build/server or git mutation was executed by this implementer.

## Evidence

Root executed `cargo test -p otto-server --lib review5_scheduled_admission_`: **5 failed, 3 passed** (`/tmp/otto-review05-schedule-red.log`). Actual engine `open_run` tests captured rows from `list_enabled`, suspended admission behind a oneshot barrier, then committed disable, retime, disable/enable ABA or retime ABA. All four obsolete snapshots incorrectly opened history rows. Concurrent admission also opened two rows. Fresh resumed, content-only edit and manual-disabled controls passed.

## Repair

Migration 0173 adds an internal, serde-skipped `admission_generation`. Timing/timezone and enabled transitions advance it atomically with task edits. `schedule_generation` remains timing-only, so pause/resume never invalidates completion of an already-admitted occurrence.

`ScheduledTasksRepo::admit_run` holds a SQLite write transaction across the eligibility claim and history insertion. Scheduled dispatch compares enabled state, captured admission/occurrence generations and completion cursor. All admissions enforce no running row for the task. Successful admission advances the eligibility generation, preventing replay of the same captured scan even after history finishes and before cadence settlement. An insertion failure rolls back the claim. `open_run`, shared by scheduler and manual engine paths, uses this method; rejected dispatch never announces a run or reaches execution.

The admission linearization point determines ownership: an edit committed before admission invalidates old timing/eligibility; a disable after admission does not cancel that run. Manual Run now remains permitted for disabled tasks. Content-only edits intentionally preserve eligibility and execute the coherent definition captured by the scan, including its prompt and destination. Such edits apply to later captures; callers requiring a pending scan to be invalidated must disable or retime. This preserves the requested content-edit control and is explicitly documented in source; it is not a claim that content edits cancel queued or running work.

The raw history insertion method remains for existing fixtures; the production engine uses atomic admission. Full-workspace Rust literal search found only the domain declaration and state row mapper for `ScheduledTask`; both include the internal field. No HTTP/TypeScript contract changes are required because the field is serde-skipped.

## Verification handoff

- Re-run original eight actual-engine cases: `cargo test -p otto-server --lib review5_scheduled_admission_`.
- New state transaction controls (insertion rollback and replay before settlement): `cargo test -p otto-state --lib review5_scheduled_admission_`.
- Preserve earlier ordering/retime controls: `cargo test -p otto-server --lib review4_` and `cargo test -p otto-state --lib scheduled_tasks::tests`.
- Formatting and affected-consumer gates remain root-controlled and pending. No passing repair result is claimed here.
