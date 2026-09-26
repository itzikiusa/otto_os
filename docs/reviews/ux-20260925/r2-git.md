# Round 2 — Git, independent review and repairs

Worktree: `/Users/itziklavon/otto-ux-audit-20260925`; original checkout untouched. Read protocol, AGENTS.md, design README/checklist and Round 1 report. Used systematic debugging and test-first regressions. No commits; no global npm check. Six owned Git components and `ui/e2e/desktop-ux-r2-git.spec.ts` changed.

## Prior regressions independently checked

All 10 tests in `desktop-ux-git.spec.ts` passed before changes (46.1s, `/tmp/otto-ux-r2-git-prior.log`) and passed again after changes on the current release daemon. This includes commit selection races, diff inline Retry, stale PR pagination, account failure Retry, RTL graph diff, branch keyboard/touch menus, and the four prior screenshot variants. Viewed prior light desktop and RTL tablet evidence, then final light desktop and Warm dark phone evidence.

## Confirmed findings and repairs

- **P2: Focus account and issue races** — `FocusView.svelte:112,230`. Held account A work/issue A responses, selected B and loaded B, then delivered A: A replaced B. Added request generations, matching account checks, invalidation on account changes/close and current-only finalization. Account changes clear the quick-view. Delayed-response regressions first failed and now pass.
- **P2: phone Focus quick-view had no keyboard ownership** — `FocusView.svelte:211,462`. Opening left focus on the underlying issue, Tab escaped and Escape did not close. Narrow panel now uses shared `dialogFocus`, dialog semantics, inert underlying issue list, modal registration with untracked push/pop, focus return, and suppression of the global `?` shortcuts sheet. Desktop remains a complementary pane. WebKit additionally reproduced pointer-click focus return to body; issue and ref openers now explicitly focus their trigger before mounting shared dialogFocus. The unchanged return-focus regression passes in WebKit. Failure has actual inline detail and Retry at `FocusView.svelte:495`.
- **P2: collapsed graph references were inaccessible to keyboard** — `GraphView.svelte:3028,3051,3366`. Former nested span had tabindex -1. Real sibling buttons now independently select the commit and expand references, preserving SVG lane hover targets. Shared dialogFocus handles opening/closing/focus return; ref rows expose explicit action buttons so checkout/merge/tag actions do not require double/right click. Modal count registers the dialog. Thirty real branch refs exercise scrolling and clamping. A final short-viewport test also reproduced a loading race: opening before refs arrived snapshotted slash-named local branches as remotes, so later action dispatch silently missed them. The expander now waits for refs and actions resolve current identity. The keyboard test explicitly waits until this loading-disabled control is enabled before focusing it. A screenshot before repair showed globe icons for local release branches; after repair it shows branch icons and the nested action menu opens.
- **P2: stash/worktree/submodule operations lacked keyboard/touch controls** — `GraphView.svelte:2754,2816,2845`. Stashes and worktrees get visible sibling action buttons; stale worktree actions remain enabled while its primary Open is disabled. Submodule row is a real clickable button. Menus use shared ctxMenu and mobile targets are 36px. Tested keyboard opening for all three and phone tapping the stash actions.
- **P2: remote browse account/query races** — `GitPage.svelte:83,175,185,208`. A delayed A response appeared under B and Clone would have used B credentials. Searches now invalidate old requests immediately, clear old results during debounce, capture account ownership, and ignore stale results/errors/finalization. Clone checks the result account. Closing invalidates queued work; reopening Browse refreshes again. Account and query races plus reopen are covered. The initial remote test had an ambiguous Account locator (shell button and select); corrected it, then reproduced the actual stale response before repair.
- **P3: Add repository tabs omitted composite keyboard behavior** — `GitPage.svelte:52,462`. Added roving tab index, arrows/Home/End, including RTL direction, matching RepoView's established behavior.
- **P1: PR edits leaked across PR identities** — `GitPage.svelte:308`, `PrDetail.svelte:68`. Edit PR 1, navigate to PR 2: PR 1's unsaved title appeared in PR 2's editor and could be sent to PR 2. Keyed the PR component by repository and number. Existing shared unsaved guard covers title/description, general comment and request-changes drafts. Regression verifies Keep editing preserves the draft and Discard opens a clean PR 2. A delayed PR 1 save cannot close/change PR 2's editor. Disposal guards avoid reading destroyed reactive props and the regression asserts no `derived_inert` warning.
- **P2: edits accepted while PR save was pending were discarded** — `PrDetail.svelte:206,361,433`. Delayed PATCH reproduced enabled editor fields during save; completion closed the editor. Fields and Cancel are disabled during save; duplicate save is guarded. Added an accessible title input label. Delayed-save regression verifies both fields and Save stay disabled until completion.
- **P2: PR file diffs and AI-review snippets reversed gutters under RTL** — `DiffViewer.svelte:827,867,901,920,999,1018`; `ReviewPanel.svelte:1200,1244,1306`. Visual inspection showed the change sign/line number after code, despite the prior graph-only fix. Explicit LTR boundaries now cover paths, unified/split/virtual code rows, hunk headers and review snippets. Surrounding navigation stays RTL. Computed-direction regression initially failed; final screenshots show gutters preceding code.

