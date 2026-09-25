# Round 4 — reviewer 2/10: automation

Worktree only: `/Users/itziklavon/otto-ux-audit-20260925`. No commits, subagents, native account writes or production mutations. Read the complete review protocol, Round4 assignment, parent notes, R1/R2/R3 reports, AGENTS and relevant design guidelines. Used systematic debugging, test-first reproductions and fresh verification. This is one reviewer’s report; it does not assert completion of the parent’s mandatory five rounds.

## Independently verified prior repairs

Reran R1, R2 and R3 automation suites together: **52 passed / 1 failed**. The sole failure read `.org-row` count before the initial phone fetch completed (count zero, then collapse found one row). Added a visible toggle wait before counting, preserving all substantive collapse/focus assertions. Subsequent broad run passes that phone check. All old setup, list Retry, Proof media/required count, missing-workspace draft, room send/paging/switching, workflow save and instructions races passed independently. Regenerated and viewed loaded screenshots rather than trusting old reports.

## Reproduced and repaired

1. **P2 — tablet Swarm detail crowded by two simultaneous rails.** R3 Warm-dark RTL 834px evidence showed the app navigator plus a 220px swarm rail, leaving approximately 390px for the actual work. `SwarmPage.svelte:48`, `:63`, `:580`, `:1180` extends the existing collapsible picker to tablets. Selecting a swarm returns keyboard focus to the picker; opening/closing it keeps detail mounted. The detail now has about 614px at 834px; tablet-to-phone retains Feed, draft and focus. The loaded final screenshot shows full-width readable posts and a compact picker band. Desktop-to-tablet preservation also exposed a separate shell remount issue, explicitly delegated below.
2. **P1 — repeated Enter duplicated task creation and completion erased the next title.** With the first POST held, a second Enter produced two requests. `KanbanBoard.svelte:156` guards the handler and disables the submit control during creation; completion clears only the submitted title in the same project. Red request count 2 → green 1, newer title retained.
3. **P1 — repeated Enter duplicated room creation and erased the next name.** The disabled Create button did not protect the input’s keyboard handler. `RoomsView.svelte:87` checks busy inside the handler and snapshots the submitted name. Red request count 2 → green 1, newer name retained.
4. **P1 — concurrent project loads could lose an entire populated board.** `swarm.svelte.ts:447` previously evaluated `this.tasksByProject[pid]` before awaiting. A sibling request then replaced that map; the late response assigned into the obsolete map. A deterministic fixture completes the empty sibling first and the populated project later. It reproduced Delivery as empty despite seven persisted tasks. Awaiting into a local, then merging into the current map, restores the populated board. This is a real async assignment-reference race, not a missing fixture wait.
5. **P2 — switching projects retained a false “1 selected” bulk toolbar.** After repairing the load race, the same regression reached its original target and failed with one selected task on the empty second project. `KanbanBoard.svelte:82` now clears selection when the project changes. The test still asserts both the populated board and cleared selection.
6. **P2 — Workflow canvas steps were pointer-only and could not open their editor with a keyboard.** Focusing the old div then pressing Enter left Message / prompt absent. `WorkflowCanvas.svelte:286` makes steps real buttons with explicit names and pressed state; `:427` gives them a semantic focus outline. Enter and Space select a step, editing and Save work, and saved input survives reload. Existing pointer-based inspector/run tests also pass.
7. **P2 — focused tablet budget input could touch the viewport edge.** After widening detail, the strict existing RTL bounds check measured the focused input at x=−0.109px. `SwarmPage.svelte:1180` now places tablet tabs and budget/parallel settings on two rows, removing the clipped input rather than relying on focus scrolling. A first scroll-margin attempt did not work reliably in WebKit and was removed. Assertions were not weakened. Final cross-engine result appended below.

## Main flows exercised

- Workflow graph parameter edit/save/reload through keyboard, plus existing inspector mode persistence, preflight errors, disabled cron preservation, durable loop attempts/version pinning, expanded live step retention, concurrent run viewing, human-approval pause→approve→success, error-step recovery, and cancel from the run view.
- Swarm keyboard card actions: assign to Ada, move to Blocked, verify stored dependency still references the prerequisite, reject a run with a meaningful dependency error, retry successfully. Launch requests intercepted; no provider launched by this new test.
- Multi-project boards and create-in-flight text preservation; room create/send duplicate protection, old-room replies, per-room drafts, and 201-message paging.
- Run with Otto and Scheduled Tasks existing suites against disposable state/fake providers; no forge publication. Personal Agents creation/error/retry, Proof waiver rejection/retry and media/count recovery were rerun.
- Rooms have a text-only `AgentRoomMessage` contract and no attachment UI. Parent confirmed not to invent attachment support. No attachment-flow verification is claimed.

## Scores and remaining uncertainty

