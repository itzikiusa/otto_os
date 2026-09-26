# Round 3 — Git independent UX review

Workspace: `/Users/itziklavon/otto-ux-audit-20260925` only. Read protocol, AGENTS, design README/checklist, prior R1/R2 reports, and parent notes. Used systematic-debugging/test-first workflow. Viewed the actual supplied 2048px screenshot `/Users/itziklavon/Library/Application Support/Otto/snips/01M3CWV66CE938YZ90051DDXHW.png`. No other checkout/branch edited, no commits/subagents, no real forge publications or user-repository mutations. Parent authorized scoped `otto-git` normalization after the collision reproduction; public API shapes unchanged.

## Findings repaired

1. **P2 — Five long repository/branch tabs still truncated with free header space.** `ui/src/modules/git/GitTabs.svelte:248,306`. Browser at 2048×900, five names (`promotions-service`, `bo_common_ui`, `cs3-platform`, `koala-backoffice`, `go_dependencies`) each on `feature/customer-experience-2026`: old 280px tab and 120px branch caps truncated identity with **519.9px unused** after the plus button. Reproduced before changing CSS and viewed screenshot. Desktop tabs now size to their actual content; excess tabs scroll inside the existing tablist while plus/auto-fetch stay pinned. Phone/tablet caps retained. A separate short-tab test verifies no unnecessary stretching. The supplied eight-tab overflow regression was independently rerun, not treated as proof of this five-tab variant.
2. **P2 — A local `origin/collision` and the actual remote `origin/collision` lost their distinct graph identities.** `crates/otto-git/src/local.rs:1215,1362,1372`; `ui/src/modules/git/GraphView.svelte:2061,2124,2362,2392`. Actual disposable Git reproduction: `%D` emits the same text twice; `refname:short` expands the local name to `heads/origin/collision`. The graph could show the local branch as remote and choose the wrong action collection. Local branch and merged-membership names now use `refname:lstrip=2`. The graph indexes the existing typed RefBranch `name`, `remote`, and `sha` metadata once, preserves that identity for context/checkout actions, and keeps tag/stash/detached decorations plus older-daemon fallback. Rust regression failed before the normalization and passed after; full crate passed. Browser regression checks local Rename versus remote Checkout at distinct tips and co-located tips. No wire schema/type change. Traced `upstream:short`: remaining callers only display/copy it and test suffix; no lookup against normalized remote names, so no speculative alteration.
3. **P2 — Pending comment and request-changes sends accepted text edits that completion would discard.** `PrDetail.svelte:161,254,531,563`. Held mocked POSTs; fields remained enabled (request-changes Cancel too). Inputs/Cancel now lock while their send is pending; duplicate sends are guarded. Failed sends keep the original draft; regression retries successfully and checks actual outbound body.
4. **P2 — Completion after leaving a PR read destroyed reactive identity.** `PrDetail.svelte:231`; `PrMergeModal.svelte:25,104`. Held comment/merge POST, navigated to PR 2, completed PR 1. Browser captured `derived_inert` warnings (two for merge); next-PR draft and title need to remain stable. Disposed components now skip reload/callback writes; merge notifications retain the captured PR number even after dismissal. Matching mutation finalizers/approve/resolve/request-changes callbacks guard disposal too. Regression checks no warnings and unchanged PR 2.
5. **P2 — Reply publication failure had no visible recovery feedback.** `CommentThread.svelte:25,55,102`. A mocked 503 left the reply textarea but displayed no error and leaked an unhandled rejection. Reply now shows inline role=alert feedback, preserves draft, and supports a successful retry. Pending reply controls lock; textarea has an explicit accessible label; shared leave guard protects route navigation with a reply draft.
6. **P3 — Review verdict exposed `request_changes` transport text.** `ReviewPanel.svelte:1114`. Seen in actual Warm dark RTL phone screenshot. Rendered verdict separates underscore words; no isolated implementation-mirroring test added for this small copy correction.

## Independent old-fix checks / negative findings

- Initial current-release run: **44 passed (2.4m)**, `/tmp/otto-ux-r3-git-prior.log`, `/tmp/otto-ux-r3-git-prior-results/.last-run.json` passed. Includes every R1/R2 test, supplied header/add-menu tests, all supplied close-tab/Always-delete tests, and blocked/override merge-modal test.
- Graph nested context menu: arrows and Escape belong to the menu; first Escape restores its action button and leaves the refs dialog open; second Escape closes the dialog and restores the expander. No shared focus change needed.
- Prior late diff and PR pagination races, inline diff/account/detail Retry, Focus account/issue selection races, phone dialog ownership, local/remote browsing races, add-mode keyboard behavior, title-save locks, PR identity guards, review outward confirmation, RTL code gutters, long refs, stash/submodule/stale worktree menus all rerun.
- Added pre-existing graph lane tests and locked/unknown worktree removal fixture to final run. Worktree remove is intercepted; it must send force=false. No actual user's worktree removal.