## Deeper verification

Loaded Focus + long issue description; loaded PR summary/comments/long branch, long-path diff, and completed AI review are rendered in Native light desktop, Native dark tablet, Warm light phone, Warm dark RTL phone, and Pro Dark RTL desktop. Review posting is fully intercepted: confirmation shows Where/What/Who, Cancel sends nothing, confirmed mock send changes draft to posted. Local review history first fails, offers Retry, recovers and expands a long historical finding on phone.

Long-content screenshots are actual loaded pages, not route aliases or no-workspace inventory. No page overflow in the covered scenarios. The first deep phone review assertions expected an ordinary document button to be visible before scrolling; that was a test mistake, not a layout defect. Corrected to scroll the button into view before checking its bounds. First stash phone assertion similarly needed to reopen its collapsed section after the shell remounted at a responsive breakpoint; keyboard menus already worked.

## Verification results and commands

All commands run from worktree `ui/`. Common environment:

```
OTTO_E2E_SLOT=ux2git OTTO_E2E_PORT=7841 OTTO_E2E_PW_PORT=5341 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/release/ottod
```

- Main final run: `npx playwright test e2e/desktop-ux-git.spec.ts e2e/desktop-ux-r2-git.spec.ts --project=desktop-browser --output=/tmp/otto-ux-r2-git-verified-results --workers=1` — **29 passed**; `/tmp/otto-ux-r2-git-verified.log`; `.last-run.json` says passed with zero failures.
- Earlier 24-test debug-binary and current-release runs also passed. Initial baseline/failure evidence: `/tmp/otto-ux-r2-git-red-results`, `remote-red-results`, `deep-results`, `deep2-results`, `reopen-red-results`, with corresponding `/tmp/otto-ux-r2-git-*.log` files.
- `npx tsc -p tsconfig.e2e.json` — passed, `/tmp/otto-ux-r2-git-tsc.log`.
- `node scripts/ui-guards.mjs` — passed, 802 files, zero ratchet regressions; `/tmp/otto-ux-r2-git-guards.log`. Did not raise baseline.
- `git diff --check -- ui/src/modules/git ui/e2e/desktop-ux-r2-git.spec.ts` — passed.
- One intermediate direct svelte-check found a missing dialogFocus import during implementation; repaired. Global type/build gate remains parent-owned.
- Phone WebKit: `npx playwright test e2e/desktop-ux-r2-git.spec.ts --project=iphone-portrait -g 'phone Focus|loaded Focus.*warm|local review history' --output=/tmp/otto-ux-r2-git-webkit-verified-results --workers=1` — **4 passed (15.6s)**, `/tmp/otto-ux-r2-git-webkit-verified.log`; `.last-run.json` passed.
- The first four WebKit tests and a context.route diagnostic were invalid as product checks: the mobile service worker owned requests, so Playwright routes were bypassed (trace: actual issue/accounts returned `[]`, actual PR GET returned 400). Matched the established repo fixture convention, `test.use({serviceWorkers:'block'})`; did not alter the product service worker. With valid fixtures, 3/4 initially passed and the fourth reproduced the Safari pointer-focus defect above. Final 4/4 pass. This tests browser UI with direct mocks, not offline/PWA service-worker behavior.
- Round 2 desktop rerun with blocked-worker fixtures: `npx playwright test e2e/desktop-ux-r2-git.spec.ts --project=desktop-browser --output=/tmp/otto-ux-r2-git-complete-results --workers=1` — **20 passed, 1 failed (1.0m)**. The sole failure discovered the real early-ref popup action defect above; this was repaired, not deferred. Quick-view error-detail/Retry passed here. An initial focused post-fix run passed the short RTL menu and stash/worktree/submodule tests but the earlier keyboard test tried focusing the deliberately loading-disabled expander; updated it to await enabled readiness without removing its focus/keyboard assertions.
- Final focused graph run after that adjustment: **3 passed (10.9s)**. Command: `npx playwright test e2e/desktop-ux-r2-git.spec.ts -g 'many graph references|graph multi-ref|stash, stale' --project=desktop-browser --output=/tmp/otto-ux-r2-git-graph-verified-results --workers=1` with the release environment above. Log: `/tmp/otto-ux-r2-git-graph-verified.log`. Final E2E TypeScript, UI guards (803 files), and scoped `git diff --check` also passed. Parent independently confirmed the full UI check: zero errors and warnings.

