# Iteration 01 — Git, repositories and PRs (agent 03)

Verdict: **Block pending explicit stash deletion identity fix and final lint verification.** Worktree provisioning, selected-stash apply, automatic stash capture/restoration and provider query encoding are fixed; explicit stash Drop retains a destructive race. Provisional quality score: **8.2/10**, not the requested 9.8. The score reflects unresolved safety work and incomplete runtime coverage, not a measured product-wide score.

## Scope and provenance

Reviewed the existing subsystem, not only the PR diff. Starting HEAD: `c009da20cc113c8fb79f3bf0600442d0c6afa358`. Shared checkout; changes from other reviewers are separate. Owned production/test edits: `crates/otto-git/src/local.rs` and `crates/otto-git/src/providers/{github,gitlab,bitbucket}.rs`. No live user repository mutations, remote actions or commits were executed. Cargo ran only in the slot assigned by root. Fixture Git mutations used temporary repositories only.

Read/trace coverage:

- `local.rs`: worktree discovery, creation, reattachment and removal; branch existence; stash save/apply/drop/pop; merge preview/start/autostash/continue/abort; process and path safeguards and relevant existing fixture tests.
- `push.rs`: source/destination pinning, explicit force lease, tracking-ref lookup, reflog integration check, publication SHA validation.
- `ops.rs`: pull modes and auto-stash conflict/error restoration.
- `patch.rs`: stale fingerprint rejection, hunk patch construction, backup-before-discard and apply flow.
- `working_diff.rs`: bounded staged/unstaged/untracked preview and unchanged index intent.
- `http.rs`: repository/workspace authorization, account selection/rebinding/adoption, credential ownership checks, PR creation/proof hooks, local operation locks and stash routes.
- `parse.rs`: status and worktree parsing; `providers/github.rs`: pinned merge, fork-branch cleanup, provider requests; `providers/client.rs`: cache ownership, invalidation and request documentation.

This is a risk-directed source review. It is **not** a claim that every provider, UI interaction, Git version or file in the crate has been exhaustively tested. GitLab/Bitbucket merge and query construction received an additional source pass; complete wire parity, browser flows and provider outage behavior remain for subsequent coverage.

## Confirmed findings and fixes

### G03-1 — Blocker, fixed: a failed branch probe can authorize destructive provisioning

Location: `crates/otto-git/src/local.rs:2052` (`worktree_add_if_absent`), with fallible boolean probe at `branch_exists`.

Trace: existing `agent` points to commit B; caller requests base A; the branch existence subprocess fails, so `branch_exists` returns false. The previous else branch called `worktree_add`, whose `git worktree add --force -B agent path A` resets `agent` from B to A. The method explicitly promises that previous agent commits survive. A second process creating the branch after the probe has the same failure shape.

Reproduced the exact Git command sequence in a throwaway repo: preservation assertion was false before the fix. Changed the fresh-creation path to `git worktree add -b`, which fails if the branch exists and cannot reset it. Existing branch reattachment/reuse remains unchanged.

Regression: `worktree_failed_branch_probe_never_resets_existing_branch` (`local.rs:6066`) injects a failing branch probe while using real Git for the rest. It requires an error, unchanged branch SHA and no new worktree. The old implementation would return success and change the branch SHA. Existing reattachment and multi-turn preservation tests remain relevant.

### G03-2 — Major, fixed: applying a SHA-selected stash could apply a different stash

Location: `crates/otto-git/src/local.rs:4105` (`stash_apply`).

Trace: requested stash A resolves to `stash@{0}`. Another process pushes B before apply executes. Executing `stash apply stash@{0}` applies B, even though the request carries A's SHA and the API promises identity stability.

Reproduced with a throwaway repo: old selector applied the wrong file content. The fixed method still validates list membership but applies the immutable SHA directly. A subsequent renumbering cannot substitute another object.

Regression: `stash_apply_keeps_sha_identity_when_stack_is_renumbered` (`local.rs:6090`) injects a real extra stash immediately before the apply subprocess. It asserts A's exact content is applied and both stash entries remain. Returning a success code or merely counting stashes would not catch the old bug; the content assertion does.

### G03-3 — Blocker, partially fixed: explicit Drop still uses a mutable selector

Current location: `crates/otto-git/src/local.rs:4117` (`stash_drop`).

Trace: resolve requested stash A to an index, concurrent push renumbers that index, then `stash drop` removes B. The route's process-local per-repo mutex does not coordinate external CLIs or linked worktrees sharing the stash reflog. Fixture Git commands confirmed wrong-entry deletion behavior. Explicit Drop remains available and this defect remains open.

The associated automatic restoration and capture defects are fixed. `stash_save_unique` (`local.rs:4188`) uses a unique marker rather than a racy read of `refs/stash`; `pop_stash_sha` (`:4212`) now applies the immutable SHA with `--index`, retains its recovery backup, and never deletes an entry. Checkout's duplicate restoration path was updated too. The exact-content/index preservation regression at `local.rs:6120` inserts another stash between capture and read and verifies both saved entries survive restoration unchanged.

Do not close explicit Drop with a second preflight SHA check: it only narrows the same race. See `git-stash-identity-options.md` for the native lock-backed proposal, crash-consistency limitations and independent-review gates. No custom Git ref writer or new dependency was added.