L = layout/readability, I = interaction, A = accessibility, S = states/recovery, R = responsiveness. These judge the actual inspected fixture UI; no automatic increase for the target or round number.

| Family / inspected variants | L | I | A | S | R |
|---|---:|---:|---:|---:|---:|
| Mission Control loaded list/graph/detail and loading/error | 9.3 | 9.3 | 9.2 | 9.5 | 9.3 |
| Run with Otto launcher/pipeline/detail/approval | 9.3 | 9.4 | 9.2 | 9.4 | 9.3 |
| Swarm desktop Org/Graph/Board/Feed | 9.4 | 9.5 | 9.4 | 9.4 | 9.4 |
| Swarm tablet 834 RTL and phone Feed/picker | 9.5 | 9.5 | 9.4 | 9.4 | 9.5 |
| Swarm desktop↔tablet transition — pending shared shell repair | 9.5 | 7.0 | 7.0 | 8.0 | 7.0 |
| Goal Loops long-card list and research draft/edit/launch/detail | 9.3 | 9.3 | 9.1 | 9.3 | 9.3 |
| Workflow desktop keyboard editor/save, live approvals/cancel | 9.4 | 9.5 | 9.4 | 9.5 | 9.4 |
| Workflow phone loaded run/canvas | 9.2 | 9.4 | 9.2 | 9.5 | 9.3 |
| Scheduled Tasks create/validation/run/report | 9.3 | 9.4 | 9.2 | 9.4 | 9.3 |
| Personal Agents create/error/retry/detail tabs | 9.3 | 9.4 | 9.3 | 9.5 | 9.4 |
| Rooms long conversations/drafts/create/send/errors | 9.3 | 9.5 | 9.3 | 9.5 | 9.4 |
| Proof contract/media/waiver rejection/retry | 9.3 | 9.5 | 9.2 | 9.5 | 9.3 |

The tableau is still **not a 9.5 certification for the entire scope**. Swarm tablet composition improved concretely. Other deductions remain: dense/truncated long identities on cards and canvas; phone workflow inspector/timeline competes with graph height; keyboard edge creation/graph positioning, long multi-level outline editing, live budget-resume interventions, and loaded per-agent schedule histories are not fully exercised. No physical VoiceOver/native Tauri, comprehensive computed-contrast sweep, or full Cartesian theme×viewport matrix is claimed.

**Shared confirmed issue assigned to parent/shell reviewer:** crossing 1024px destroys/recreates module content in `ui/src/shell/App.svelte:1074` desktop/mobile branches. Repro: 1440 Swarm Feed → type draft → focus selected swarm row → resize834. Local component selection/draft/focus are lost before the compact picker can restore them. Kept the active regression `desktop-to-tablet swarm: detail gains rail width and navigation preserves selection and focus` in the R4 spec; parent explicitly owns shell integration. Final local commands exclude only this test pending that integration. This limitation is separate from the scored steady tablet/phone variant and must be resolved before whole-app completion.

## Commands and results

All browser commands run from worktree `ui/`, one Playwright invocation per assigned slot, waiting for exit/teardown. Prefix:

```sh
OTTO_E2E_SLOT=ux4automation OTTO_E2E_PORT=7862 OTTO_E2E_PW_PORT=5362 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/debug/ottod
```

- Prior: `npx playwright test e2e/desktop-ux-automation.spec.ts e2e/desktop-ux-r2-automation.spec.ts e2e/desktop-ux-r3-automation.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r4-automation-prior-results` → 52 passed, 1 test-readiness failure as explained above. `/tmp/otto-ux-r4-automation-prior.log`.
- New initial red R4 suite: `/tmp/otto-ux-r4-automation-red.log` → 4 failed: tablet layout, two duplicate submissions, and multi-project empty board preceding the intended selection assertion.
- `--grep 'project change|keyboard selects'`: `/tmp/otto-ux-r4-automation-red2.log` → 2 failed: empty board, keyboard editor absent.
- First repair run `/tmp/otto-ux-r4-automation-green.log` → 4 passed, 2 failed: shared desktop/tablet remount and multi-project empty board.
- Deterministic load-order confirmation `/tmp/otto-ux-r4-automation-red3.log` → 2 failed: remount and populated-board loss.
- After cache repair, `--grep 'project change'` `/tmp/otto-ux-r4-automation-selection-red.log` → 1 failed at the intended stale selection assertion. Final broad run passes it.
- Broad verification: `npx playwright test e2e/desktop-ux-r4-automation.spec.ts e2e/desktop-ux-r3-automation.spec.ts e2e/desktop-ux-r2-automation.spec.ts e2e/desktop-workflow-recovery.spec.ts e2e/desktop-workflows-live.spec.ts e2e/desktop-workflow-runview.spec.ts e2e/desktop-workflow-review-mode.spec.ts e2e/desktop-run-with-otto.spec.ts e2e/desktop-scheduled-tasks.spec.ts --project=desktop-browser --workers=1 --grep-invert 'desktop-to-tablet' --output=/tmp/otto-ux-r4-automation-final-results` → **81 passed, 1 failed** (3.5m). Sole failure was the fractional RTL input bound; corrected afterward. `/tmp/otto-ux-r4-automation-final.log`.
- Final E2E TypeScript: `npx tsc -p tsconfig.e2e.json --noEmit` → exit0, `/tmp/otto-ux-r4-automation-types-final.log` empty. UI guards exit0, `/tmp/otto-ux-r4-automation-guards-final.log`. Scoped `git diff --check` exit0. Parent owns full npm check/build.
- One Playwright attempt launched from repository root failed before tests (“desktop-browser not found”); immediately reran from `ui/`. No results from that attempt count as verification.

