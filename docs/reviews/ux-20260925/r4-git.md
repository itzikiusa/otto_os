# Round 4 — Git independent UX review (final round)

Workspace: `/Users/itziklavon/otto-ux-audit-20260925` only. Read protocol, round assignments, AGENTS, design README/checklist/accessibility, and previous Git reports. Used systematic-debugging, test-driven-development and verification-before-completion. No commits, subagents, user repository changes, real forge publication, or Rust/API shape changes. All real Git mutations were in newly created temporary repositories. Parent-owned global gates/PR/integration/install remain outside this report.

## Prior regressions independently rerun

**58/58 passed**, exit 0, 4.1 minutes: `/tmp/otto-ux-r4-git-prior.log`; `/tmp/otto-ux-r4-git-prior-results/.last-run.json` = passed, empty failedTests. This includes every R1/R2/R3 Git regression, exact 2048px five long repository/branch tabs, natural short tabs, integrated Always delete across tab, pane, menu and sidebar paths, eight-tab overflow/add menu, graph lanes, locked worktree removal force=false, and PR merge readiness/override fixtures.

Viewed the fresh 2048 screenshot: the tabs now use the header width; the final tab reaches the scroll edge with no unused-space truncation. That same screenshot visibly confirmed the graph's leading label still clipped, addressed below. The last tab may scroll when the content exceeds the actual available width; that is intended and the plus button stays visible.

## Confirmed repairs

1. **P2 — PR subtab switches destroyed reply drafts.** `ui/src/modules/git/PrDetail.svelte:434,580`. Type a Summary comment reply, inspect Files and Commits, return to Summary: the reply textarea and draft disappeared with no warning. The first new regression failed with the textarea absent. Summary and loaded Files now remain mounted while hidden, preserving child reply/edit state and route-leave guards. Both Summary and inline Files replies survive round trips; Keep editing still protects navigation to another PR. Hidden content is absent from the visible/focus surface.
2. **P2 — Graph branch labels clipped at the beginning.** `ui/src/modules/git/GraphView.svelte:3861`. Long checked-out branch plus HEAD and +33 ref markers: DOM measured the ref-select button starting **94.94 pixels outside** its clipping parent at 2048px. Its child chip could shrink, but the new accessible parent button could not. Make the parent shrink and contain overflow. The original bounds test passes and the final screenshot shows the leading checkmark/prefix, trailing ellipsis, expander and HEAD intact. Full identity remains in the ref popup/title.
3. **P2 — A delayed review poll revived a cancelled run.** `ui/src/modules/git/ReviewPanel.svelte:83,399,463`. Hold running GET, cancel successfully, then return the old running snapshot: Cancelled vanished and Reviewing returned. Add run generation checks to history/progress reads, start/retry/cancel transitions and disposal; stop pending poll timers on cancellation and resume on failed cancellation. The unchanged delayed-response regression passes. Active terminal output remains mounted across a synthetic review_changed event and progress refresh (one WebSocket attachment).
4. **P2 — Local review ref-name collision crashed the branch picker.** `ui/src/modules/git/LocalReviewPanel.svelte:103,296,319`. A real local `origin/collision` plus a remote `origin/collision` produced `each_key_duplicate`, leaving Compare to unusable. Graph identity had been fixed in R3; this separate consumer still collapsed identity to short names. Preserve both choices with `(local)`/`(remote)` labels and submit `refs/heads/...` or `refs/remotes/...` only for colliding names. The regression checks both outbound bases and absence of browser runtime errors. Wire shape remains the existing base:string.
5. **P3 — Loaded review history wasted vertical space on a large empty-state banner.** `ui/src/modules/git/LocalReviewPanel.svelte:353,510`. Twenty-four historical reviews appeared below 262.84 pixels of an oversized “No active review” panel. Replaced that panel with a compact status sentence when history exists; retained shared EmptyState for an actually empty page. Five theme/size variants verify history starts within 150px of toolbar bottom, expand a long finding, and check page overflow. Viewed all five rendered variants. The long list is substantially easier to scan and the phone reaches its first finding sooner.