## Verification

Common Playwright environment: `OTTO_E2E_SLOT=ux3git OTTO_E2E_PORT=7855 OTTO_E2E_PW_PORT=5355 OTTO_E2E_SWEEP_ORPHANS=0`, run from worktree `ui/`, `--workers=1`. Only one Playwright process uses that slot at a time. Fixture specs block service workers (production worker behavior is not claimed).

- Initial 44-test command used `OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/release/ottod` and `npx playwright test e2e/desktop-ux-git.spec.ts e2e/desktop-ux-r2-git.spec.ts e2e/desktop-git-add-menu.spec.ts e2e/desktop-close-tab-ends-session.spec.ts e2e/desktop-git-pr-merge-modal.spec.ts --project=desktop-browser --output=/tmp/otto-ux-r3-git-prior-results --workers=1`.
- New regression red evidence: `/tmp/otto-ux-r3-git-red.log`, `red-results`, `red2.log`, `red2-results`. First header attempt incorrectly assumed restoring localStorage would open all five tabs; it opened one. Fixed fixture to use the real plus picker; then the intended 519.9px-gap failure reproduced. No product claim is based on that initial fixture failure. Other red findings have actual enabled-field/inline-alert/disposed-read evidence.
- Intermediate 8-test run: `/tmp/otto-ux-r3-git-green1.log`: 6 passed, collision failed against unchanged old release binary, delayed merge exposed the new callback issue subsequently repaired.
- Rust red: `cargo test -p otto-git refs_keep_local_names_when_remote_names_collide`, `/tmp/otto-ux-r3-git-rust-red.log`, expected failure.
- Rust green/full: `CARGO_BUILD_JOBS=3 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test -p otto-git` — **236 passed**, `/tmp/otto-ux-r3-git-rust-full.log`.
- Parent rebuilt `target/debug/ottod` with the changed Rust; `/tmp/otto-ux-r3-debug-daemon.log` exit 0. Final browser command uses that binary and adds `e2e/desktop-ux-r3-git.spec.ts e2e/desktop-git-graph-lanes.spec.ts e2e/desktop-git-worktree-status.spec.ts` to the initial command, output `/tmp/otto-ux-r3-git-verified-results`, log `/tmp/otto-ux-r3-git-verified.log`. **57 passed (3.6m)**; `.last-run.json` passed with zero failures.
- `npx tsc -p tsconfig.e2e.json` and `node scripts/ui-guards.mjs` passed; `/tmp/otto-ux-r3-git-tsc.log`, `/tmp/otto-ux-r3-git-guards.log`. No baseline increase. Scoped git diff --check passed. Parent owns full check/build; parent caught one Svelte null-narrowing issue in `haveTypedRefs`, repaired using the established explicit local annotation.

## Rendered inspection and dimensions

Fresh initial-run evidence was viewed, not accepted from old reports:

| Page family/variant | Dimensions / themes inspected | Evidence |
| --- | --- | --- |
| Git header + long identities | 2048×900 Native light; supplied 2048px reference | red2 and green1 `five-long-tabs.png`, original Snip |
| Graph + loaded commit diff | 1440×900 Native light; 820×1180 Native light RTL | prior `desktop-ux-git-graph-visual-review-*` PNGs |
| Collapsed 30-ref dialog | 1100×500 Native light RTL | prior `graph-many-refs.png` |
| PR summary/conversation | 1440×900 Native light and Pro Dark RTL | prior loaded scenario `pr-summary.png` |
| PR files long paths | 390×844 Warm light | prior loaded Warm light `pr-files.png` |
| Completed AI review | 390×844 Warm dark RTL | prior loaded Warm dark `review.png` |
| Focus loaded long issue | 820×1180 Native dark | prior loaded Native dark `focus.png` |
| Local review history | 390×844 Native light | prior `local-review.png` |

All five theme combinations occur in the executed loaded Focus/PR/review screenshot scenarios. Above records specifically viewed evidence; it does not imply each subpage was visually inspected in every theme. New final screenshots include loaded 2048px header, collision menu, Warm light phone reply error, Warm dark RTL phone merge error/retry.

## Scores and limits

Scores below apply to the verified variants. No automatic upward adjustment to meet the round target. Browser fixture mutations establish UI behavior but do not claim native Tauri/VoiceOver, live forge publication, active multi-agent review streaming/cancellation, PWA/offline, actual clone authentication, or every subpage×theme×viewport combination. Reply composition across switching PR subtabs (distinct from leaving the route), moving refs during an open dialog, and larger-than-fixture history remain useful fresh-review variants for rounds 4/5. No confirmed feasible finding above is deliberately deferred.