## Screenshots viewed

Main directory `/tmp/otto-ux-r2-git-verified-results/` contains five `desktop-ux-r2-git-loaded-*` scenario directories, each with `focus.png`, `pr-summary.png`, `pr-files.png`, `review.png`; prior graph scenarios have named graph PNGs. Individually inspected light desktop graph, RTL tablet graph, Warm dark phone graph; Native light desktop Focus and review; Native dark tablet Focus; Warm light phone PR files and local review; Warm dark RTL phone Focus and review; Pro Dark RTL desktop PR files and review. Also inspected Warm light phone Focus, Native dark tablet review, Pro Dark RTL Focus, and the WebKit Warm dark RTL review. Initial RTL screenshots caught the reversed-gutter defect; final screenshots verify it corrected. Also viewed the final 1100×500 RTL 30-ref popover screenshot at `/tmp/otto-ux-r2-git-graph-final-results/desktop-ux-r2-git-many-gra-25243-ble-in-a-short-RTL-viewport-desktop-browser/graph-many-refs.png`: the last reference and its action button remain visible inside the scrollable popup.

## Scores (0–10)

Scores are scoped to these verified browser flows, not a claim of whole-app completion. Each row covers an actual page family/variant; lowest unverified areas are kept visible.

| Page/variant | Layout/readability | Interaction | Accessibility | States/recovery | Responsive |
| --- | ---: | ---: | ---: | ---: | ---: |
| Graph/commit diff, desktop + phone + RTL tablet | 9.4 | 9.5 | 9.4 | 9.5 | 9.5 |
| Add repository local/remote/URL | 9.5 | 9.5 | 9.5 | 9.5 | 9.4 |
| Loaded Focus desktop | 9.5 | 9.5 | 9.4 | 9.5 | 9.5 |
| Loaded Focus phone/tablet overlay | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| PR list + PR summary/editor | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| PR files + completed review, including RTL phone | 9.5 | 9.5 | 9.4 | 9.5 | 9.5 |
| Local review history/recovery, phone | 9.4 | 9.4 | 9.4 | 9.5 | 9.5 |

No confirmed feasible defect above is intentionally deferred. A universal 9.5 claim is not yet justified: native Tauri/VoiceOver, actual remote clone/publication/merge, active multi-agent review streaming/cancellation, local review handoff, and a full all-five-themes matrix for every subpage remain untested. Review publication was mocked deliberately; no user cloud account, session or working repository was modified. Rust integration/global checks and final screenshots for PR packaging belong to parent.
