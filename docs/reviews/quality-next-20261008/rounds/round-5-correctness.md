# Round 5 — independent correctness review

Reviewer **19/20**. Starting head `a911ce2bc614ae430bf9a16bcd9ee7614813d9d4`, compared with `origin/main` at `7cff9e538deccdd0e346af7c6285e11863285c00`. Used the full correctness-review skill and independently read the branch's production source diff and the relevant callers. Earlier verdicts were context, not evidence of absence. Owned-source hashes and raw red/green output are in [round-5-correctness](../evidence/round-5-correctness/).

**Verdict: Approve the repaired rating-policy scope; one major finding was reproduced and fixed.** No unresolved confirmed correctness defect remains from this review. The concurrent retry-admission repair was independently rechecked below.

## Major, confirmed — a rating silently replaced explicit zero weights

**Location:** `crates/otto-server/src/eval_score.rs:321`, called by `rate_iteration` in `skill_eval.rs`.

**Intended behavior:** a human rating changes the human signal and recomputes the composite using the evaluation's configured weights. Zero is a valid relative weight excluding a signal from that composite. Commands and their evidence are preserved.

**Trigger and consequence:** an iteration has a passing lint signal, failed tests, and deliberately configured weights `{tests:0, lint:1, diff:0, review:0, human:0}`. Its correct composite is 100. Submit a human rating of 1. The old `rescore_with_human` treated `tests == 0 && review == 0` as an absent policy and replaced all weights with defaults. It thereby reintroduced failed tests and a human signal explicitly excluded by the caller, changing the composite and potentially promotion eligibility. This was an existing adjacent defect exposed by the branch's rating-publication path, not a claim that Round 4 introduced it.

**Reproduction:** the real rating-handler regression `human_rating_preserves_explicit_zero_signal_weights` failed before the repair with the persisted actual defaults `{tests:0.35, lint:0.1, diff:0.15, review:0.25, human:0.15}`, versus the configured lint-only policy. See `weights-red.log`. The test uses real repository transactions and Proof publication in an isolated SQLite fixture, rather than mocking the score calculation.

**Repair:** remove the heuristic fallback. An absent prior score already gets defaults through `EvalScore::default`; a present score retains its configured weights. The contract and TypeScript score comment now explicitly describe this behavior.

**Acceptance passed:** the saved human rating becomes 1, every configured weight remains unchanged, original failed-test and passing-lint evidence remain intact, and both iteration composite and run headline remain 100. The recovery fixture also invokes a rating with no prior score, retaining coverage of the legitimate default path.

## Independently traced coverage

- **Rating, initial scoring, and retry publication:** per-evaluation lock ownership; the fresh human signal read after long-running commands; durable pending snapshots before Proof publication; transactional rollback of iteration/headline updates; pending-score exclusion from selection and promotion, including optional Proof; fresh winner reselection after rating; original command evidence after retry interruption.
- **Cancellation and promotion:** persisted cancellation before signalling workers; retry leases and conditional terminal updates; matrix read/cancel failure propagation; root-human checks before forced promotion; waiver errors before library writes; metadata error and missing-row handling after library writes. File and SQLite publication remain separate boundaries, as documented below.
- **Validation verdicts:** explicit clean arrays versus malformed/missing output, incomplete multipass results, retained findings, stable validator identity despite out-of-order completion, and zero contribution from errored retry validators.
- **Goal-loop updates:** validate the combined candidate before writes, preserve omitted fields, reject invalid mode/empty executors/running-limit edits, and persist all fields in one SQLite statement. The caller's operation guard is retained.
- **Vault refine:** the registry, durable row, ready callback, and terminal callback now share the same turn ID; the identity check prevents an old callback from rebinding a newer turn.
- **Browser transport/admission:** pending-call RAII removal, command/event capacity failure closure, byte-charge lifetime through guard work, connection-close cancellation, safe-method cached admission versus outward-method approval, proxy accepted-task ownership and cleanup. Fixture socket substitution remains test-only and follows production vetting.
- **Changed UI state boundaries:** workspace-load generation/token ownership, cold Proof links and workspace selection, rating/poll revision checks and fresh post-write reads, scorecard response disposal, conversation focus ownership, and keyed goal-loop detail lifetime. These are source traces; rendered UX behavior is the separate design/UX reviewer's responsibility.

## Executed verification

- New rating regression: **red before repair**, then green in the complete recovery module.
- `cargo test -p otto-server --lib skill_eval::recovery_tests -- --nocapture`: **11 passed**, including the new policy regression, aborted pending-rating recovery, real child-process SQLite recovery, cancellation storage failure, promotion serialization/authorization, metadata and waiver failure, and retry-result failure recovery. The child helper is intentionally a no-op unless invoked by its parent fixture.
- `cargo clippy -p otto-server --all-targets -- -D warnings`: **passed**. This included the concurrent reviewer's newly registered latency-test source, but did not execute that test or validate its subsequent production repair.
- Owned Rust source formatting and source/document whitespace checks: **passed**.
- Rust LOC ratchet: **passed**, `otto-server 128330/130735` at this checkpoint.

No external agent provider, real user data, running production daemon, release build, full workspace suite, commit, push, or deployment was used by this reviewer. Root-owned final integration checks remain required after all Round 5 source changes settle.

## Score and remaining bounded limitation

**Correctness: 9.8/10 for the reviewed branch behavior after the rating-policy repair**, subject to final integration gates. Confidence is high for the executed evaluator handler/storage cases and medium-high for the additional source-traced paths; confidence is separate from the quality score. This is not a numerical proof that every product path is defect-free.

The remaining **0.2** is the concrete publication limitation already visible in the final code: library files and evaluation metadata are not one transaction. Process death between those writes cannot return the handler's partial-publication error, so recovery still requires inspecting/retrying that publication. Improvement: persist an intent containing the intended content hash and reconcile it on startup without replacing subsequent user edits. **Acceptance:** kill an owned process after writing the library but before metadata, reopen its isolated state and library, reconcile matching content exactly once, and preserve divergent content. Automatic reconciliation is not implemented or claimed in this branch.

## Independent recheck of reviewer 20's retry preparation repair

The inspected `retry_validation` change moves only the capped Git diff preparation before `update_guard`. The initial read chooses a worktree and rejects already-ineligible requests. After Git completes, the original guarded fresh evaluation read still determines admission status, iteration, validator, and `previous_scoring`.

Traced interleavings: a rating committed while Git is pending is present in the fresh snapshot; a cancellation after the initial read is rejected before any retry-pending mutation; deletion waits for the held retry lease instead of removing the worktree under the reader; a rejected retry drops its lease. No state mutation was moved out of the publication guard. The test-only checkpoint is keyed to the fixture path and absent from production builds. The new regression holds that checkpoint, requires cancellation to finish before it is released, then requires stale retry admission to fail and leave the validator unchanged. Its red/green execution belongs to [the performance reviewer](round-5-performance.md), not to the earlier correctness Cargo run. This independent source/test recheck found no additional correctness defect.
