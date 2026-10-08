# Independent evaluator correctness review — iteration 3

Verdict: Approve with fixes applied, pending the campaign's final integration gates. Three confirmed defects resolved (one blocker/lost update, two major correctness failures). This is a focused correctness verdict, not a four-vertical release certification.

Reviewer: independent evaluator reviewer (campaign reviewer 6). Base/HEAD: `7cff9e538deccdd0e346af7c6285e11863285c00`; reviewed the shared working-tree changes on `review/quality-20261008-next`, 2026-10-08. The preceding author explicitly handed over evaluator files before edits. No live user data, external providers, publishing, or delegation used.

## Intended behavior and findings

1. **Blocker — initial scoring overwrote a rating accepted during command execution. Confirmed by an observed red regression; fixed.** The rating UI remains enabled while a run works (`ui/src/modules/skills-eval/RunDetail.svelte:608`). Initial generation and score-only scoring previously held the iteration snapshot across test/lint commands, built the human signal from that old snapshot, then published over a newer rating. The score-only regression blocks a real shell command on a release file, calls the real rating handler, releases the command, and checks persisted iteration/score/headline/Approval evidence. Before the fix, `human_rating=Some(1)` but `scoring.human.rating=None`, and the test failed on precisely that assertion. The final phase now takes the existing evaluation update lock, re-reads the iteration, updates evidence, and persists scoring together (`crates/otto-server/src/eval_score.rs:233`). Commands stay outside this lock. Both callers no longer publish an unguarded returned snapshot. Final generation and score-only summaries re-read persisted scores under the same lock (`crates/otto-server/src/skill_eval.rs:1680`, `:1812`).

2. **Major — incomplete multi-pass validators published clean review evidence. Confirmed by concrete trace and green regression against the fixed behavior; fixed.** For two requested passes, `[]` followed by malformed output previously produced `scores=[100]`, `findings=[]`; `finish_validation` only rejected an empty scores vector, so it marked the validator done/passed. Proof review treated that as complete. The finalizer now requires all requested verdicts, marks partial sets error/0, and preserves real findings already collected (`crates/otto-server/src/skill_eval.rs:622`). Initial and retry paths share this finalizer. The actual review-artifact test covers clean+malformed, malformed+clean, and two clean passes. The generation early-perfect predicate also requires every expected validator result with score 100, so missing/error output cannot advertise “all validations passed” (`:1601`). Requested passes remain capped at 1–3 by existing callers; no extra policy was invented.

3. **Major — rating a different iteration, or lowering the winner, left the best iteration/headline stale. Confirmed by trace and regression; fixed.** Previously, rating publication only updated `composite_score` when the rated iteration was already the winner; `best_iteration`/`best_score` were not recomputed. Concrete case: iteration 1 rates 5/5, iteration 2 rates 4/5, then iteration 1 is corrected to 1/5. The best iteration must become 2 at 80. A shared fresh-score winner selector is used by ratings and retry publication, with latest iteration winning ties (`crates/otto-server/src/eval_score.rs:71`, `crates/otto-server/src/skill_eval.rs:2853`). The regression verifies all three headline fields.

## Additional independent coverage

- Read parser/capture, initial and retry loops, scoring/Proof publication, rating handler, UI rating admission, contract, and existing/new regression paths. Traced explicit `[]`, fenced/wrapped verdicts, empty/malformed entries, incomplete files, ignored errored outcomes, successful and incomplete multi-pass paths, concurrent completion attribution, retry/rating lock order, initial scoring/rating order, cancellation while a command runs, and final winner publication.
- Reviewed the atomic Goal PATCH route/store/test diff: all candidate fields validate before one UPDATE; storage failure aborts all fields; missing values preserve existing settings; mode/status restrictions remain. Confirmed create still invokes the shared validator after executor non-empty validation moved there. No additional Goal PATCH defect found. This reviewer did not independently execute the integration suite; the campaign's broader gate covers it.
- Parser deliberately takes the first JSON object/array and tolerates surrounding prose/fences. It does not silently salvage a nested empty array from an error object. Arbitrary earlier bracketed prose may be rejected; that conservative behavior is not scored as a successful verdict.
- Contracts now describe all-pass completion and rating-safe publication. Public DTO shapes did not change.

## Verification actually executed

- `cargo test -p otto-server --lib initial_scoring_publishes_rating_saved_while_commands_run -- --nocapture`: observed meaningful red, `left: None`, `right: Some(1)`, after fixing fixture-only required-field setup errors.
- `cargo test -p otto-server --lib skill_eval::output_tests -- --nocapture`: 7 passed, including actual score-only command/rating interleaving, retry/rating publication, reselected winner, and real Review artifact assertions.
- `cargo test -p otto-server --lib eval_ -- --nocapture`: 6 passed, including cancellation process termination and stale retry human-signal regression. This filter does not select the entire skill-evaluator suite.
- Requested companion browser check: `cargo test -p otto-browser --lib cached_ -- --nocapture`: both cache/burst tests passed. The first attempted filter `process_burst_tests` selected zero tests; it is not counted as validation.
- `rustfmt --edition 2021` on the three owned Rust files and `git diff --check`: clean. A final empty-match to if-let cleanup was made after the evaluator build started; the parent final integration gate must compile the exact final source. No Cargo runs after handing the lease to the isolated terminal measurement.

## Scores and limits

| Area | Score / 10 | Basis and limit |
|---|---:|---|
| Evaluator correctness in reviewed paths | 9.4 | Lost rating, incomplete verdict, and stale winner defects fixed with observable persisted-output assertions. Full generate-mode external-provider execution and arbitrary cancellation interleavings were not exercised here. |
| Regression evidence for these fixes | 9.5 | Actual red for command/rating overwrite; production handler, score-only runner, SQLite, and Proof artifact assertions. Does not prove crash atomicity across separate score/headline statements. |
| Goal PATCH correctness review | 9.2 | Full changed-path trace and existing database rejection test inspected; no independent integration execution in this reviewer slot. |
| Whole evaluator/product readiness | Not scored | Provider/native/full-workspace behavior exceeds this focused review. |

Scores express coverage and remaining uncertainty, not test counts. This reviewer cannot justify 9.8 from this evidence. Final full gates and the campaign's other reviewers remain required. No unresolved confirmed defect from this focused pass; database-failure injection across multi-statement score publication, process crashes, live-provider timing, and full native flows remain explicit coverage limits.