## Rendered evidence actually opened

Baseline R3 `warm-dark-834-org.png` exposed the two-rail crowding. Freshly regenerated R2 paths `/tmp/otto-ux-r2-automation-screens/` opened: `native-light-1440-workflow.png`, `warm-dark-1024-rooms.png`, `warm-light-375-swarm-Board.png`, `native-dark-1440-mission-control.png`, `native-light-1440-run-with-otto.png`, `pro-dark-dark-1440-loops.png`, `native-dark-1440-scheduled-tasks.png`, `warm-light-375-proof.png`, `warm-light-375-agent-schedules.png`, `warm-light-375-workflow-run.png`.

New `/tmp/otto-ux-r4-automation-screens/warm-dark-tablet-rtl-feed.png` opened: compact swarm picker above detail, readable posts and full-row composer; app navigation remains. Final five-theme Org and WebKit images/results appended after completion. Screenshots contain synthetic fixtures; no host account/transcript evidence is intended for publication.

## Final verification addendum

Cross-engine broader follow-up:

```sh
npx playwright test e2e/desktop-ux-r3-automation.spec.ts e2e/desktop-ux-r4-automation.spec.ts --project=desktop-browser --project=iphone-portrait --workers=1 --grep 'org expansion|tablet swarm: picker|swarm board:|rooms: pending creation|keyboard selects|phone composer|personal agent: failed save' --output=/tmp/otto-ux-r4-automation-verified-results
```

**24 passed, 2 failed**, `/tmp/otto-ux-r4-automation-verified.log`. All new task/room creation, project-map/selection, assignment/dependency/retry, and workflow keyboard/save checks passed in BOTH engines. WebKit’s RTL input bounds disproved the initial scroll-margin attempt. Chromium’s second failure was Org absent while navigation restarted around22:12:28UTC; parent reported a one-time Vite config restart for the content reviewer’s D2 fix around22:12UTC. It was not accepted as a passing test and was rerun after the configuration stabilized.

After replacing tablet focus scrolling with the two-row toolbar:

```sh
npx playwright test e2e/desktop-ux-r3-automation.spec.ts e2e/desktop-ux-r4-automation.spec.ts --project=desktop-browser --project=iphone-portrait --workers=1 --grep 'org expansion|tablet swarm: picker' --output=/tmp/otto-ux-r4-automation-layout-final-results
```

**12/12 passed in44.7s**, exit0. `/tmp/otto-ux-r4-automation-layout-final.log`; `.last-run.json` is `{"status":"passed","failedTests":[]}`. Both engines verify all five themes, tablet RTL input/tab bounds, Org collapse/editor/Escape focus, picker collapse/focus return, and tablet→375px phone draft/focus preservation. R4 desktop-oriented tests explicitly use1440×900; actual narrow layouts are set375×812 and834×1112. This is WebKit browser coverage, not a physical device/native app claim.

Personally reopened all five final Org screenshots, plus final Warm-dark RTL tablet Feed and phone Feed. Tablet has full-width tabs on their own row, fully visible budget and parallel controls below, no spare swarm rail, and approximately614px of detail. Phone retains its full-row composer and readable feed. Copies of61 loaded baseline images +10 final Org/editor images +2 new Feed images are preserved in `/tmp/otto-ux-r4-automation-screens/` (73 synthetic images total); selected Org screenshots live under `org/`, loaded matrix under `loaded/`.

Final UI guards passed after the last CSS change; scoped whitespace checks passed. Every Playwright process in the assigned slot exited with teardown complete. The sole still-pending product regression is the explicitly handed-off desktop↔tablet shell remount test; it is NOT skipped in source, and parent must run it after shell integration.

## Parent integration check

The parent inspected the final Warm-dark RTL tablet Feed screenshot and reran all six new standalone automation cases on the current integrated tree: **6passed23.0s**, exit0, `/tmp/otto-ux-r4-parent-automation.log`. The desktop-to-tablet case remains tracked with the shell integration and is not skipped in source.
