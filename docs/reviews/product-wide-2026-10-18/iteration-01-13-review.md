# E13-review — code review, proof packs, Run with Otto

## Implementation handoff — source frozen, execution verification pending

The original read-only findings below remain the audit trail. Root subsequently allocated all five fixes to E13 and approved the narrow core proof helper. All five now have implementation changes and regression source. **Do not treat them as execution-verified fixes or increase the score to 9.8: Cargo has not run.** No new visible sessions or subagents were opened. No commits, pushes, deployment, stash, or reversions.

Exact files changed by E13:

- `crates/otto-core/src/proof.rs`: current passing-test helper shared by derivation, configured test policy, badges and done score; stale/null automatic identities do not certify a refreshed diff. Explicit manual evidence and waivers preserve existing semantics. Core regression included.
- `crates/otto-server/src/proof.rs`: capture HEAD plus sorted changed-file content identity before/after assembly and command execution; stream binary file bytes off the runtime, include symlink targets/file modes, and decline inconclusive captures. PR consistency uses the same current-test predicate. Added temporary-repository regression for A→B, dirty binary bytes, commands changing the tree, and waiver preservation.
- `crates/otto-server/src/run_engine.rs`: only `stage_prove` policy linkage/error propagation is E13's hunk; link the Task pack to `run.repo_id` before recomputing. E16 separately owns push-credential edits.
- `crates/otto-server/src/run_workspace.rs`: persist base/branch/path before creating the worktree; reread/reuse durable base on replay; refuse a legacy existing tree with no recorded base instead of inventing one.
- `crates/otto-server/src/run_service.rs`: only historical Task-pack repository-link repair before live proof recompute is E13's hunk. E16 separately owns credential construction.
- `crates/otto-server/src/modules.rs`: PR retry uses stored head and errors explicitly when unavailable; reviewer retry persists checkout failure; summarizer retry respects cancellation on checkout failure and rereads findings after the atomic claim. E13 formatted only its three retry functions, not the whole shared file. E12 story deletion and E16 token resolver remain separate owners.
- `crates/otto-state/src/reviews.rs`: summarizer claim atomically excludes pending/running/waiting reviewer rows, while allowing an interrupted trailing summarizer to be retried. Added mixed admission regression in both orders.
- `crates/otto-server/tests/unit/run_pr_proof.rs`: provisioning failure-before-side-effect/replay and strict repository policy backfill regressions.
- `crates/otto-server/tests/unit/review_launch_recovery.rs`: PR retry pins A after checkout moves to B, without requiring current forge metadata; missing original metadata fails explicitly.
- `docs/contracts/api.md`: narrow proof identity/policy paragraph and summarizer retry semantics; metadata remains extensible JSON, no transport type fields changed.
- This report.

Actual verification completed:

1. Executed the exact old and new summarizer-claim SQL in isolated in-memory Python SQLite. Old SQL incorrectly admitted a pending reviewer (`True`); new SQL rejected it (`False`). New pending/running/waiting reviewer cases, settled reviewers, interrupted trailing summarizer, and empty arrays all matched expectations.
2. `rustfmt --edition 2021` succeeded on exclusively owned files and the two owned test files. Shared files were not whole-file formatted; retry functions were individually formatted through stdin.
3. Scoped `git diff --check` exited 0 after final changes.
4. Second source pass checked cancellation on failed retry checkout, manual/waiver compatibility, missing revision metadata, original base durability before failed worktree creation, binary and file-mode changes, and the post-claim findings snapshot. No new assignments were started.

**Cargo status: never acquired, never run; no Cargo process or shared build lock held by E13.** Root was sent the verification request twice. Pending commands, to be scheduled by root/Ronaldo/Pele in the shared slot:

```sh
cargo test -p otto-core --lib proof::tests
cargo test -p otto-state --lib reviews::tests
cargo test -p otto-server --lib command_proof_tracks_commits_and_dirty_binary_content
cargo test -p otto-server --lib run_pr_proof
cargo test -p otto-server --lib pr_retry_uses_recorded_revision
cargo clippy -p otto-server -p otto-state -p otto-core --all-targets -- -D warnings
```

Then run the normal affected-consumer gate when the shared workspace is stable. Compilation, full Rust regressions and clippy are **unverified**, not passing. No live CLI/forge or browser exercise was performed. If these commands expose fixture/compile errors, correct them before accepting the fixes. The legacy provisioning-without-base case intentionally returns a recoverable relaunch error; inconclusive tree capture cannot certify automatic tests; neither changes root privileges or adds an approval gate. Remaining broad review boundaries from the original report still apply.

