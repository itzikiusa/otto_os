# Round 4 — correctness publication faults

Reviewer 16/20. Starting commit `2913a7d17`; final owned-source and daemon hashes are in [the manifest](../evidence/round-4-correctness/source-manifest.json). Used the full correctness-review skill, its evidence/severity template, and the test-driven-development skill. No nested agents, commits, pushes, deployment, external providers, or real user state.

**Verdict: Approve the repaired evaluator publication scope.** Three major findings and one narrow minor finding were confirmed and fixed; no unresolved confirmed finding remains in this inspected scope. This is not a whole-product correctness guarantee.

**Intended behavior:** a successful promotion means its library write and promotion marker succeeded; a forced override must record its promised Proof waiver. A validator result must persist before retry completion publishes the score or marks the run done. Cancellation and human/root restrictions must survive these repairs.

## Findings and fixes

### Major — promotion discarded its metadata error

**Location:** `crates/otto-server/src/skill_eval.rs:2755` (R3-C3 follow-up).

**What / why:** after replacing a library skill, `set_promoted` failure was discarded and the handler returned success while the evaluation still said `promoted=false`.

**Evidence:** confirmed by the real `promote_skill` handler and a SQLite trigger rejecting the metadata UPDATE. The original code failed `promotion_metadata_failure_reports_written_skill_and_can_retry` with “promotion returned success although its metadata write failed” in `promotion-red.log`. The fixture primes a real existing library entry/cache, so it exercises replacement, not merely creation.

**Fix and acceptance:** return HTTP 500 explicitly naming the already-written skill and advising retry; retain the written artifact rather than attempting destructive rollback. The test verifies the rendered HTTP error, replacement body, absence of false promotion metadata, removal of the fault, and successful retry with the correct actor and timestamp.

### Major — forced promotion ignored Proof waiver failure

**Location:** `crates/otto-server/src/skill_eval.rs:2729`.

**What / why:** the unmet-gate force branch promised a visible Proof override but discarded `waive` errors, then replaced the library and marked the evaluation promoted without that waiver.

**Evidence:** confirmed by a trigger rejecting `waived_by` updates on the actual fixture proof pack. The original handler failed `promotion_waiver_failure_preserves_library_and_can_retry` with “forced promotion returned success although its proof waiver failed” in `promotion-red.log`.

**Fix and acceptance:** propagate the waiver error before the library write. The passing test verifies the old library body remains, the run stays unpromoted and unwaived, and removing the fault allows a successful forced promotion with the expected waiver actor. The existing best-effort audit policy in `ServerCtx::audit` remains intentional and unchanged.

### Major — retry result storage failure became a completed run

**Location:** `crates/otto-server/src/skill_eval.rs:3067`, called by the retry worker at `:3242`.

**What / why:** the worker discarded `set_iter_agent_at` failure, then rescored the saved pending validator and marked the run done. The actual completed validator result was lost while the UI saw a completed run. Promotion itself remained blocked by the pending validator; that does not make the completion truthful.

**Evidence:** confirmed by a trigger rejecting `agents_json`. The existing completion sequence was extracted without altering its ignored-result behavior so the test could invoke the exact production path. `retry_result_storage_failure_stays_pending_and_recovers` failed with actual `Done`, expected `Error` in `retry-result-red.log`.

**Fix and acceptance:** completion now saves the validator result with error propagation before rescoring, inside the cancellable operation. A failure records the run as `error` with the storage error, keeps the pending score and preserved test/lint signals, leaves the headline unpublished, and cannot promote. Removing the fault and running the real completion path again publishes a coherent done result. Failure of the terminal-status write is now logged rather than silently discarded. The five new tests do not launch any external validator; they use a deterministic final validator state with the real repository, scoring, and completion code.

### Minor — a vanished evaluation still counted as a successful metadata write

**Location:** `crates/otto-state/src/skill_evals.rs:642` and the handler's `:2759`.

**What / why:** an UPDATE affecting zero rows returned success. Deletion acquires the publication lock for cancellation, then releases it before its final row deletion; a forced promotion can have already read that row. A missing row at the metadata boundary therefore must not produce a success response.

**Evidence:** confirmed by a handler fault test whose SQLite trigger deletes the evaluation at the metadata UPDATE boundary and suppresses that UPDATE. The original repository failed `promotion_does_not_report_success_when_metadata_row_disappears` with “promotion returned success when no metadata row was updated” in `promotion-disappeared-red.log`. This deterministically models the missing-row boundary; it does not claim an executed HTTP delete/promotion scheduling race.

**Fix and acceptance:** require one updated row, return NotFound internally when absent, and expose a distinct partial-publication error explaining that the evaluation no longer exists and the already-written library skill should be inspected. Do not recommend a retry that cannot succeed. The test verifies the retained skill and removed row.

## Independent verification and boundaries

Evidence is in [round-4-correctness](../evidence/round-4-correctness/).

- `cargo test -p otto-server --lib skill_eval -- --nocapture`: **32 passed**, including all five new tests and the prior cancellation persistence, promotion serialization, abort-after-pending-rating, and real child-process SQLite recovery tests.
- Explicit forced-promotion authorization regression: a non-root human and a root-owned managed-session credential both receive Forbidden, with no library write or promotion marker.
- `cargo test -p otto-state --lib skill_evals -- --nocapture`: **6 passed**.
- `cargo test -p otto-server --lib eval_score::tests -- --nocapture`: **3 passed**.
- Performance reviewer's requested `transcript_` filter: **32 passed**; log retained under `evidence/round-4-performance/transcript-tests.log`.
- `cargo clippy -p otto-server -p otto-state --all-targets -- -D warnings`: **passed**.
- `cargo build -p ottod`: **passed**, 1 minute 18 seconds. Its debug linker `__eh_frame` size warning is retained in `ottod-build.log`; it was not suppressed. SHA-256 `14335b30342d5ab8603e9716620d545778a20be7c86775e65dfa1463c00f8dd9`.
- Owned-source rustfmt and whitespace checks: **passed**. Final hashes are in the source manifest.

All Cargo work was serialized. No browser/native measurement, full workspace suite, UI check, or release packaging was run by this reviewer. Parent-owned integration verification remains separate. Contract and TypeScript request comments now describe partial publication and root-human authorization.

## Score and concrete remaining limitation

**Correctness: 9.8/10 for the reviewed evaluator publication, cancellation, recovery, and promotion scope.** All four confirmed findings above are repaired and their fault tests pass. Confidence is high for the exercised storage/handler/completion branches and moderate for complete daemon/provider lifecycle behavior; confidence is separate from this quality score.

The remaining **0.2** reflects one explicit operational limitation: library files and evaluation metadata are not a distributed transaction. A process death between those writes cannot return the new explanatory error, and recovery still requires inspecting/retrying the library publication. A concrete optional improvement is a durable publication intent with the intended content hash and startup reconciliation that never overwrites a user's subsequent edit. **Acceptance:** kill an owned process after the file write but before metadata, reopen the isolated state/library, reconcile the matching hash exactly once, and refuse to overwrite divergent content. No automatic reconciliation is implemented or claimed here; introducing it is a separate persistence design, not a reason to hide the current bounded recovery behavior.
