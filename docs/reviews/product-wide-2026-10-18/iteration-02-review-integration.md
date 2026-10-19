# E13 integration cross-review — root, second source pass

Verdict: execution verification remains required. This is a source cross-review of
E13's five repairs, not a second twenty-reviewer round or a 9.8 assessment.
Inspected the current shared worktree; the changes remain uncommitted.

## Traced outcomes

- `run_workspace.rs::provision_worktree` reads the durable base first and saves a
  newly selected base before creating the worktree. A failed creation followed by
  repo HEAD movement reuses the saved base. An existing legacy branch/worktree
  without a saved base conflicts rather than inventing its origin. The calling
  `run_engine.rs::advance` holds the per-run claim for the whole stage, preventing
  two local stage executions from independently selecting bases.
- `run_engine.rs::stage_prove` links the task pack to its run repository before
  recomputation. `run_service.rs` repairs the link for an old pack only after
  validating workspace, work-item ID and Task kind. It recomputes the live pack
  before publication admission; stale cached run proof status is not used.
- `proof.rs::proof_identity` includes changed binary and untracked file bytes,
  symlink targets, file mode, sorted status and HEAD. Missing/read failures,
  truncated untracked listings and unsupported changed directories decline an
  identity. Command capture checks before/after identity. The core's
  `is_current_passing_test` rejects a null/mismatched automatic identity after a
  refreshed diff, while explicit manual evidence and unrefreshed legacy packs
  retain their documented semantics. Status, test policy, badges and done score
  share this predicate. Publication separately requires a clean worktree at the
  diff's HEAD. This is evidence of the intended wiring, not proof of a globally
  atomic filesystem snapshot during arbitrary concurrent writes.
- `modules.rs::pr_retry_worktree` loads the recorded run context and requires its
  original head. Both reviewer and summarizer retry call it. Missing or
  unmaterializable original evidence returns Conflict instead of using the
  current forge head. The transport credential remains typed through the
  materializer wrapper.
- `ReviewsRepo::try_begin_rerun` excludes pending/running/waiting reviewer rows
  inside the same UPDATE that marks the review running. `claim_agent_retry`
  requires a non-running review. Thus either mixed claim wins, and the other is
  refused. The last summarizer row is deliberately excluded so interrupted
  summaries remain retryable; this depends on the existing reviewers-then-summary
  array convention. Two different settled reviewers can still retry concurrently.

## Required execution evidence

Pending shared Cargo slot: core proof tests; state reviews tests; server
`command_proof_tracks_commits_and_dirty_binary_content`; `run_pr_proof` tests
including provisioning replay and old-pack policy; `pr_retry_uses_recorded_revision`;
and the affected consumers plus all-target clippy. The existing SQL-only evidence
and this source pass do not substitute for those compiled tests.

No new production defect asserted in this bounded pass. Do not interpret that as
full proof/review subsystem approval: provider failure/retry execution, long-run
cancellation, and browser proof presentation still need the integration gate.

## Focused compiled verification

Root subsequently ran `cargo test -p otto-core --lib proof::tests -- --test-threads=2`:
25 passed, 0 failed, 0 ignored. Ran `cargo test -p otto-state --lib reviews::tests
-- --test-threads=2`: 16 passed, 0 failed, 0 ignored (the substring also selects
five skill_reviews tests). Mixed reviewer/summarizer exclusion passed. Logs:
/tmp/otto-root-core-proof.log and /tmp/otto-root-state-reviews.log. These close the
core/state execution gap above; server assembly/publication/checkout and consumers
remain pending. Cargo released to Pele immediately after the batch.

## Server integration execution completed

Root rebuilt all server test targets and ran the focused proof/publication/retry/provisioning regression filters successfully in the 74/74 batch; the full server library then passed 1087 tests, 0 failures, 1 ignored. Exact names/output are in `/tmp/otto-root-server-focused-tests.log` and `/tmp/otto-root-server-lib-all.log`; durable hashes/counts in `verification-evidence.json`. This supersedes the server assembly/publication execution gap above. Final workspace all-target lint/consumer tests remain required; no claim of exhaustive provider-failure or cancellation testing.