Verdict: **Block the 9.8 readiness claim pending fixes**. Five confirmed source-traced correctness findings: four major / P1 and one major / P2. No execution-based reproduction was performed. This is a subsystem review, not a changed-lines-only PR review.

## Provenance and constraints

- Reviewer: E13-review; iteration 01; 2026-10-18.
- Branch: `fix/db-parity-session-consistency`; HEAD at start and final evidence pass: `c009da20cc113c8fb79f3bf0600442d0c6afa358`.
- The checkout is shared and moving. Initially dirty: git/local, server/workflow engine and recovery tests, sessions/manager, plus an untracked workflow module and review reports. Later swarm runtime and workflow checkpoint edits appeared too. Those edits are not mine.
- The principal finding files (`modules.rs`, `proof.rs`, `run_engine.rs`, `run_service.rs`, `run_workspace.rs`, `otto-review`) had no local diff in the final evidence pass. References below describe the inspected source, not hypothetical future fixes.
- No Cargo, build, test, install, production edit, staging, commit, or git mutation. The only authored file is this report. Findings were sent to root session `01M57PYK75FC1F2AFS4C73BBF7` with the E13-review identifier.
- Applied correctness-review, test-review, architecture-review and performance-review skills. Memory lookup supplied only moving-checkout review discipline, not evidence of current product behavior.

## Findings

### E13-01 — P1 / major: refreshing a diff makes old passing tests certify a new revision

**Locations:** `crates/otto-server/src/proof.rs:318-329`; `crates/otto-core/src/proof.rs:415-422`; `crates/otto-server/src/run_service.rs:575-589`; assembly entrypoint `crates/otto-server/src/routes/proof.rs:638-649`.

**Intended behavior:** the proof gate must substantiate the code being published. The run publication check explicitly rejects a changed revision and directs the user to assemble fresh proof.

**Concrete trace:**

1. A Task pack contains the diff for commit A and a successful recognized test command run at A. `run_command_artifact` records test kind, exit code and duration, but no revision or worktree-content identity.
2. Commit B changes the implementation. The user assembles this same pack with `cwd`, `base`, and no commands. This is a supported request; the endpoint updates the diff and runs zero commands.
3. The diff artifact now records `head_commit=B`; the existing passed command artifact is untouched.
4. `required_met` accepts any Diff plus any passed test artifact. Recompute therefore returns Passed, although B has never been tested.
5. Run publication checks that the Diff head matches current HEAD B. It never checks the command evidence's revision, so this new check also passes.

**Actual versus expected:** old tests are reported as proof of B; expected is incomplete/stale proof until relevant tests run for B, or an explicit waiver. This requires neither malicious input nor a permission bypass.

**Fix direction:** bind command/test evidence to the tested revision and working-tree content identity; validate the identity before and after execution. Derivation must exclude stale command evidence when the diff identity changes. Preserve old evidence for history instead of silently treating it as current. HEAD alone is insufficient for tests over uncommitted work.

**Regression needed:** real temporary repo A → successful test → B → diff-only reassembly; assert Partial and publication conflict, then rerun tests on B and assert Passed. Include changed dirty content with unchanged HEAD. Existing `tests/unit/run_pr_proof.rs:151-186` waives the pack and exercises diff freshness only; it cannot catch stale successful test reuse.

**Confidence:** confirmed by complete producer → assembly → derivation → publication source trace; not executed.

### E13-02 — P1 / major: Run with Otto omits the repository link required to enforce proof policy

**Locations:** `crates/otto-server/src/run_engine.rs:470-493`; `crates/otto-server/src/proof.rs:382-393`; `crates/otto-state/src/proof.rs:182-235`.

**Concrete trace:** configure a registered repository with `require_ci=true` (also applies to `require_review` and `require_pr_consistency`). Launch Run with Otto against that repository. `stage_prove` calls `gate`, which calls `ensure_pack/create_pack`. The insert does not set `repo_id`. The stage assembles a diff/self-review and recomputes without calling `set_repo_link`. `policy_for_pack` returns default policy immediately for `repo_id=None`. Add a passing test to satisfy the Task baseline: the pack becomes Passed without CI. `open_pr` recomputes the same unlinked pack and still applies defaults.

**Actual versus expected:** a repository's explicit proof requirements are ignored by its Run with Otto tasks; expected is that the run uses the selected repository's policy. The session path already links its pack before recomputation (`proof.rs:1048-1062`), demonstrating the intended integration.

