# Automation reservation recovery — independent follow-up

Root reviewed the newly added never-started reservation release paths while the
earlier Rust snapshot ran its workspace gate. These findings are source-traced;
new focused regressions are pending. Existing Pele session owns the repairs.

## Swarm lifetime budget still consumes a released occurrence — major

`otto-state/src/swarm.rs::release_unstarted_scheduled_occurrence` changes the
queued, sessionless scheduled row to `stopped` and restores its schedule cursor.
However, `reserve_run_on`, `swarm_spend` and `total_run_count` count that row;
retention also rolls runs into the lifetime counter. With `max_total_runs = 1`,
reserve the one-shot occurrence, release before spawn, then retry: the cursor is
due again but the run budget is exhausted. The released occurrence must receive
consistent accounting treatment in admission, reporting and retention, while
started/session-bearing rows must keep consuming their budget. Required test:
one-run cap → reserve → release → successful retry, including after retention.

## Proactive release can overwrite a newer goal stamp — major

`otto-state/src/personal_agents.rs::release_unstarted_proactive_goal` rereads the
current autonomy config in a transaction, but unconditionally assigns the old
`previous_last_run_at` to its matching goal. `save_autonomy` can replace that
goal's timestamp before release. Keeping unrelated config fields does not
preserve a newer timestamp on the same goal. Compare against the stamp from this
admission before restoring it, and preserve a changed stamp. Required negative
test: admit → update same goal's stamp/config → release → newer stamp survives.

These are completion gaps in the existing recovery fix, not new access controls.
No production source was edited during this independent pass. The earlier
5,978-case workspace test pass predates these newly written reservation methods
and is not counted as their execution evidence.

## SQL NULL excludes ordinary failed proactive runs — major

`personal_agents.rs:954,1224` excludes the release marker using
`NOT (status = 'error' AND session_id IS NULL AND error = ?)`.
For an ordinary failed, sessionless row whose error is NULL, the predicate is
NULL, so SQL WHERE excludes it too. This refunds failures that were not marked
as never-started. An isolated SQLite SELECT reproduced NULL for this expression;
no application database was used. Use a NULL-safe marker comparison and test that
an ordinary error/NULL row counts while the exact release marker does not.

### Qualification after tracing admission generations

Migration0182 increments the agent's admission_generation for every autonomy
config_json update, insert and delete. admit_captured_run checks the captured
agent generation inside BEGIN IMMEDIATE before changing the goal. Therefore a
normal save_autonomy edit between the caller's config read and admission rejects
the stale dispatch; the earlier suggested pre-admission overwrite sequence is
not a confirmed reachable defect under that guard. Preserve and test this guard.
The confirmed defect remains **after admission**: a later config save can be
clobbered by release's unconditional restoration. A claimed stamp or generation
comparison can protect that restoration without a broad API/schema refactor.

## Source follow-up

Pele has now added an in-memory, serde-skipped ProactiveStampClaim captured in
admit_captured_run's transaction. The release compares the current goal timestamp
to claim.admitted before restoring claim.previous; other configuration fields
come from the current transactional read. The engine passes that claim, and
loaded historical run rows have no claim. This closes the traced overwrite path
in source without changing the HTTP DTO or requiring a migration. Both proactive
count predicates now use NULL-safe `error IS ?`. The new regression covers an
intervening same-goal timestamp edit plus an ordinary failed NULL-error row and
its daily-budget admission consequence. That regression has now passed.

Swarm admission, spend and retention share `not_unstarted_scheduled_stop`.
The marker comparison is NULL-safe `error IS`. Focused otto-state lib runs on
2026-10-18 passed `released_scheduled_occurrence_does_not_spend_the_lifetime_cap`,
`null_error_stopped_scheduled_run_still_spends_the_lifetime_cap` (NULL error and
a different error still spend the cap, including after retention),
`unstarted_proactive_goal_restores_its_previous_stamp`, and
`proactive_release_keeps_a_newer_stamp_and_counts_null_errors`.
`cargo check -p otto-assistant --lib` passed after `launch_proactive` started
passing the in-memory stamp claim. The completed package batch was not repeated.

### Swarm NULL follow-up: reproduced and repaired

Root extracted the exact current helper predicate and executed it against an
isolated in-memory SQLite table. Released, session-bearing, other-error and
running controls behaved correctly, but ordinary stopped/sessionless/scheduled
error=NULL counted0 instead of1. Root changed only `error = marker` to
`error IS marker`. All five controls then passed. Before/after predicate and
results are preserved in swarm-refund-sql-oracle.json. Pele was notified and
retains ownership of the full Rust admission/retention regression. Those
application tests have now passed: the NULL-error and other-error rows still
spend the one-run cap before and after retention, and the exact marker does not.

### Root lint integration

State/assistant/swarm all-target clippy with `-D warnings` passed in
/tmp/otto-root-refund-clippy-r2.log. The first run found only a test assertion
using get/is_none; changing it to !contains_key preserved the assertion.
Scoped rustfmt and git diff --check passed. Root's independent `release` filter
also passed three tests, including the swarm retention refund and proactive
newer-stamp/NULL cases (/tmp/otto-root-reservation-release-tests.log).
