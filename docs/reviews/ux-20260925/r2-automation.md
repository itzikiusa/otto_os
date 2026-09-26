# Round 2 — reviewer 2/10: automation

Worktree: `/Users/itziklavon/otto-ux-audit-20260925`. Owned pages: Mission Control, Run with Otto, Swarm, Goal Loops, Workflows, Scheduled Tasks, Personal Agents (including rooms), Proof. No commits. Shared `personalAgents.svelte.ts` and `DoneContractMeter.svelte` changes explicitly coordinated with parent.

## Result and per-page scores

All reproduced defects in this pass were repaired. This is materially stronger evidence than round 1, but **not a defensible 9.5 across all pages**: live agent execution, native WebKit/Tauri, large concurrency/race matrices, and exhaustive keyboard/screen-reader coverage remain outside this browser pass. Scores below judge the verified scope, not the entire automation backend.

| Page / important variants | Layout/readability | Interaction | Accessibility | States/recovery | Responsive | Overall |
|---|---:|---:|---:|---:|---:|---:|
| Mission Control: setup, delayed load, list/graph/detail | 9.2 | 9.2 | 9.1 | 9.5 | 9.2 | 9.2 |
| Run with Otto: launcher, run detail, approval/draft pipeline | 9.2 | 9.3 | 9.0 | 9.3 | 9.2 | 9.2 |
| Swarm: loaded org/graph, Board, Feed | 9.0 | 8.9 | 8.8 | 9.1 | 9.0 | 9.0 |
| Goal Loops: long-card list, research draft/edit/launch/detail | 9.2 | 9.2 | 9.0 | 9.3 | 9.3 | 9.2 |
| Workflows: canvas, node inspector, completed runs, step zoom | 9.1 | 9.3 | 9.0 | 9.2 | 9.2 | 9.2 |
| Scheduled Tasks: loaded list, create form, runs/report | 9.2 | 9.3 | 9.0 | 9.2 | 9.2 | 9.2 |
| Personal Agents: setup, create/edit, detail tabs | 9.2 | 9.3 | 9.3 | 9.3 | 9.2 | 9.3 |
| Personal Agent Rooms: messages/error/retry, phone push/back | 9.2 | 9.2 | 9.1 | 9.3 | 9.3 | 9.2 |
| Proof: loaded contract/evidence, setup, failed media recovery | 9.2 | 9.2 | 9.0 | 9.4 | 9.3 | 9.2 |

## Round-1 verification

`desktop-ux-automation.spec.ts` independently reran: **17/17 passed**, `/tmp/otto-ux-r2-automation-prior.log`. This covers Workflows/Proof/Run setup, Personal Agents top-level keyboard focus, all eight list errors and Retry, real Proof media failure/recovery, and setup theme/viewport scenarios. Opened regenerated Native-light Workflows and Warm-dark Proof setup screenshots to verify rendered state.

All four listed round-1 residuals were reproduced or regression-tested and repaired:

1. **P1 — Personal Agent form discarded user input without a workspace.** `ui/src/modules/personal-agents/AgentEditSheet.svelte:111` now returns with actionable inline validation instead of falling through to close; an Add workspace action preserves the mounted form. `PersonalAgentsPage.svelte:132` gates both agent and room first-run views with workspace setup. Regression opens a valid form, types a name, removes scope, saves, and asserts the same dialog/name remain with the error.
2. **P1 — Goal Define with AI became an enabled no-op without workspace.** `ui/src/modules/loops/LoopsPage.svelte:71` provides setup; `GoalDefineForm.svelte:169` preserves the goal with inline Add workspace recovery, and definition/refinement/launch gates include scope. Missing-workspace regression and a phone research draft→edit→launch→detail flow pass (the drafting/launch transport in the latter is mocked).
3. **P2 — Mission Control loading masqueraded as zeros/blank list.** `ui/src/modules/mission-control/MissionControlPage.svelte:273` displays pending em dashes, `:331` renders an explicit loading skeleton, and workspace changes invalidate requests and clear prior summary/graph. Delayed transport regression verifies pending values and actual loading. Also fixed the reachable first-run Refresh no-op with workspace setup at `:265`.
4. **P2 — AgentPage deferred event.currentTarget focus loss.** `ui/src/modules/personal-agents/AgentPage.svelte:62` derives RTL direction; `:71` focuses the requested tab from the captured tablist immediately. ArrowRight/End and RTL ArrowLeft focus regressions pass. The top-level Agents/Rooms control has only two tabs, so either directional arrow wraps to the other in LTR and RTL; Home/End select logical endpoints.

