# Round 1 — Git UX audit

Scope: Git landing/header and repo tabs, graph/diff/branches, PR list/detail, reviews, Focus, menus. Sources inspected in `/Users/itziklavon/otto-ux-audit-20260925`; AGENTS.md and design guidelines read. Reserved `fix/git-tabs-use-free-width` diff inspected and excluded; parent subsequently integrated it. No edits to original checkout or reserved branch. No commits.

## Scores (0–10)

| Lens | Before | After focused repairs |
| --- | ---: | ---: |
| Layout | 8.0 | 8.2 |
| Interaction | 6.5 | 7.8 |
| Accessibility | 6.8 | 7.5 |
| States | 6.5 | 8.0 |
| Responsive | 7.5 | 8.2 |
| Mean | 7.06 | 7.94 |

Scores reflect remaining source-confirmed defects and limited browser coverage. 9.5 is NOT warranted: PR publication/review flows, Focus populated quick-view, VoiceOver, high-volume history, and all five themes have not been exercised end-to-end in this pass. Initial RTL screenshot showed otherwise orderly dense three-column layout, with the diff visibly reversed. Header/reserved fixes are not credited to this agent.

## Fixed findings

All six regressions were first observed failing in the browser against an isolated real repository. Tests are `ui/e2e/desktop-ux-git.spec.ts`. Source line references below are post-fix unless explicitly described as original.

1. **P1 — A late diff response displayed the wrong patch beneath the selected commit title.** Original `GraphView.svelte:1456–1489` overwrote `diffResp` regardless of selection. Reproduction: select A, hold its response, select B, deliver B then A; A's files appeared under B's subject. Fixed `GraphView.svelte:1474` with request generation invalidated by selection, close, and WIP. Browser test verifies B stays selected with B's content.
2. **P2 — A failed diff claimed “No file changes.”** Original `GraphView.svelte:1482` assigned `{files:[]}` after a toast. Reproduction: return 503 on commit diff; false empty state remained. Fixed `GraphView.svelte:1494,3287` with inline LoadState, persistent failure text and Retry. Retry reloads the current commit without toggling it closed.
3. **P2 — Old PR pagination polluted a new state filter.** Original `PrList.svelte:73–86` appended an earlier request unconditionally; initial-page responses had the same stale-write path. Reproduction: hold Open page 2, switch to Merged and let it load, release Open page 2; an Open PR appears under pressed Merged. Fixed `PrList.svelte:44,83` with request revision, effect cleanup and matching finalization so old loads cannot overwrite new content or loading state.
4. **P2 — Focus confused account-service failure with missing setup.** Original `FocusView.svelte:99` swallowed account failures and showed “Connect a Jira account.” Reproduction: 503 `/issue/accounts`; page invited duplicate setup and provided no Retry. Fixed `FocusView.svelte:90,96,374` with explicit error state, Retry and loading reset.
5. **P2 — RTL reversed code/diff gutters.** Original GraphView diff hunks inherited document direction; browser computed `.dl-table` direction was `rtl`, and screenshot put line numbers/signs after code. Fixed `GraphView.svelte:3313,3332`: LTR boundaries for paths and diff bodies while surrounding chrome remains RTL.
6. **P2 — Branch operations required right-click/double-click.** Local/remote branch rows selected commits on Enter/click and exposed no keyboard/touch action trigger. Fixed `GraphView.svelte:2509,2566,2598,3557` with sibling real menu buttons carrying full branch labels, shared ctxMenu, and ≥36px mobile targets. Test opens the menu with Enter and at phone width, confirms viewport containment and no horizontal page overflow.

## Remaining findings for fresh review (source confirmed; not browser reproduced here)