## Final follow-ups and completed outcome

- Final diff inspection caught a priority change introduced by the typed-chip repair: chips sort head last for rendering, but lane tooltips must prefer HEAD first. Added a real browser tooltip assertion, observed the wrong local name (`/tmp/otto-ux-r3-git-lane-red.log`), then made priority explicit in `GraphView.svelte:1632`. This is fixed, not left for a later round.
- **P2 — Local review finding filenames were irretrievably clipped**, `LocalReviewPanel.svelte:708`. Actual prior phone screenshot showed only directory prefixes; the span had no title/action and hard 280px/nowrap clipping. Full paths now wrap, remain LTR, and retain line numbers on desktop/touch. Strengthened the existing R2 local-history test with actual non-clipping and direction assertions, avoiding a duplicate smoke test. Viewed the final WebKit screenshot: `reviewed-file.ts:1` is plainly visible at the end of the wrapped path.
- Final scoped desktop command: same fresh debug environment, `npx playwright test e2e/desktop-ux-r3-git.spec.ts e2e/desktop-git-graph-lanes.spec.ts --project=desktop-browser --output=/tmp/otto-ux-r3-git-final-results --workers=1` — **13 passed (59.7s)**. `/tmp/otto-ux-r3-git-final.log`. The nested-menu regression now checks Tab as well as arrows/Escape; reply regression checks Keep editing preserves the draft.
- Actual iPhone WebKit command: same fresh debug environment, `npx playwright test e2e/desktop-ux-r2-git.spec.ts e2e/desktop-ux-r3-git.spec.ts -g 'nested graph|failed comment reply|phone RTL merge|local review history|phone Focus|loaded Focus.*warm' --project=iphone-portrait --output=/tmp/otto-ux-r3-git-webkit-results --workers=1` — **7 passed (37.5s)**. `/tmp/otto-ux-r3-git-webkit.log`. Includes repaired complete local-review path, Focus keyboard ownership, Warm light/dark loaded PR/review, nested refs menu, reply failure→retry/navigation guard, RTL merge failure→retry.
- Verified all three final `.last-run.json` files (`verified-results`, `final-results`, `webkit-results`) say passed, empty failedTests. All tool sessions finished; no tests left running.
- Final E2E tsc, UI guards (803 files, no ratchet regressions), and scoped diff --check passed after the local-history change. Full `npm run check`/global Rust gates remain parent-owned.
- Viewed final loaded wide header, collision menu, reply error, RTL merge error, and final iPhone WebKit local-review history PNGs. Final reply evidence also confirms the parent's shared provider-banner repair: “A request to your Git provider failed with a gateway error. Local work remains available.” I reported the unsupported old automatic-retry promise; parent alone edited App.svelte.

| Verified family/important variant | Layout/readability | Interaction | Accessibility | States/recovery | Responsive |
| --- | ---: | ---: | ---: | ---: | ---: |
| Header, 2048px five long tabs + short tabs + overflow | 9.5 | 9.6 | 9.5 | 9.5 | 9.5 |
| Graph/diff/refs, desktop + short RTL + phone | 9.4 | 9.6 | 9.5 | 9.5 | 9.5 |
| Add repository local/remote/URL | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| Focus loaded desktop + phone/tablet modal | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| PR list + summary/title editor + conversation | 9.5 | 9.6 | 9.5 | 9.6 | 9.5 |
| PR files + completed review/posting | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| Merge readiness/error/retry/pending navigation | 9.5 | 9.6 | 9.5 | 9.6 | 9.5 |
| Local review history/recovery/long path | 9.4 | 9.5 | 9.5 | 9.5 | 9.5 |
| Stash/worktree/submodule action menus + unknown status | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |

Layout deductions are deliberate: the dense graph still requires expanding refs to inspect long identities at narrow desktop widths, and the history view spends substantial vertical space on its “No active review” region before the requested past runs. These are visible design tradeoffs, not a reproduced broken action. Active review streaming/cancellation/handoff and real worktree create/open workflows are **unscored**, not silently represented by the history/action-menu rows. No blanket >=9.5 claim. Round 4/5 fresh reviews must continue as required.

Owned changes: `crates/otto-git/src/local.rs`; `ui/src/modules/git/{GitTabs,GraphView,PrDetail,PrMergeModal,CommentThread,LocalReviewPanel,ReviewPanel}.svelte`; new `ui/e2e/desktop-ux-r3-git.spec.ts`; four additive assertions in `ui/e2e/desktop-ux-r2-git.spec.ts`. No other ownership taken.