## Newly reproduced and repaired

- **P1 — Proof could claim all requirements met despite a failed required test.** The loaded phone screenshot showed “3/3 required met” with a red required “Tests passed”. `crates/otto-core/src/proof.rs:1058` counts all satisfied items, including optional ones; the UI mislabeled that count. `ui/src/lib/components/DoneContractMeter.svelte:14` now derives only `required && satisfied`, displayed at `:51`. No API semantics changed. A fixture with two required successes, one required failure, and one optional success failed before repair and now renders **2/3**. Visually re-opened the corrected phone screenshot.
- **P1 — Rooms hid a message-fetch failure as “No messages yet”.** `ui/src/lib/stores/personalAgents.svelte.ts:194` now tracks loading/error per room and uses per-room request sequence checks, so superseded fetches cannot overwrite the latest fetch. `ui/src/modules/personal-agents/RoomsView.svelte:258` uses LoadState with inline Retry while retaining prior messages. A 503 regression failed before repair; after Retry the real isolated-daemon conversation appears. The load effect is untracked around store reads so cache updates do not retrigger a fetch loop.
- **P2 — Long phone conversations pushed room controls out of reach on composer focus.** Visually reproduced using long seeded messages. `RoomsView.svelte:323` and `:342` bound the detail/feed height; phone uses a room list→conversation flow with Back to rooms at `:215`, preserving the composer and header while the message region scrolls. `backToRooms` at `:73` restores focus to the prior room button. Bounds/scroll regression failed before repair and passes after.
- **P2 — Mission Control view tabs lacked arrow/Home/End handling.** `MissionControlPage.svelte:38` adds roving focus and keyboard selection; `:308` wires the tablist and per-tab tabindex. Browser red→green verifies Graph selection/focus then Home.

No browser-reproduced defect from this pass is being left unfixed.

## Test evidence and exact invocation

Run from the worktree `ui/`, prefix each Playwright command with:

```sh
OTTO_E2E_SLOT=ux2automation OTTO_E2E_PORT=7842 OTTO_E2E_PW_PORT=5342 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/release/ottod
```

The prior/main initial runs used `/Users/itziklavon/claude_ade/target/debug/ottod` per protocol. Later runs switched to the matching release binary when parent announced availability.

