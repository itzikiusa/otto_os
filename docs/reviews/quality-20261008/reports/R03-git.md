# R03 — Git, PRs, code review and issue integration

Reviewed snapshot: `a0bd718b9fbc008d72c164ce643a24ed78e6368c` (PR #94 merge), main parent `196048df5bbd55a691b093ea0a3fa456f0912674`, branch `review/quality-20261008`, 2026-10-08. The owned Git/review/issues crates and Git UI have no changes in this merge relative to the main parent. **All five findings below are existing defects, not demonstrated PR #94 regressions.** Concurrent foundation/session/context edits are outside this verdict. Production code was not edited.

Verdict: material corrections are needed before this domain supports 9.8. Five confirmed source-traced defects; two have lightweight execution evidence; one additional draft-ownership suspicion was falsified by a browser journey. No claim of an approval-policy bypass, comprehensive browser coverage, or measured large-repository performance is made.

## Repair handoff (subsequent authorized implementation)

The findings and source lines below describe the reviewed merge snapshot. The coordinator subsequently authorized production fixes. All five are now implemented in the working tree; **the final force-push Rust code and later HTTP regression still await the coordinator's Cargo gate**. No commit, push, deployment or real remote mutation was performed. Scores remain the review assessment until the verification cycle closes.

| Finding | Working-tree correction | Regression evidence / status |
|---|---|---|
| R03-01 | New `otto-git/src/push.rs` and local-only `GET /repos/{id}/push-target`. The UI captures source OID/ref, destination/ref/URL hash and observed remote OID before the first push; force requires this snapshot. Final argv uses immutable SHA:destination and explicit remote OID lease. A bounded branch-reflog reachability check preserves integration protection, since Git ignores `--force-if-includes` with an explicit OID lease. Missing/changed target returns 409. Single-ref upstream/current/simple modes supported; ambiguous/multiple refs, mirror and URL-rewriting configurations refuse force retry. Normal push remains compatible. | Production `runPush` unit regression failed before the fix and now passes. Five `push_tests.rs` cases prepare/assert command arguments only: non-origin upstream, post-preflight checkout/source movement, source/destination/tracking changes, unintegrated fetched commits, own rewrite, multi-ref and chained rewrite refusal. These Rust cases await execution. HTTP no-target refusal regression also awaits execution. |
| R03-02 | Origin reconciliation now runs before hosted provider resolution and after origin API edits. Review/readiness server resolver reuses the same Git resolver. Collaborator reads reconcile before cache lookup; remote changes clear collaborator cache entries, and origin UI edits invalidate PR caches. | `provider_ctx_tracks_changed_and_removed_origin` failed with `old != new`, then passed. It verifies external-origin edit, persisted URL, provider mismatch refusal and origin removal. Additional actual remotes-route persistence regression awaits Cargo. Existing HTTP tests moved unchanged to `http_tests.rs` to respect the LOC ratchet. |
| R03-03 | GitHub approval and reviewer outputs derive from one chronological unique-reviewer reduction. Comments/pending events preserve opinion; changes requested/dismissal replace it. | Wire fixture failed with `[alice,bob,bob,dan]`, then passed with `[bob,dan]`; reviewer booleans also asserted. |
| R03-04 | Jira walk keys include normalized base URL, credentials and JQL. | Two HTTP fixture sites with identical credentials failed by returning `B-25` to A, then passed A0/B0/A25/A100. Four Jira walk tests passed. |
| R03-05 | Ref updates track changed old tips; retained tails are invalidated for removed refs, moved tips outside the checked prefix, and moved tips displaced below the fresh page by unrelated new commits. Normal prefix growth keeps the splice path. | Both the old-side-branch and displaced-prefix cases were observed failing before correction; graph suite now passes seven tests. |

Final verification handoff:

- `npm run check` passed both before and after the final graph/push-copy followups (0 errors/warnings); final output `/tmp/otto-r03-ui-check-final.log`. `npm run test:unit` passed **1,716/1,716**, no skips; `/tmp/otto-r03-ui-unit.log`. Focused `node --test unit/gitGraphSplice.test.ts unit/gitPushTarget.test.ts` passed **8/8** after those followups.
- Rust observed red→green commands: `cargo test -p otto-git --lib approval_history_reports_effective_unique_reviewers -- --nocapture`; `cargo test -p otto-git --lib provider_ctx_tracks_changed_and_removed_origin -- --nocapture`; `cargo test -p otto-issues --lib jql_walk -- --nocapture` (four passed). Formatting, `git diff --check` and `python3 scripts/loc-ratchet.py` pass. No LOC baseline was raised.
- Coordinator queue: `cargo test -p otto-git --lib push::tests -- --nocapture`; `cargo test -p otto-git --lib remote_edit_route_persists_origin_and_force_requires_a_target`; then Git/issues library suites and `cargo clippy -p otto-git -p otto-issues --all-targets -- -D warnings`. To honor the strict no-force-push execution constraint even for old tests, exclude the legacy `force_with_lease_replaces_our_own_rewritten_history` test, which actually force-pushes a temporary bare repo; the new command-plan regressions do not execute a push. Existing other push tests use fixture-owned repositories. Run server `--test it route_inventory` and `--test it policy_coverage` plus appropriate review consumers to verify the shared resolver/new endpoint. Route documentation and TypeScript types are updated; the inventory/policy tests discover the route automatically.
- Still unverified: real/native force dialog journey, realistic performance workloads, complete dark/mobile/accessibility coverage, final Rust compile/clippy and affected server tests. The immutable argv fixes proven checkout/source-ref retargeting; it is **not a claim that arbitrary external Git configuration changes after the final read are impossible**. Existing URL rewrites fail closed, but a new rewrite introduced concurrently after preflight is a remaining Git configuration race. The integration guard deliberately fails closed if needed reflog history is older than the last 256 entries. PR browser cache invalidation is immediate for UI remote edits; external changes are reconciled at hosted server calls, and already-rendered browser state can remain stale until its normal refresh.

## Scores and confidence

These are bounded engineering assessments, not a calculation from test counts or inherited CI results.

| Vertical | Score / 10 | Confidence | What prevents 9.8 |
|---|---:|---|---|
| Correctness / bugs | 7.5 | Moderate overall; high for the five traced paths | Push consent can follow a different branch, stored forge identity can diverge from origin, approval state and Jira cache identity are wrong, graph reachability can be stale. Regression fixtures and affected-consumer checks remain required. |
| Performance | 8.3 | Moderate-low | Strong source-level bounds, caching and virtualization; no representative graph/diff/provider/Jira workload measurement in this assignment. Tail-splice optimization has a correctness hole. |
| Design | 8.0 | Moderate | Useful Git command, provider and review-core boundaries. Repo/remote identity is resolved in separate layers with inconsistent freshness; graph reachability decisions span ref updates, fetching and tail splicing. These boundaries need explicit invariants. Broad rendered design coverage is incomplete. |
| UX / usability | 7.7 | Moderate-low | Force confirmation can name the wrong eventual target; approvals can mislead; changing origin does not align hosted actions. One real draft-discard/navigation journey passes, but native, mobile, dark, keyboard/AT and recovery journeys were not exercised comprehensively. |

## Confirmed findings

### R03-01 — High: force-push consent does not bind the branch, commit or destination

**Source:** `ui/src/modules/git/pushFlow.ts:46`, `:67`, `:72`; caller `ui/src/modules/git/GitToolbar.svelte:146`; server `crates/otto-git/src/http.rs:1881`, `:1904`, `:1911`; command construction `crates/otto-git/src/local.rs:3451`, `:3469`, `:3499`.

**Trigger:** push branch A, receive a non-fast-forward rejection, then another session or external Git command checks out branch B while the rejection dialog is open. Choose “Force push with lease.” The dialog at `pushFlow.ts:20–26` still describes A and its captured upstream, but the retry body contains only `force_with_lease: true`. It carries no branch, expected HEAD, source SHA, remote or destination ref. `repo_push` passes `None` for branch; `push_with` constructs a plain current-branch `git push --force-with-lease --force-if-includes`. B can satisfy both protections if its remote history was previously integrated before a rewrite. The issue is target consent, not a bypass of Git's remote-race protection.

**Evidence:** executed the production TypeScript `runPush` via TypeScript transpilation/VM with stubbed API and confirmer. The first request was `{}` while the fixture was on `feature-a`; confirmation switched the fixture to `feature-b`; the retry was `{force_with_lease:true}`, followed by a “Force pushed / origin/feature-b” success notice. Artifact: `/tmp/otto-r03-force-push-identity.json`. The backend chain above establishes which real command that unbound request selects. No real force push or remote mutation was performed.

**Consequence:** a consequential rewrite may affect B although the user approved replacing A. A normal concurrent commit can also change the source content after the displayed decision. The existing remote lock serializes Otto network operations, not external CLI checkout/ref/config changes.

**Contract:** `ui/src/lib/api/types.ts:3026–3034` and `docs/contracts/api.md:111` deliberately make an empty body push the current branch. The current contract therefore cannot express the identity to which the confirmation applies.

**Correction:** carry a captured source commit/ref and destination remote/ref (including the expected remote state) through the decision and mutation. Reject changed identity with a recoverable conflict or require a new confirmation. Construct the final command from immutable source/destination values so an external checkout after a preflight check cannot redirect it. Preserve configured upstream semantics: merely supplying the existing `branch` field is insufficient because its explicit-branch path currently chooses `origin` (`local.rs:3497`). Update API, types and contract together; keep both current force protections or their equivalent explicit-lease semantics.

**Regression fixture:** exercise the production flow and handler with a fixture-owned repo, two branches and a non-origin upstream. Change checkout and then source ref while confirmation is pending; assert an expected conflict or a command pinned exactly to the confirmed source/destination, with the unrelated branch untouched. A mocked process sink can prove argv without any push. Include external movement after preflight, not just a synchronous check before an await.

### R03-02 — High: editing origin does not refresh the repository's forge identity

**Source:** `crates/otto-git/src/ops.rs:482–485`; `crates/otto-git/src/http.rs:386–391`, `:405–421`; UI `ui/src/modules/git/RemotesPanel.svelte:55–70`.

**Trigger:** register and bind a repository with origin `https://github.com/team/old.git`, then edit origin to `https://github.com/team/new.git`. The remote operation changes `.git/config` and returns live remotes but never calls `set_repo_remote`. Subsequent provider resolution rereads disk only when the stored provider is `None`; an existing GitHub repository uses its old `remote_url` indefinitely. Removing origin or changing it with an external CLI has the same stale-snapshot problem. Account rebinding happens to refresh it (`http.rs:1393`), but ordinary remote edits do not.

**Evidence:** traced remote mutation, persistence helper (`http.rs:1430–1441`), every `refresh_remote` caller, provider detection and UI result handling. Local fetch/push use the live Git configuration; hosted PR operations obtain the old `RemoteRef` from the persisted URL. The review/readiness resolver in `crates/otto-server/src/modules.rs:1435` is another consumer of stored repository identity and needs the same invariant. This is source-confirmed; no hosted mutation was attempted.

**Consequence:** PR listing and outward PR actions can target the old repository while the local Git workspace now pushes to the new one. A host/provider change also produces confusing account validation or stale credentials selection. A success toast for editing the remote does not explain this split identity.

**Correction:** reconcile the canonical live origin before hosted operations and after origin mutations; update the stored snapshot, invalidate repo-scoped PR/collaborator state, and revalidate account/provider/host compatibility. Share this resolver between the Git crate and server review consumers. A mutation-only refresh does not cover external CLI changes.

**Regression fixture:** isolated temporary Git repo plus stub provider: register old origin, change it through the remotes endpoint, then request a PR and assert the provider receives the new owner/repository. Repeat with an external `git remote set-url`, origin removal, and a provider/host change. Never send an actual publication.

### R03-03 — Medium: GitHub approvals count historical events rather than effective reviewers

**Source:** `crates/otto-git/src/providers/github.rs:771–775`, reviewer reducer at `:777–794`; consumers `ui/src/modules/git/PrDetail.svelte:506–508`, `crates/otto-server/src/modules.rs:3544–3547`, `ui/src/modules/git/PrMergeModal.svelte:233–237`.

**Trigger:** Alice approves then requests changes; both chronological reviews remain in the API response. `approved_by` includes the old approval although the independently reduced reviewer entry reports not approved. Two approval reviews by Alice count twice. Conversely, the separate latest-any-review reducer lets a later COMMENTED entry erase the reviewer's approved flag even though it is not a new approval opinion.

**Evidence and affected decisions:** the PR header renders a green approval chip from this vector; the server reports its raw length as readiness approvals; the merge dialog displays that count. This is a misleading readiness/display result. **It is not an established merge authorization bypass:** `PrMergeModal.svelte:107–117` gates on CI failure, mergeability, open blocker findings and unavailable readiness, not an approval threshold; the forge also controls its merge policy.

**Consequence:** users receive contradictory or inflated review status at the point where they decide whether to merge. The duplicate name vector also violates the intended unique-reviewer representation.

**Correction:** reduce review history once into each reviewer's effective opinion, respecting replacement, dismissed reviews and comment-only events; derive both `reviewers` and `approved_by` from that result. Preserve timestamps/avatars separately from opinion when needed.

**Regression fixture:** a production mapper/provider fixture containing APPROVED→CHANGES_REQUESTED (zero approvals), repeated APPROVED (one), DISMISSED approval (zero), and APPROVED→COMMENTED (approval retained). Verify both output fields and the readiness count. The dismissed fixture should mirror GitHub's returned review state, not assume dismissal is always a separate appended event. Existing mapping tests were read; this state-transition oracle is missing. No Rust test was run in this assignment.

### R03-04 — High: Jira pagination cache aliases distinct sites sharing credentials

**Source:** `crates/otto-issues/src/jira.rs:87–92`, `:355–364`, `:531`, `:538–554`, `:602–603`.

**Trigger:** sites A and B use the same Atlassian email/token and the same JQL. A fetches offset 0; B fetches offset 0 and replaces the shared walk. A asks for offset 25. The cache key hashes only Authorization and JQL; Authorization encodes email/token, not site. A can receive B's cached summaries and URLs without making an A request. A subsequent page can instead send B's continuation token to A.

**Evidence:** traced constructor, cache key, fresh-search reset, cached-page return, token resume and page recording. The HTTP client cache correctly includes `base_url`; the separate walk cache does not. The comment claiming the auth header encodes the base URL is false. The key test at `jira.rs:2738` distinguishes credentials/queries but not sites with identical credentials. Contract `docs/contracts/api.md:1776` describes memoization by account/JQL; a site is part of that account identity.

**Consequence:** cross-site search result contamination and broken “load more,” with wrong issue links presented as results for the selected site. This finding does not assert access to a site the user lacks credentials for.

**Correction:** include normalized base URL in the walk identity alongside credentials and exact JQL; apply it consistently to lookup/reset/resume/record. Preserve existing TTL/page/walk limits.

**Regression fixture:** two local HTTP stubs, identical email/token/JQL, distinct issue prefixes and continuation tokens. Run A0, B0, A25 and A100; assert A results remain A and A never receives B's token. A key-only unit test is useful but insufficient to verify the cache consumers.

### R03-05 — Medium: a same-name ref rewind leaves unreachable commits in the cached graph tail

**Source:** `ui/src/modules/git/GraphView.svelte:377–396`, `:843–859`, `:1044–1068`; `ui/src/modules/git/graph-splice.ts:15–29`.

**Trigger:** the user has loaded more than the first 2,000 commits. An old side branch whose unique commits lie below that page is reset/force-pushed to an earlier ancestor without changing its name. The first page remains identical. `historyMoved` notices the SHA change and reloads, but `lostRef` compares names only. The successful prefix splice retains the obsolete tail; the full-reload guard only runs for a disappeared ref name.

**Evidence:** production `spliceHistory` was transpiled and executed with a fresh 2,000-row prefix and an old tail containing `old-branch-tip, base`. After the same-name rewind removes reachability of `old-branch-tip`, it still returns 2,002 rows instead of the expected 2,001. Artifact `/tmp/otto-r03-graph-splice-repro.json`. The synthetic fixture proves the splice result; the source trace establishes the ref-update route to it. A real Git-history/browser reproduction remains to be added.

**Consequence/cost:** the graph displays a commit no current ref reaches; `skipCursor = g.commits.length` advances over nonexistent rows and can skip valid history on later paging. This is a correctness failure in an otherwise valuable optimization from O(loaded history) to O(first-page + retained tail handling) network refresh work.

**Correction:** invalidate/revalidate retained history when a moved ref can remove reachability outside the fresh prefix, including tag moves. A backend ancestry/reachability classification can retain the fast path for verified fast-forward growth. Do not treat immutable commit objects as proof that membership in `log --all` is immutable.

**Regression fixture:** temporary Git history with >2,000 recent commits and an old side branch; load the tail, rewind the old branch without renaming it, refresh, and compare loaded SHAs and the subsequent pagination cursor with a fresh `git log --all`. Keep a fast-forward fixture to protect the intended optimization.

## Coverage and separate review passes

The correctness, performance, architecture and test-review skills were read and applied as separate lenses. The UI design guidelines and review checklist were read. This was a domain-wide risk-directed review, not a claim that every line or visual state in this large surface was examined equally deeply.

| Surface | Inspected implementations, callers and invariants | Remaining limits |
|---|---|---|
| Git mutations and recovery | `local.rs`, `ops.rs`, `http.rs`, `recovery.rs`: stage/unstage/discard, literal paths, staged renames, patch fingerprints, checkout/autostash, merge/abort, rebase/bisect, recovery plans, push/fetch/remote locks; relevant embedded tests and API/type contracts | Tests read but not run; no exhaustive combination of submodule, filesystem, concurrent external Git or crash/restart behavior |
| Hosted providers and accounts | GitHub/GitLab/Bitbucket mappings and merge paths; provider/account resolution, host checks, SHA pinning, comment/thread/check pagination, collaborator cache, PR UI/readiness | Live forges not called; GitHub transition fixtures pending; not every provider endpoint executed |
| Review and findings | `otto-review` pure core, budgets/lenses/prompts/parsing/fallback; server review orchestration seams in `modules.rs`; assigned `finding_agent.rs`, `finding_context.rs`, `repo_directory.rs`, `routes/findings.rs`, `routes/repo_rules.rs`; per-attempt cancellation, session publication, result parsing, partial results and scope | No real agent spawned; not a proof of all cancellation interleavings or every finding action |
| Issues | Jira/Confluence clients and routes, account ownership, host/token changes, JQL token pagination, update conflict/content checks and relevant test definitions | No live issue update; cross-site cache HTTP fixture pending |
| Graph, diff and background work | GraphView ref/log refresh, splice, graph cache/virtual window; shared DiffViewer virtualization/highlighting, diff/status/HTTP caches; git store active/background polling and fetch ownership | No large-history or huge-diff latency/memory benchmark; no native rendering run |
| Git UI | Toolbar push/pull, remotes, PR detail/create/merge/comment and review flows, WIP/conflict/recovery affordances and draft leave path; loaded/empty/error/retry source patterns | GitAccounts, FocusView, FindingsBoard and remaining component inventory received less depth; not every loading/error state rendered; full light/dark/mobile/RTL/focus/AT coverage unverified |

Positive preservation evidence includes literal path handling and raw patch validation; abort paths guarded by an operation in progress and `reset --merge` for squash cleanup; autostash restoration/recovery rather than unconditional destructive reset; expected HEAD/base validation in recovery; and both force lease and include protections. GitHub/GitLab merge paths use expected source SHA. Fork review checkout verifies source identity and cleans up its own refs/worktree. Review orchestration uses per-attempt cancellation, bounded reviewer concurrency and deterministic fallback rather than treating missing summarizer output as an empty successful review. These are source-supported strengths, not execution certifications.

The architecture pass found the most useful corrective boundary at repository target resolution: local Git config, persisted Repo, account selection, provider RemoteRef and UI confirmation need one explicit identity lifecycle. The origin and force findings show the concrete cost of today's split responsibilities. Graph fetch/ref/splice ownership similarly needs a reachability invariant at its boundary. No separate taste-only refactor finding is added.

## Performance evidence and limits

Source inspection found substantive bounds: desktop/mobile graph virtualization with overscan; first-page reload and chunked history paging; a four-repository graph cache with per-repo row cap; byte-bounded diff/provider caches; status single-flight/memoization; bounded watcher count/idle expiry; background fetch concurrency of two with active/inactive ownership; deferred syntax highlighting with a short time slice and bounded queue; Jira walks limited by TTL, walk/page counts and per-request page cap. These address real repeated-I/O and DOM cost paths.

No central workload was timed or profiled in this assignment. The 2,000-row splice fixture is a correctness case, not a benchmark. The browser test duration below is also not a performance result. To support 9.8, measure named graph and huge-diff specs at their intended cardinalities, retained-history refresh under ref moves, visible/hidden-tab request rates, cold/warm provider loads and long Jira pagination using representative network delay. Record CPU/heap/request counts and renderer/device; protect data truth while optimizing.

## Executed verification and falsification

- Two isolated lightweight executions of production TypeScript: force-push flow identity and graph splice, with artifacts above. The API/confirmer were stubs for force flow; graph data was synthetic. No destructive Git operations or production daemon access.
- Added `ui/e2e/desktop-git-pr-draft-ownership.spec.ts` under explicit test-only authorization. It creates a temporary repository/workspace in an isolated daemon, stubs forge PR responses, edits PR 1, follows real hash navigation and leave confirmation, discards, and verifies PR 2's editor starts with PR 2 content. **Passed: 1 test; 4.0 seconds total.** The first attempt failed on an incorrect test locator before reaching the behavior; the corrected locator passed. This falsifies the suspected draft leak in the actual shell lifecycle; the candidate is withdrawn.
- Screenshot inspected: `/tmp/otto-r03-pr-draft-results-v2/desktop-git-pr-draft-owner-c5573-er-PR-clears-its-edit-state-desktop-browser/pr-two-after-discard.png`. Light desktop, 1280×800: PR 2 title/description present, editor closed, comment destination/audience copy visible. It does not establish dark/mobile/native accessibility quality.
- No mutation testing was performed. The new E2E asserts the cross-PR values and visibility through the production shell, so it would detect persistence of the prior editor content; forged PR payloads remain fixture-owned. No Cargo, broad UI unit/type/build gate, full Playwright suite, benchmark, live provider, deployment or publication was run by R03. Existing CI totals are baseline only and are not counted as fresh verification here.

Command (from `ui/`):

```sh
OTTO_E2E_SLOT=33 OTTO_E2E_PORT=7833 OTTO_E2E_PW_PORT=5233 \
OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto_os/target/debug/ottod \
npx playwright test e2e/desktop-git-pr-draft-ownership.spec.ts \
  --project=desktop-browser --workers=1 --output=/tmp/otto-r03-pr-draft-results-v2
```

Environment: macOS workspace, local Node v22.22.3 (CI Node 26.10.0), existing merged-source debug daemon supplied for the review, isolated slot 33. Rust 1.99.0 installed but no Rust build/test invoked. Browser lease returned to coordinator.

## Suggested verification queue after correction

Keep Cargo and browser leases coordinated. Add the focused fixtures described above before broadening checks. Run relevant `otto-git` and `otto-issues` library tests, then affected server consumers through the `it` integration target. API changes to push require type/contract and UI checks. Named browser candidates are `desktop-git-pr-merge-modal`, `desktop-git-recovery`, `desktop-git-graph-perf`, `desktop-diffviewer-huge-perf`, `review-cancel` and the new PR draft test; select only those justified by the final fixes. Cancellation/review-result integration and representative graph/diff workloads remain priority evidence gaps even if the five targeted regressions pass.