## Deeper executed flows and negative findings

- Created an actual linked worktree with `git worktree add` in an owned temp fixture, clicked it in Git, verified registration and the active tab's linked branch, and verified the original checkout stayed on its original branch. Git currently exposes list/open/remove/prune, not a create-worktree UI; no such UI flow is claimed.
- Kept a Create branch dialog open for an explicitly selected commit, advanced HEAD with an empty commit in the fixture, then confirmed Create. The resulting branch still points to the originally selected SHA. No defect in this immutable-commit path. This does not claim all merge/rebase moving-ref combinations are tested.
- Started a mocked local review, verified selected findings, exercised failed handoff recovery with preserved selection, retried successfully to an intercepted synthetic session and verified agent tab/navigation. No actual provider launch.
- Opened an active PR reviewer's embedded terminal using faithful scrollback frames, delivered progress via review_changed, and verified live output stayed visible with one terminal attachment. Cancellation race regression is separate.
- Added existing 60-commit search/navigation tests to the final regression run, rather than duplicating their fixture.
- PR publication/merge/replies remain intercepted; the previous confirmation/body/error/retry regressions run unchanged.

## Verification

All Playwright commands run from worktree `ui/` with:

```
OTTO_E2E_SLOT=ux4git OTTO_E2E_PORT=7867 OTTO_E2E_PW_PORT=5367
OTTO_E2E_SWEEP_ORPHANS=0
OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/debug/ottod
```

One process at a time in this slot. Fixture specs block service workers; this is browser UI evidence, not production PWA evidence.

- Prior: `npx playwright test e2e/desktop-ux-git.spec.ts e2e/desktop-ux-r2-git.spec.ts e2e/desktop-ux-r3-git.spec.ts e2e/desktop-git-add-menu.spec.ts e2e/desktop-close-tab-ends-session.spec.ts e2e/desktop-git-pr-merge-modal.spec.ts e2e/desktop-git-graph-lanes.spec.ts e2e/desktop-git-worktree-status.spec.ts --project=desktop-browser --output=/tmp/otto-ux-r4-git-prior-results --workers=1` — **58 passed**.
- New red: `npx playwright test e2e/desktop-ux-r4-git.spec.ts --project=desktop-browser --output=/tmp/otto-ux-r4-git-red-results --workers=1` — **9 failed, 2 passed**, expected draft/clipping/history/cancel failures, plus branch-picker crash blocking local handoff. `/tmp/otto-ux-r4-git-red.log` has the duplicate-key trace. The genuine worktree open and moving-HEAD selected-commit tests passed before repairs.
- Initial green same new spec/output `green-results`: **13 passed (1.5m)**, `/tmp/otto-ux-r4-git-green.log`.
- Added success handoff and inline Files draft tests: `... desktop-ux-r4-git.spec.ts -g 'inline reply|hands only selected' ... --output=/tmp/otto-ux-r4-git-handoff-results` — **2 passed (24.2s)**, `/tmp/otto-ux-r4-git-handoff.log`, .last-run passed.
- **Final combined: 74 passed (4.9m), exit 0, .last-run passed:** prior command plus `e2e/desktop-ux-r4-git.spec.ts e2e/desktop-git-graph-search.spec.ts`, output `/tmp/otto-ux-r4-git-verified-results`, log `/tmp/otto-ux-r4-git-verified.log`.
- WebKit: `npx playwright test e2e/desktop-ux-r4-git.spec.ts e2e/desktop-ux-r2-git.spec.ts e2e/desktop-ux-r3-git.spec.ts -g 'reply survives|cancelled active|history composition.*warm|local review disambiguates|active PR agent|phone Focus|phone RTL merge' --project=iphone-portrait --output=/tmp/otto-ux-r4-git-webkit-results --workers=1` — **9 passed (1.1m), exit 0, .last-run passed**, `/tmp/otto-ux-r4-git-webkit.log`.
- `npx tsc -p tsconfig.e2e.json` — passed, `/tmp/otto-ux-r4-git-tsc-final.log` (empty output); `node scripts/ui-guards.mjs` — passed again, 805 files, zero ratchet regressions, `/tmp/otto-ux-r4-git-guards-final.log`. No baseline edits.
- Scoped `git diff --check -- ui/src/modules/git ui/e2e/desktop-ux-r4-git.spec.ts` passed. No full npm check or Rust commands; parent owns those gates.

