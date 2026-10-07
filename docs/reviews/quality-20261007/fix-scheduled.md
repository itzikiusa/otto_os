# Scheduled-run durability repair (C2/C3, T2/T3)

Baseline: `196048df`. Worktree: `claude_ade-quality-20261007`; branch: `fix/quality-20261007`.

## Implemented repair (green verification pending)

- C2: `resume_workflow_handoff` reloads the current task, losing the admitted destination and schedule generation. `ScheduledTask` deliberately skips internal generation fields in HTTP serialization, so persisting that type alone is insufficient. The admission transaction now persists a private versioned envelope with explicit generations. Recovery reads that envelope; NULL, malformed, unsupported, or mismatched snapshots terminate the scheduled history row with an explanatory error without delivery or schedule settlement. The underlying workflow remains linked and independent.
- C3: report paths contain only task ID and a second-resolution timestamp. Success and failure filenames now include the immutable run ID. Historical stored paths remain readable. Pruning deletes history in a transaction and returns only paths with no surviving references, including across task histories.
- Schema: one append-only nullable column in migration `0178_scheduled_task_snapshot.sql`; no public API/domain/TypeScript change. Older inserts omit the column and older readers ignore it.

## Test ledger

Coordinator owns all Cargo execution with jobs 2, debug info disabled and incremental compilation disabled. This worker has run no builds or test suites.

| Finding | Regression | Baseline | Repaired |
| --- | --- | --- | --- |
| C2/T2 | `quality_recovered_handoff_preserves_admitted_generation_and_destination` — new ServerCtx, edited time/destination, real resume/completion | **Failed as intended:** old completion consumed retimed occurrence; recovery command 0 passed, 2 failed (`/tmp/otto-quality-20261007-recovery-red.log`) | Pending |
| C2/T2 | `quality_legacy_handoff_does_not_consume_or_deliver_edited_task` — old raw insertion | **Failed as intended:** old completion consumed retimed occurrence (same recovery log) | Pending |
| C3/T3 | `quality_reports_at_same_instant_keep_distinct_content` — actual report writes at equal timestamp | **Failed as intended:** first report read `second run` instead of `first run`; 0 passed, 1 failed (`/tmp/otto-quality-20261007-reports-red.log`) | Pending |
| C3/T3 | `quality_prune_preserves_reports_still_owned_by_retained_runs` — shared historical path | **Failed as intended:** pruning returned a retained run’s shared path; 0 passed, 1 failed (`/tmp/otto-quality-20261007-prune-red.log`) | Pending |

Additional authored regressions (not yet executed):

- `quality_malformed_handoff_does_not_consume_or_deliver_edited_task` — fresh-context recovery from broken JSON.
- `quality_recovered_failed_handoff_report_is_owned_by_its_run` — failed workflow completion writes a run-owned report and preserves the retimed occurrence.
- `quality_admission_snapshot_preserves_nonzero_epochs_and_original_content` — explicit epoch round trip, original content/settings retained after edits, old named-field reader and finish update preserve snapshot.
- `quality_old_insert_read_finish_paths_remain_compatible_with_snapshot_schema` — unchanged raw insert omits the nullable column, old reader/update remain valid, legacy snapshot read fails conservatively.
- `quality_invalid_admission_snapshots_are_rejected_without_current_task_fallback` — broken/incomplete/null JSON, unsupported version, negative/missing epochs, unrelated task/workspace.
- Expanded the same-instant report regression with real history/prune/file deletion and surviving content assertions.

Requested green commands, to be run by the coordinator with the existing reduced-debug profile:

```sh
cargo test -p otto-state --lib scheduled_tasks
cargo test -p otto-state --test migration_compat
cargo test -p otto-automation --lib scheduled_tasks_engine
cargo test -p otto-server --lib scheduled_tasks_engine_tests
```

The root affected-consumer gate selects these Rust changes; package lib tests and the state integration migration suite are required. The new test-only checks beyond the original four baseline failures do not yet have separately executed red/green evidence.

Worker verification completed: `rustfmt --edition 2021` on the three owned Rust files (exit 0); `git diff --check` (exit 0). No Cargo command or test suite was run by this worker. No live data, provider, external delivery, staged file, commit or publication was touched.

Owned implementation files: `crates/otto-state/src/scheduled_tasks.rs`, `crates/otto-state/migrations/0178_scheduled_task_snapshot.sql`, `crates/otto-automation/src/scheduled_tasks_engine.rs`; integration regressions in `crates/otto-server/src/scheduled_tasks_engine_tests.rs`. No public HTTP DTO, core type, TypeScript contract, or existing migration was changed.

The recovery destination oracle uses distinguishable unsupported destination kinds and the real delivery dispatcher's error. It proves which captured destination reaches delivery without sending external messages; it does not exercise a live Slack/email/webhook service.
