# Round 3 — independent correctness reassessment

Reviewer 14/20. Starting commit `d4b8626d9e4ce3af818c8d6e498fd4299f263dd2`; shared worktree source hashes and the newly built daemon hash are recorded in [the manifest](../evidence/round-3-correctness/source-manifest.json). Used the full correctness-review skill and independently traced the Round 2 changes and their callers. No nested agents, commits, pushes, deployment, external providers or real user data.

**Verdict: repaired eligibility and cancellation paths pass; one concrete publication-boundary follow-up remains.** Two confirmed major findings repaired, plus the related matrix error-propagation branch. The remaining issue is explicit below, not hidden as a confidence caveat.

## Findings and repairs

1. **Major, confirmed by failing handler regression — cancellation claimed success when its durable write failed.** `crates/otto-server/src/skill_eval.rs:2510` discarded the cancelled-status UPDATE error, then raised the worker's cancellation flag. The retry worker returns on that flag without a terminal write, so the persisted evaluation remains `running` while its worker has stopped. `cancellation_storage_failure_is_reported_without_signalling_worker` injects a real SQLite trigger rejection: the original handler failed the assertion “cancel returned success although durable cancellation failed.” Cancellation now propagates persistence failure before raising any flags. Cancel, delete, and matrix callers propagate it. Cancellation takes the same per-evaluation guard as score publication and promotion; all direct callers were checked for prior ownership to avoid recursive locking.

   The matrix caller at `crates/otto-server/src/eval_lab_routes.rs:399` also converted `list_for_matrix` errors into an empty successful list and marked the matrix cancelled. It now propagates read and cell-cancellation errors. Its regression injects a row decoding failure, then a cancellation UPDATE failure, and verifies that neither marks the matrix cancelled or signals its cell. After removing the fault, both cell and matrix cancel successfully. API documentation states the partial-failure behavior: already cancelled cells stay cancelled when a later cell fails.

2. **Major, confirmed by trace and failing interleaving regression — promotion could consume a stale eligible score during rating/retry publication.** `crates/otto-server/src/skill_eval.rs:2674` previously read its evaluation without the publication guard, then awaited live Proof recomputation before writing the library. A rating/retry can invalidate the score during that await, but promotion still tests its old snapshot. The regression holds the actual per-evaluation publication lock and drives the production promotion handler: before repair, it escaped the lock and wrote the isolated fixture skill. Promotion now acquires that guard before reading the evaluation and holds it through eligibility and library publication. The passing regression commits a pending rating while promotion waits, releases the lock, and verifies a conflict with no library write.

## Round 2 recovery tasks executed

- **Abort after pending rating commit:** `crates/otto-server/src/skill_eval_recovery_tests.rs:124` starts the real `rate_iteration` handler, pauses it immediately after its pending transaction using a test-only checkpoint keyed to the fixture iteration, and aborts/joins that task. The durable new human rating and original failed-test/passing-lint signals survive; no headline is publishable, and promotion remains blocked even with Proof optional and threshold zero. A subsequent rating call recovers the score/headline and refreshes the Approval metadata. The checkpoint does not exist in production builds and cannot intercept unrelated test iterations.
- **Actual process/disk interruption:** `:265` seeds a disposable database through production `db::open`, closes the parent's pools, and starts a separate test executable process. The child opens that file, commits production `begin_validation_retry`, records its distinct PID, and waits. The parent SIGKILLs and reaps it with the pools still open, reopens the same SQLite WAL database, and calls the actual startup recovery repository method `fail_running`. Original failed-test and passing-lint evidence remain pending and cannot promote. A second retry uses a local `[]` verdict through the production parser/final-state builder and `publish_validation_retry`; the original command signals, fresh review score, and headline remain coherent.

The latter is a real OS process and disk/WAL boundary, **not** a complete `ottod` HTTP restart or an external validator CLI test. Test/lint signals are seeded durable fixtures; they are not newly executed commands. Existing scoring cancellation tests separately execute and terminate their owned shell command.

## Verification

Evidence logs: [round-3-correctness](../evidence/round-3-correctness/).