Required deletion regression: inject an extra stash between lookup and destructive invocation; assert the requested stash alone is removed and the concurrent entry remains byte-for-byte recoverable. Include shared linked-worktree callers, newest/oldest/middle/last entries and failed native writes.

## Security handoff (not counted as a proven exploit in this review)

`http.rs` `optional_token` checks ownership but does not perform the origin/provider checks used by `provider_ctx_with_account`. Fetch/push receive that token through host-agnostic AskPass setup. Sent root a focused request for the security reviewer to trace changed origin/pushurl/pushRemote/URL rewrites and credential destination confinement. Do not interpret this handoff as a completed exploit test.

## Verification

- Real Git fixture reproductions: both original invariants failed before production edits.
- Real Git fixture checks of replacement primitives: `-b` refused an existing branch and preserved its SHA; SHA apply restored the requested content after stack renumbering.
- `rustfmt --edition 2021 crates/otto-git/src/local.rs`: passed.
- Added two Rust integration-style fixture tests in the existing test module.
- `cargo test -p otto-git --lib`: **393 passed, 0 failed, 1 ignored**. Clippy was attempted but blocked by concurrent credential migration compile errors (details below). A final stderr-conflict-output correction awaits its focused recheck after that migration compiles.
- No browser screenshots or real hosted-provider mutations performed.

## Next pass

1. Close explicit Drop in G03-3 with a concurrency-safe identity operation, run its regression tests, and repeat final lint/focused checks on the settled checkout.
2. Complete account credential destination investigation with the security lens.
3. Exercise the Git UI's real stage/discard/conflict/retry flows in isolated fixtures; verify provider wire parity and worktree unusual-path behavior before raising the score.

## Native atomic-operation investigation

Git 2.46 fixture probes confirmed that `git reflog delete` does not invoke a rejecting `reference-transaction` hook, so that hook cannot validate the selected identity while Git holds its lock. A `refs/stash@{SHA}` argument is not a supported identity-deletion primitive (fixture returned success without deleting the matching entry). Manual reflog/ref rewriting was proposed but rejected as introducing durability/backend risks; no such writer was added. Root approved immutable-SHA automatic restoration with retained backups; it is implemented. Explicit Drop remains functional and requires the separate native identity-safe design; disabling it was rejected.

## Additional provider pass — G03-4 (minor, fixed)

Locations: `crates/otto-git/src/providers/github.rs:24`, `gitlab.rs:24`, `bitbucket.rs:25`.

All three providers had a partial query encoder that escaped spaces and selected delimiters but left plus and percent literal. Input `C++ 100%25` became `C++%20100%25`, which a standard query decoder reads as `C   100%`. This changes repository/user search meaning before the server sees it. Verified the failing roundtrip with a standard URL query parser. Replaced the three partial encoders with the crate's existing `urlencoding::encode`. Each module now has `query_encoding_preserves_literal_plus_percent_and_unicode`, which decodes the final URL's query value and asserts exact equality for plus, percent, Unicode and delimiter characters. Rustfmt passed; these three new tests also await the coordinated Cargo run.

Provider source traces also covered GitLab native merge SHA pin/rebase polling and Bitbucket uncached preflight head comparison. Bitbucket's read-then-merge cannot provide an atomic expected-head guarantee; this is a provider capability boundary, not silently counted as proven atomic safety.

## End-of-pass update

Automatic stash capture/restoration is now fixed in addition to the first three defects: `stash_save_unique` identifies the created entry by a unique marker; successful and failed-checkout restoration applies the captured full SHA with `--index` and retains the backup. The duplicate checkout restore path uses the same immutable-object rule. Exact-content/index and preservation-of-both-entries regression added (`automatic_stash_capture_and_restore_preserve_identity_and_backup`). Existing successful-restoration tests now assert retained recovery backups. API docs rows 45/46/merge were updated; root assigned checkout toast copy to a separate UI owner.

**G03-3 is narrowed to explicit `stash_drop`, which remains functional and still needs an identity-safe deletion implementation.** See `git-stash-identity-options.md` for the native transaction proposal and required independent concurrency/durability review. The earlier description of auto-pop as unresolved is superseded by this update.

Verification after these changes: `cargo test -p otto-git --lib` completed with **393 passed, 0 failed, 1 ignored** (30.51 seconds). Initial regression fixture failure was repaired: the test shim must skip Git's leading `-c` arguments to inject its race. The old handler assertion expecting zero backups was updated for the authorized retention contract. Logs: `/tmp/otto-git-review03-tests.log`.

`cargo clippy -p otto-git --all-targets -- -D warnings` was attempted but encountered 12 E0308 errors during reviewer E16's concurrent transport-credential signature migration in `http.rs`; this is a moving-checkout result, not a clean lint pass. Log: `/tmp/otto-git-review03-clippy.log`. One final tiny correction includes stderr conflict markers in auto-restore's returned output; its focused recheck is pending until the concurrent migration compiles. Cargo slot released to root.

Current provisional score remains **8.2/10** because explicit stash deletion identity is unresolved and no UI/provider end-to-end validation has been completed. This report does not claim 9.8 or product-wide completion.
