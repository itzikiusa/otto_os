# Round 2 — correctness and durable evaluator publication

Reviewer 11 of the campaign; correctness-review skill. Reviewed branch `review/quality-20261008-next`, starting at `195de62c8`, including the evolving shared worktree. This is FULL ROUND 2, not a reinterpretation of the earlier targeted reviewer sessions as full rounds. Scope: evaluator rating, initial/retry score publication, winner selection, Proof boundaries, failure recovery and cancellation guards. No subagents, commits, pushes, real providers, production daemon or user data were used.

Verdict: **Approve the repaired scope, subject to independent reassessment and final integration gates.** Two confirmed findings repaired: one blocker, one major. This review does not score unrelated modules as defective because they were not inspected.

## Confirmed findings and repairs

1. **Blocker — a failed rating publication returned success with contradictory durable scores.** At the original `rate_iteration` path (`crates/otto-server/src/skill_eval.rs`, now line 2833), a 5-star iteration rated down to 1 first persisted `human_rating=1`; failure of the later scoring UPDATE was swallowed. The handler returned success with the original 5-star score/headline. Winner and headline fields were also split across statements. A real SQLite trigger rejecting scoring publication reproduced this through the production handler: `rating falsely reported success after its score failed to persist`.

   Rating acceptance now atomically stores the new human signal and a **pending** score snapshot, preserving prior test/lint/diff/review signals, while clearing the headline (`crates/otto-state/src/skill_evals.rs:529`). Proof errors propagate. `publish_iter_scoring` writes scoring, optional badge, proof link, selected winner, best score and composite in one transaction (`:556`); initial scoring, retries and ratings share it. Final run summaries update all headline fields with one statement (`:334`). Pending scores are excluded from winners and promotion even with proof optional (`crates/otto-server/src/skill_eval.rs:2595`). Successful recovery replaces pending evidence without rerunning commands. Approval persistence failures are no longer swallowed. A validator retry also refreshes a saved rating's Approval before publishing, covering recovery after an earlier Approval write failed.

   Regression `rating_storage_failure_does_not_keep_a_stale_publishable_score` (`crates/otto-server/src/skill_eval_output_tests.rs:549`) injects failures at score UPDATE, headline UPDATE, Approval UPDATE and Proof status UPDATE. Each returns error, retains the current human and original command signal, leaves the score pending/headline unavailable, blocks promotion with `require_proof_pass=false` and threshold zero, then recovers on another rating call after the injected fault is removed. The command signal in this storage fixture is seeded, not a freshly executed test command. Existing command/rating interleaving coverage still executes its actual blocking shell command.

   The state regression (`crates/otto-state/src/skill_evals.rs:819`) rejects the headline after the iteration write and verifies rollback of scoring, badge and proof link. It also rejects rating admission after its first write and verifies the old human and score remain together. Proof artifacts and derived Proof status remain a separate persistence boundary: this repair guarantees safe pending publication and transactional evaluator state, **not** one transaction across all Proof operations.

2. **Major — daemon interruption during validator retry discarded original command signals.** `begin_validation_retry` previously replaced `scoring_json` with NULL. Its caller retained the original tests/lint/diff only in the worker's `previous_scoring`. After interruption, startup's `fail_running` marks the run error; another retry reads NULL and derives the score without the original signals, including a failed test signal. The regression produced the meaningful red `retry admission lost persisted command evidence`.

   Retry admission now retains the score's original signals in a durable pending snapshot and clears publication eligibility/headline (`crates/otto-state/src/skill_evals.rs:304`). `interrupted_validation_retry_preserves_original_command_signals` (`:889`) reconstructs the repository, applies the actual startup recovery method, admits a second retry, and checks that the original failed test signal is still present and pending. This is a restart-shaped repository test on the same SQLite pool; it does not claim a killed daemon or a disk/power-loss test.

## Verification executed

All Cargo work was coordinated with the parent workload lease; Git fixture configuration used `GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1`.

- Observed red: `cargo test -p otto-server --lib rating_storage_failure_does_not_keep_a_stale_publishable_score -- --nocapture` — HTTP success assertion failed on pre-repair code.
- Observed red: `cargo test -p otto-state --lib interrupted_validation_retry_preserves_original_command_signals -- --nocapture` — original command evidence missing before the retry preservation fix.
- Final server evaluator suite: `cargo test -p otto-server --lib skill_eval -- --nocapture` — **22 passed**.
- Final scoring suite: `cargo test -p otto-server --lib eval_score::tests -- --nocapture` — **3 passed**, including owned command termination on cancellation and retry/human evidence consistency.
- Final state suite: `cargo test -p otto-state --lib skill_evals::tests -- --nocapture` — **6 passed**.
- `cargo clippy -p otto-state -p otto-server --all-targets -- -D warnings` — **passed**, 28.62 seconds.
- Owned Rust files formatted with rustfmt; `git diff --check` clean at review handoff, excluding verbatim evidence logs if necessary.

Logs are retained under [round-2-correctness evidence](../evidence/round-2-correctness/). Earlier interim eight-test runs are not additional unique coverage. No full workspace test suite or external-provider execution was performed by this reviewer. Root owns independent repair review, UI error/poll synchronization and final integrated gates.

## Assessment and remaining deductions

**Correctness quality: 9.8/10 for the reviewed evaluator publication/recovery scope**, subject to independent reread. This is engineering judgment about these paths, not a test-pass percentage or a whole-product score. Scope confidence: high for traced storage failures, transaction rollback and rating/retry synchronization; moderate for abrupt process termination. Whole-product coverage is not established by this specialist pass.

Remaining deductions are bounded validation tasks, not undiscovered-feature penalties:

- **0.1 — abruptly interrupted rating publication:** pause a real rating request after the pending transaction and before final publication, cancel/drop its future, then verify a subsequent request recovers signals, headline and Approval. Acceptance: pending snapshot cannot promote and recovery retains every prior command signal. Existing SQLite abort tests and command cancellation test cover adjacent paths, not this exact interruption.
- **0.1 — file-backed process restart:** stop an isolated daemon after retry admission, restart against the same disposable SQLite database, and retry that validator using a deterministic local fixture. Acceptance: original failed test/lint signals survive, pending score cannot promote before completion, and final score/headline remain coherent. The executed repository-reconstruction test establishes the logic but does not independently validate a process/disk boundary.

No remaining confirmed defect in this inspected scope. Independent reassessment is still required before ROUND 3; source changes made by that reviewer must be validated on their final source.