- Original cancellation regression: **failed as intended**, `cancellation-red.log`.
- Original promotion serialization regression: **failed as intended**, `promotion-red.log`.
- `cargo test -p otto-server --lib skill_eval -- --nocapture`: **27 passed**, including the four meaningful new recovery/cancellation/promotion checks and the child helper (which is a no-op unless invoked by its parent fixture).
- Matrix read/cancellation failure regression: **1 passed**.
- `cargo test -p otto-server --lib eval_score::tests -- --nocapture`: **3 passed**.
- `cargo clippy -p otto-server --all-targets -- -D warnings`: **passed**.
- Owned-file rustfmt check and source/document whitespace checks: **passed**.
- `cargo build -p ottod`: **passed**, 55.24 seconds. Linker emitted its debug-build `__eh_frame` size warning; this is retained in `ottod-build.log`, not suppressed. Daemon SHA-256: `bd1e244b32d6895a7738a6dc8b4f0db79c19de0ef9482f90f609a27f2f795d34`.

Cargo execution was serialized and released to the performance reviewer before their workload. No full workspace suite or UI/native workload was run by this reviewer. The fresh daemon also incorporates concurrent parent-owned Vault repairs; those need their own validation and are not claimed as reviewed here.

## Remaining actionable item and score

**Correctness score: 9.7/10 for the examined evaluator publication/recovery/cancellation scope.** This is independent judgment, not a test percentage. It intentionally stays below 9.8 while the following known boundary remains unresolved:

- **R3-C3, 0.3 deduction — promotion metadata failure is discarded.** At `crates/otto-server/src/skill_eval.rs:2752`, `put_skill` writes the library, then `set_promoted(...).await` is explicitly discarded. If that UPDATE fails, the handler returns a library skill as success while the evaluation still says `promoted=false`. The reachable error branch is hand-traced; no injected failure test was executed in this round. The parent assigned its independent fault test and repair to Round 4. **Acceptance:** reject the `promoted` UPDATE using a SQLite trigger through the real promotion handler; require a visible error with an explicitly documented already-written library artifact, no false promoted marker, then remove the fault and verify retry completes coherently. Preserve human/root authorization and eligibility locking. No claim of a cross-filesystem/SQLite transaction.

Confidence is separately high for the executed handler/storage/interleaving cases and moderate for full daemon/provider interruption behavior. The process test closes the earlier same-pool reconstruction limitation, but does not prove HTTP bootstrap or external provider recovery. Additional tracing of workflow cancellation (`request_cancel` conditional UPDATE before event publication) and goal-loop start/resume/pause operation guards found no confirmed defect in those inspected branches; this was a focused trace, not a whole-module audit or executed workflow suite.

## Addendum — parent-owned Vault turn identity repair

Read-only independent review of the two-line `vault_docs_agent.rs` repair and `desktop-vault-agents.spec.ts` extension: **approve, pending the parent's fresh-daemon browser run**. At `vault_docs_agent.rs:1532–1655`, admission, durable `VaultDocsRun.id`, `on_ready`, and the finalizer now share the originally registered turn ID. The prior second generated ID made both callbacks fail their identity fence, leaving the note's binding unset/running. Reset at `:1748` still replaces the entry with the default tombstone (`turn_id=None`); a later turn installs its own fresh ID. The unchanged `update_refine_entry` equality filter rejects old callbacks in either case. The existing `detached_refine_callbacks_cannot_resurrect_reset_or_replace_new_turn` test explicitly models those guards; it was read, not rerun in this addendum.

The browser extension checks the completed `running=false` state, a successful second request, and that request's registered session. This catches the original admission/finalizer mismatch without claiming real CLI session resumption: the E2E stub does not create session records. Suggested narrow strengthening: compare the post-reset fresh ID against the immediate `followup.session_id`, in addition to the original first ID. No new confirmed defect in this repair. Hosted provenance was read from `evidence/round-3-coordinator/hosted-provenance.json`; its advisory classification is unchanged. No Cargo or browser commands were run for this review.