**Fix direction:** persist the known `run.repo_id` on the Task pack before assembling/recomputing. Make failure to persist that link fail this stage. For existing run packs, backfill the relationship from their owning run or validate/link it at publication. Do not weaken root overrides or waiver support.

**Regression needed:** invoke the production proof stage for a run with a strict repo, assert the stored `pack.repo_id`, then add diff + successful test with no CI and assert Partial. Add CI and assert Passed. Test an existing unlinked run pack too. Pure policy tests that manually provide a linked pack do not test this integration.

**Confidence:** confirmed source trace; not executed.

### E13-03 — P1 / major: PR retries combine the original diff/findings with the current PR head

**Locations:** `crates/otto-server/src/modules.rs:3038-3057`, reviewer retry `4395-4405`, `4465-4478`, `4492-4527`, summarizer retry `4592-4596`, `4643-4654`. Original revision persistence: `2058-2076`.

**Concrete trace:** review PR head A, then push head B with changed line positions or corrected logic. Retry one failed reviewer. Its prompt and diff are restored from the original review's durable rows, but `pr_retry_worktree` fetches `provider.get_pr` now and materializes B. The original prompt tells the reviewer that its checkout is the actual reviewed source. The reviewer consequently evaluates A's diff against B's files. Retrying only the summarizer also uses B while consuming A's stored findings; `anchored_line_text` at `modules.rs:2701-2704` fingerprints the old line number using new line text.

**Actual versus expected:** retry changes the evidence revision without changing the durable diff or findings, yielding invalid review evidence and unstable/duplicated finding identities. Expected is a retry of A, or an explicit new review of B with fresh diff and agents.

**Fix direction:** use the already-persisted `ReviewRunContext.head_sha` as the required revision for PR retry materialization. If A cannot be materialized, return an explicit retry error or a clearly represented diff-only mode; do not silently claim a current checkout matches the stored diff. Both reviewer and summarizer retry need the same resolution.

**Regression needed:** mock forge PR metadata changing A→B after the first review. Assert both retry paths materialize A and preserve original line-anchor identity. Also test unavailable A instead of falling back silently to another checkout. Worktree tests currently verify a requested host head is materialized; they do not prove the caller requests the original review's head.

**Confidence:** confirmed source trace; not executed.

### E13-04 — P1 / major: summarizer retry can publish while a reviewer retry is still producing findings

**Locations:** `crates/otto-state/src/reviews.rs:201-225` and `124-131`; `crates/otto-server/src/modules.rs:4592-4617`; reviewer retry task `4511-4541`.

**Concrete interleaving:** a Done review has reviewers A and B, and B has a stored finding. Start retry A. `claim_agent_retry` atomically clears A's findings and marks that row pending, but leaves the parent review Done. Before A finishes, request summarizer retry. Its status guard accepts Done, snapshots A's now-empty findings plus B's finding, and `try_begin_rerun` accepts because it checks only the parent status. The summary publishes B alone. A later saves its new findings to its agent row; its retry task never reruns the summarizer or promotes them to comments/tracked findings. Even if A finishes during summarization, the input vector was already captured.

**Actual versus expected:** a review can finish with new reviewer findings absent from the canonical summary/comments. Expected is that summarization sees a stable, settled set of reviewer results or is rejected while retries are active.

**Fix direction:** coordinate reviewer and summarizer admission atomically in the state layer. A summarizer claim should reject active reviewer rows (`pending/running/waiting`) and take its findings snapshot only after acquiring a claim that prevents new retries. Keep separate-agent retries concurrent if desired. Avoid relying on a handler read/check, which leaves a race.

**Regression needed:** test both mixed claim orders, including one active reviewer with another nonempty batch; assert summarizer admission fails while A runs, then succeeds and includes A's new finding after settlement. `reviews.rs:632-639` and `766-776` test same-operation exclusivity separately; those tests do not exercise mixed admission.

**Confidence:** confirmed concrete interleaving; not executed.

### E13-05 — P2 / major: provisioning replay records a new base for an old worktree

**Locations:** `crates/otto-server/src/run_workspace.rs:24-31`; `crates/otto-server/src/run_engine.rs:273-290`; `crates/otto-state/src/runs.rs:155-164`; callee `crates/otto-git/src/local.rs:2052-2077`.

**Concrete trace:** provisioning creates `otto-run/<id>` and its worktree at A. The daemon stops before transitioning the run out of Provisioning (whether before or after saving the first base). The main checkout advances or switches to B. Recovery deliberately re-drives Provisioning. `provision_worktree` reads the main checkout HEAD B, while `worktree_add_if_absent` returns early for the already-existing worktree at A. The function returns B as the base, and the stage stores it. Subsequent proof/review uses B as though it were the worktree's captured origin.