## Screenshots actually viewed

- Prior-results exact `desktop-ux-r3-git-five-lon-826af-e-available-width-at-2048px-desktop-browser/five-long-tabs.png`.
- Prior-results Pro Dark RTL completed PR review `desktop-ux-r2-git-loaded-F-2214e-visual-pro-dark-desktop-rtl-desktop-browser/review.png`.
- Red-results `desktop-ux-r4-git-history-composition-native-light-1440-desktop-browser/history-before.png` and `desktop-ux-r4-git-graph-le-91ebb-s-inside-its-cell-at-2048px-desktop-browser/head-label-before.png`.
- Green-results `history-expanded.png` in all five `desktop-ux-r4-git-history-composition-*` directories: Native light 1440, Native dark 834, Warm light 390, Warm dark RTL 390, Pro Dark RTL 1440.
- Green-results fixed graph `desktop-ux-r4-git-graph-le-91ebb-s-inside-its-cell-at-2048px-desktop-browser/head-label-before.png` (filename retained for before/after comparison; this green file is after repair).
- Green-results `desktop-ux-r4-git-active-P-46ced-out-remounting-the-terminal-desktop-browser/active-review-stream.png`.

- WebKit-results Warm dark RTL phone `history-expanded.png` and Native light phone `active-review-stream.png` were also opened with view_image after the 9/9 WebKit run.

- Requested settled header evidence: a transient copy of the existing R3 fixture waited for loaded ref rows and captured both full screen and `.ph-row`. **1 passed (51.2s)**, `/tmp/otto-ux-r4-git-header.log`; temporary evidence spec removed. Viewed both `/tmp/otto-ux-r4-git-header-results/desktop-ux-r4-git-evidence-6c92e-e-available-width-at-2048px-desktop-browser/five-long-tabs-settled.png` and sibling `five-long-tabs-header.png`; the left branch rail is fully loaded. These supersede the earlier skeleton-bearing header screenshot for durable PR evidence.

## Per-family dimension scores (0–10)

Scores describe inspected composition and executed browser behavior; they are judgments, not automated quality measurements. No blanket target assignment.

| Family/important variant | Layout/readability | Interaction | Accessibility | States/recovery | Responsiveness |
|---|---:|---:|---:|---:|---:|
| Git header / exact five-long tabs / plus overflow | 9.6 | 9.5 | 9.5 | 9.5 | 9.5 |
| Graph / refs / commit detail / worktree open | 9.4 | 9.5 | 9.4 | 9.5 | 9.5 |
| Add repository / remote account search | 9.5 | 9.5 | 9.5 | 9.5 | 9.4 |
| Focus loaded desktop / narrow dialog | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| PR Summary / Files / comments and editors | 9.5 | 9.6 | 9.5 | 9.5 | 9.5 |
| PR AI review active / terminal / cancelled / completed | 9.5 | 9.5 | 9.4 | 9.5 | 9.5 |
| Local review / populated long history / selected handoff | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |

Concrete deductions: graph's fixed ref gutter still needs ellipsis/popover to read long branch names when HEAD and many refs share a row; dense graph controls require more keyboard stops than a full tree/grid keyboard model. Add-repository narrow multi-control forms are usable but crowded. Active AI review's configuration and multiple simultaneous terminal interaction are not fully exercised, so accessibility confidence remains below the target. These are limits and design tradeoffs, not deferred confirmed failing regressions.

Remaining limits: no physical macOS Tauri/VoiceOver session; no real forge writes or real provider launch; not every theme × page × breakpoint combination; no 10,000+ commit paging stress in this pass; no claim about every moving named-ref merge/rebase dialog combination. Production worker-enabled reload/outage belongs to parent evidence. No confirmed feasible defect listed above is deferred.
