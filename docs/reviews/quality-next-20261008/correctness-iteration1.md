# Correctness iteration 1

Verdict: **Approve repaired evaluator paths; broader integration remains pending** — 1 high-impact correctness finding, 2 medium findings. Review base `7cff9e538`; branch `review/quality-20261008-next`. Findings below are independently traced against current source; prior reports R08/R14/R15 supplied coverage boundaries, not verification of the present snapshot.

## Findings

### C1 — High: invalid validator output is accepted as a clean review

Confirmed by hand trace and the failing parser regression described below.

`crates/otto-server/src/skill_eval.rs:551` maps every parse error to `[]`. `run_agent_capture` at line 1011 accepts any readable output file, including empty/partially-written content, as successful. Both initial validation (lines 1468–1472) and retry (3115–3119) set `any_ok=true` before parsing. An output file containing `not json` therefore becomes `done`, passed=true, score=100, no findings. `crates/otto-server/src/eval_score.rs:317` sees all validators done and persists a passed Review artifact. This defeats the explicit agent prompt contract at `skill_eval.rs:1892`: only a valid findings array, including deliberate `[]`, is a verdict.

The same ambiguity has the opposite effect in transcript fallback at line 1022: valid `[]` fails `!parse_findings(...).is_empty()` and is not accepted until process exit/timeout, unlike a nonempty findings array.

Repaired: parse success is separate from findings cardinality; malformed records are rejected, partial files stay in place for the next poll, and valid empty transcript findings complete immediately. Initial and retry validation now share pass recording and finalization. Tests cover the actual parser, acceptance, persisted score/proof, malformed/partial output and valid clean controls.

### C2 — Medium: concurrent validators are attributed to the wrong quality dimension

Confirmed by hand trace at `crates/otto-server/src/skill_eval.rs:1513–1523`. `JoinSet::join_next()` returns completion order, but the label comes from `val_names[joined_idx]` in spawn order. With `[security, performance]` and performance completing first, its finding enters `all_findings` labelled security; the eventual security finding is labelled performance. `build_improver_prompt` (line 1915) passes the swapped dimensions to the improver. Per-validator persisted slots remain correct; the incorrect output is the skill-improvement prompt.

Repaired: every spawned task returns its originating index, which collection uses for the dimension label. A controlled reverse-order completion test asserts the actual improver prompt and failed before this change.

### C3 — Medium: a validator retry overwrites a newer human rating in the score

Confirmed by persisted regression: while a retry retains the old scoring snapshot with rating 5, `rate_iteration` accepts and persists rating 1. `eval_score::rescore_validation` replaced only review/proof fields of the old snapshot; final publication therefore wrote human rating 5 back into scoring while the iteration metadata still showed rating 1. The test failed with `Some(5)` versus `Some(1)` (`evidence/correctness-iteration1/quality-next-correctness-human-red.log`). Independent I/O awaits also allowed rating and retry score publications to interleave after the fresh read.

Repaired: human signals derive from current iteration fields; rating, retry admission and retry final publication share a short per-evaluation mutex whose weak registry entries prune idle IDs. No lock spans validator execution. A controlled test pauses the actual retry on the proof mutex, submits the real rating handler concurrently, then checks persisted iteration and headline scores.

## Verification

- Original invalid-output regression failed as expected, treating empty output and `[]` identically: `evidence/correctness-iteration1/quality-next-correctness-red.log`.
- Following parser/capture/state fixes, evaluator suite: **16 passed, 1 expected attribution failure**. Includes the real temporary-file partial-write check and persisted malformed-output/Review regression: `evidence/correctness-iteration1/quality-next-correctness-collector-red.log`.
- Attribution test uses the production collector and improver prompt, with security blocked while performance finishes. It demonstrated swapped labels before the repair.
- Human-rating suite: **2 passed, 1 expected stale-rating failure**: `evidence/correctness-iteration1/quality-next-correctness-human-red.log`.
- Coordinator-owned rejected goal PATCH regression also reproduced expected partial-write failure: `evidence/correctness-iteration1/quality-next-goal-patch-red.log`. Root owns that repair.
- Fresh focused final green: `cargo test -p otto-server --lib eval` — **26 passed, 0 failed** in 5.86s; `evidence/correctness-iteration1/quality-next-correctness-green.log`. This includes the parser, acceptance, failed-proof, attribution, latest-human and actual simultaneous rating/publication regressions.
- Coordinator-owned goal PATCH repair: fresh integration regression **1 passed**, and server `goal_loop` filter **4 passed**. Logs are retained alongside evaluator evidence. The integration binary emits the platform linker warning that its `__eh_frame` section exceeds 16 MiB; no test failed.
- The eval run emitted one `unused_must_use` warning from the new concurrency fixture discarding the rating handler response. Fixed immediately afterward with `let _ =`; this test-only cleanup awaits the coordinator's final gate. Rustfmt and diff whitespace checks pass.
- Final full integration/clippy gates remain coordinator-owned. The browser burst tests were handed back to root/native coordination and were not run by this reviewer.

## Scope and assessment

Read evaluator initial/retry admission, cancellation, score/proof persistence and promotion; matrix admission/cancel/terminal presentation; Run-with-Otto cancellation, current proof, clean revision and pinned push publication; selected workflow admission, acting-user ownership and cancel cleanup. Re-read the reported R15 fixes and adjacent branches. No fresh confirmed defect was found in the inspected pinned-push, current-proof or workflow-cancel seams. This is not exhaustive coverage of the 10,000-line workflow executor or the whole app.

Provisional correctness assessment after repairs: **9.5/10 for the inspected scope, medium confidence**; confidence is high for the repaired paths exercised by the regressions. A whole-app 9.8 claim is unsupported by this source pass. The focused Cargo results above are the only executed checks in this review; no browser suite, external publication, real provider turn or production data operation was run. Integration verification belongs to the coordinator; native session/provider lifecycle remains unverified. A separate adjacent concern remains for independent review: initial generation/score-only scoring reads an earlier iteration before command execution and publishes later without the new retry/rating guard. The UI allows ratings during that interval; this initial-scoring interaction has not received a regression in this pass and must not be claimed covered by the retry fix.

## Handoff

Owned repairs and tests handed frozen to `evaluator_independent` for the initial-generation/score-only publication seam on 2026-10-08. This report and evidence capture iteration 1; later independent edits are not covered by the 26-test snapshot above. No commits, staging, PR, merge or deployment were performed by this reviewer.