**Actual versus expected:** the recovered run's diff can include unrelated reversions or changes from B, and its baseline no longer represents where its work began. Expected is replay with the original immutable A baseline.

**Fix direction:** persist the selected base before the first worktree creation and reuse it on every replay. Preserve a previously recorded base. Handle legacy partially provisioned runs by recovering a demonstrable branch origin or failing explicitly; do not substitute the current main checkout HEAD. Also ensure the chosen base branch and base commit are captured consistently across resolve/provision.

**Regression needed:** provision at A, retain Provisioning, advance the main checkout to B, replay, then assert both worktree HEAD and stored base remain A. Cover interruption before and after the metadata write. The callee's existing tests deliberately verify that reused worktrees preserve their commits; they cannot validate the caller's returned base.

**Confidence:** confirmed restart trace; not executed.

## Cross-lens assessment

| Lens | Provisional score / 10 | Basis |
| --- | ---: | --- |
| Correctness / recovery | 7.2 | Strong CAS/cancel foundations, but revision provenance and mixed retry admission still fail concrete paths. |
| Tests | 7.5 | Meaningful cancellation, provider cleanup, diff identity, worktree and duplicate-claim tests exist. The five integration/interleaving cases above are not guarded by the inspected tests. No claim that suites pass. |
| Architecture | 8.0 | Pure derivation and fallback plus state repositories are useful boundaries. Proof ownership linkage is left to individual callers, and retry identity/admission is split across transport/state helpers; these are the concrete seams causing the defects. Prefer small shared operations over a general engine rewrite. |
| Performance | 8.3, low confidence | Reviewer semaphore, capped diff capture, metadata-only recompute, batched finding events, and pruned proof locks reduce known costs. No benchmark or independent production-scale blocker established. |

Overall scoped engineering assessment: **7.6/10, provisional**, not 9.8. These scores are review judgments, not measured quality metrics; no full-product score is implied. The architecture notes explain refactor locality and are not additional defect counts.

Performance sizing: reviewer fan-out uses a shared semaphore; deterministic fallback deduplicates with a hash map and sorts distinct findings (O(F log F)); tracked finding persistence is serial per finding, with a linear seen-fingerprint membership check (O(F²) comparisons), but no measured input threshold or user-visible regression was established, so no performance finding is asserted. Review proof-pack event loading is batched rather than N+1. Snapshot generation still loads artifact contents before capping each snapshot item; aggregate memory at very large packs was not benchmarked. The skill's read-only hotspot seed against HEAD reported no changed files; the subsystem pass used direct source inspection rather than treating that result as coverage.

## Coverage and residual boundaries

Read/traced substantive paths in `otto-review/{engine,session,summarizer,worktree,fallback}.rs`, server review launch and orchestration/retries in `modules.rs`, Run with Otto stage/approval/cancel/publication/provisioning/reaper paths, proof diff/command/derivation/recompute/snapshot/PR gates, proof routes and review proof-pack assembly. Followed relevant state operations in `reviews.rs`, `runs.rs`, `proof.rs`, the worktree reuse callee, startup recovery ordering, and API retry contracts. Inspected relevant embedded tests and `tests/unit/run_pr_proof.rs`.

Positive checks: empty-diff run reviews fail rather than producing clean approval counts; review timeout stops the owned review; cancelled attempts skip summarizer fallback/persistence; summarizer result directories are attempt-isolated and managed-session cleanup covers failures; stored user decisions are reused on summary rerun; partial/fallback reviews do not resolve absent findings; PR worktree materialization verifies the head it was asked for; run PR opening recomputes live pack status and pins the pushed revision. These checks do not cancel out the findings above.

This was broad source review, **not every line of every file**. No live CLI/provider run, crash injection, browser/UX check, SQL execution, benchmark, or automated test was authorized/performed. API/DB/Kafka evidence capture, media retention/blob handling, webhook callbacks, channel delivery, complete finding-resolution workflows, all provider-specific failures, and full UI behavior were not exhaustively reviewed. Full session-holder recovery belongs to the session owner; no surviving-holder defect is asserted here. Git credential rebinding/pushurl/urlrewrite investigation belongs to E16, not E13. Existing root privileged capabilities and explicit waivers must remain usable.

Ready for coordinated ownership assignment, focused regression implementation, and cross-review. No production files have been claimed or edited.