- Prior: `npx playwright test e2e/desktop-ux-automation.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r2-automation-prior-results` → **17 passed**.
- Initial seven reproduction tests: `/tmp/otto-ux-r2-automation-red.log` → **7 failed** for their intended residuals. Core rerun `/tmp/otto-ux-r2-automation-core.log` → **7 passed, room-message regression failed** before its fix.
- Additional reproduction run with `--grep 'keyboard switches|RTL arrow|phone conversation'` → **3 failed** as intended, `/tmp/otto-ux-r2-automation-deeper-red.log`.
- Main integration invocation: `npx playwright test e2e/desktop-ux-r2-automation.spec.ts e2e/desktop-workflow-runview.spec.ts e2e/desktop-scheduled-tasks.spec.ts e2e/desktop-run-with-otto.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r2-automation-main-results` → **43 passed, 1 failed**. The failure was the new phone fixture looking for desktop Runs button; phone deliberately exposes Runs in More actions. Corrected the test to follow that menu, retaining the substantive assertions. The **15 Run with Otto + 12 Scheduled Tasks + 4 Workflow run-view tests all passed**. Log: `/tmp/otto-ux-r2-automation-main.log`.
- Proof red run with `--grep 'goal research flow|optional successes'` → **goal flow passed, Proof regression failed** before repair. Log `/tmp/otto-ux-r2-automation-proof-red.log`.
- **Final full new suite**: `npx playwright test e2e/desktop-ux-r2-automation.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r2-automation-verified-results` → **18/18 passed**, 56.2 seconds. Log `/tmp/otto-ux-r2-automation-verified.log`; `.last-run.json` is `{"status":"passed","failedTests":[]}`.
- Workflow node inspector: `npx playwright test e2e/desktop-workflow-review-mode.spec.ts --project=desktop-browser --workers=1 --grep 'inspector defaults' --output=/tmp/otto-ux-r2-automation-inspector-results` → **1 passed**, `/tmp/otto-ux-r2-automation-inspector.log`.
- Final `npx tsc -p tsconfig.e2e.json --noEmit` → exit 0, `/tmp/otto-ux-r2-automation-types-final.log` empty.
- Final `node scripts/ui-guards.mjs` → exit 0, no regressions, `/tmp/otto-ux-r2-automation-guards-final.log`.
- Scoped `git diff --check` → exit 0. Full npm check/build intentionally left to parent, per coordination protocol.

## Visual evidence

61 screenshots in `/tmp/otto-ux-r2-automation-screens`: all eight page families with loaded content in Native light/dark (1440×900), Warm light phone (375×812), Warm dark RTL tablet (1024×768), and Pro Dark (1440×900), plus goal detail. Matrix verifies selected tab before capture, populated content and no horizontal document overflow; wide graphs/timelines remain internal scroll regions.

Actually opened with `view_image` (prefix `/tmp/otto-ux-r2-automation-screens/`):

- `native-light-1440-swarm-Board.png`, `native-dark-1440-mission-control.png`, `native-light-1440-workflow-run.png`, `native-light-1440-run-with-otto.png`, `native-dark-1440-scheduled-tasks.png`.
- `warm-light-375-rooms.png` before and after bounded-layout repair, `warm-light-375-workflow.png`, `warm-light-375-workflow-run.png`, `warm-light-375-swarm-Graph.png`, `warm-light-375-agent-schedules.png`, `warm-light-375-loops.png`.
- `warm-light-375-proof.png` before and after the required-count repair; `phone-goal-detail.png`.
- `warm-dark-1024-workflow-run.png`, `warm-dark-1024-rooms.png`, `pro-dark-dark-1440-loops.png`.
- Prior regenerated setup evidence in `/tmp/otto-ux-screenshots/`: `automation-after-native-light-1440-workflows.png`, `automation-after-warm-dark-1440-proof.png`.

## Limits / next independent pass

No native Tauri/WebKit, screen-reader, full axe/contrast sweep, or full Cartesian theme×viewport×RTL matrix. Execution tests use isolated daemon state and stub provider CLIs; no real agent account/cloud mutation. Research drafting is mocked. Swarm execution/session panes, extensive keyboard navigation beyond tested controls, many-room switching with in-flight responses, >200-message paging, complex workflow inspector validation and many-node graphs deserve the next fresh pass. These are explicit coverage gaps, not asserted defects or claimed 9.5 readiness.

## Parent independent Swarm verification

Matching release daemon: desktop session-panel and empty-workspace specs passed (2 tests, `/tmp/otto-ux-swarm-desktop.log`). Phone Swarm/Goals/Mission Control run passed 23 tests, with one explicitly desktop-only skip and one stale selector failure: the cap input is `swarm-cap`, while the old test searched `#cap`. Replaced the stale selector with the accessible “Max parallel” spinbutton and asserted viewport bounds. Both affected header tests then passed in WebKit (`/tmp/otto-ux-swarm-phone-fixed.log`, 2 passed, 17.5s). No product assertion was dropped.