- **P2 — Focus async selection races remain (`FocusView.svelte:109,199`).** `loadWork` and `openQuick` write completion state without matching account/key or request generation. Select issue A then B (or account A then B), complete B first, then A: A details can render under B's quickKey. Use request generation/current selection guards; invalidate on close/account change. Add delayed-response browser tests.
- **P2 — Mobile Focus quick-view has incomplete overlay behavior (`FocusView.svelte:421,858`).** Absolute positioned aside visually covers list at ≤1024px; no modal registration, initial focus, focus trap, Escape handler, or return focus. Use shared Modal at those widths or a shared accessible drawer; verify keyboard-only close and Tab containment. Desktop can remain a side pane.
- **P2 — Multi-ref graph expander and popover remain keyboard-incomplete (`GraphView.svelte:3028,3365`).** Expander is role=button span at tabindex=-1 nested in a button, no alternative keyboard command; popover doesn't move/restore focus or supply menu keys. Replace collapsed-ref interaction with accessible sibling button/shared menu, or implement an appropriate managed popover. Test 30 refs, keyboard open and return focus, short viewport, RTL.
- **P2 — Stash/worktree/submodule operations still depend on contextmenu (`GraphView.svelte:2743,2784,2839`).** Stash Enter only selects it; worktree Enter opens it; submodule has keyboard Enter but no normal click. There are no visible overflow buttons for these rows. Extend the branch sibling-action pattern and use native controls for tag/stash/submodule rows; verify keyboard/phone access and disabled worktree behavior.
- **P2 — Remote repository search is vulnerable to stale account/query responses (`GitPage.svelte:147`).** Fetch completion replaces remoteRepos and loading state unconditionally, while cloneRemote uses current browseAccount. Hold account A search, switch to B, return B then A: A results display under B account. Add request generation, capture account for results, clear obsolete results, and test delayed searches.
- **P3 — Add-repository mode tablist does not implement arrow/Home/End keys (`GitPage.svelte:415`).** Real buttons allow Tab navigation but composite-widget semantics promise arrow navigation. Use shared tab keyboard handling and roving tabindex.

## Verification

- Baseline: six regression tests failed for the intended visible symptoms; artifacts copied to `/tmp/otto-ux-git-red-results`.
- Direct `npx svelte-check --tsconfig ./tsconfig.app.json`: **0 errors, 0 warnings**; `/tmp/otto-ux-git-svelte.log`.
- `npx tsc -p tsconfig.e2e.json`: **passed**; `/tmp/otto-ux-git-tsc.log`.
- Full `npm run check` initially stopped at concurrent out-of-scope SftpBrowser global-class guard debt (6 vs baseline 3). Parent notified; do not claim full gate passed on this evidence.
- Final E2E: **10 passed (52.4s), exit 0**. Six regression flows plus four screenshot scenarios; `/tmp/otto-ux-git-e2e.log`.
- All four screenshots visually inspected after the run: native light/dark desktop 1440×900; Warm dark phone 390×844; native-light RTL tablet 820×1180. Graph/diff occupies the expected space, branch tools remain reachable, code gutters stay LTR, no horizontal page overflow in tested scenes. Phone and tablet use existing accordion navigation. This does not establish contrast numerically or cover deep review flows.
- Final `node scripts/ui-guards.mjs`: **passed**, 802 files, no ratchet regressions (concurrent SftpBrowser issue resolved).
- `git diff --check -- ui/src/modules/git ui/e2e/desktop-ux-git.spec.ts`: **passed**.

Screenshots:
- `/tmp/otto-ux-git-results/desktop-ux-git-graph-visual-review-light-desktop-desktop-browser/light-desktop.png`
- `/tmp/otto-ux-git-results/desktop-ux-git-graph-visual-review-dark-desktop-desktop-browser/dark-desktop.png`
- `/tmp/otto-ux-git-results/desktop-ux-git-graph-visual-review-warm-dark-phone-desktop-browser/warm-dark-phone.png`
- `/tmp/otto-ux-git-results/desktop-ux-git-graph-visual-review-light-rtl-tablet-desktop-browser/light-rtl-tablet.png`

Changed files owned by this pass: GraphView.svelte, PrList.svelte, FocusView.svelte, and new desktop-ux-git.spec.ts. No other files modified by this agent.

Command: `OTTO_E2E_SLOT=uxgit OTTO_E2E_PORT=7811 OTTO_E2E_PW_PORT=5311 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/claude_ade/target/debug/ottod npx playwright test e2e/desktop-ux-git.spec.ts --project=desktop-browser --output=/tmp/otto-ux-git-results --workers=1` from worktree `ui/`.
