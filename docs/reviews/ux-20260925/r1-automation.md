# Reviewer 5 of 10 — Round 1 automation UX

Worktree: `/Users/itziklavon/otto-ux-audit-20260925`. Scope: Mission Control, Run with Otto, Swarm, Goal Loops, Workflows, Scheduled Tasks, Personal Agents, Proof. No shared component changes, commits, or original-checkout writes.

## Rating after focused repairs

| Dimension | /10 | Basis |
|---|---:|---|
| Layout | 8.3 | Shared quiet toolbar, readable empty states, contained phone setup controls; loaded complex editors not fully audited. |
| Interaction | 7.3 | Setup and Retry paths verified; remaining no-workspace flows still lose intent or silently do nothing. |
| Accessibility | 7.8 | Repaired tab focus; keyboard setup/open/Escape verified. No comprehensive axe or screen-reader pass. |
| States | 7.5 | All eight list failures recover with inline Retry; media now recovers; initial Mission Control loading and some first-run states remain incomplete. |
| Responsive | 8.2 | Viewed phone and RTL evidence, tested setup overflow and viewport containment; rich populated graphs/editors still need coverage. |
| Mean | **7.82** | **Below 9.5; main agent execution flows are not fully verified and known material residuals remain.** |

## Confirmed and repaired

1. **P1: endless first-run loading in Workflows and Proof.** With no workspaces, their effects skip the fetch while initial loading/listLoaded flags retain skeletons. Visible in `phone-light-workflows.png`, `phone-light-proof.png`, `native-dark-proof.png`, `warm-dark-workflows.png`. Added actionable workspace setup states using existing EmptyState + existing workspace dialog. Locations: `ui/src/modules/workflows/WorkflowsPage.svelte:3046`, `ui/src/modules/proof/ProofPage.svelte:717`.
2. **P1: Run with Otto tells a first-run user to paste into an absent input.** `RunLauncher` is gated on a workspace but the runs empty state wasn't. `native-dark-run-with-otto.png` shows the contradiction. Added workspace setup state around the launcher/list content: `ui/src/modules/run-with-otto/RunWithOttoPage.svelte:151`.
3. **P2: Personal Agents tabs change selection without moving focus.** Focus Agents, press ArrowRight: Rooms becomes selected but Agents retains focus. Original deferred callback reads `event.currentTarget` after dispatch (null); routing itself updates asynchronously. Capture tablist and focus the requested tab synchronously. `ui/src/modules/personal-agents/PersonalAgentsPage.svelte:99`. Browser red then green verifies ArrowRight and Home selection plus focus.
4. **P1: broken Proof media remains “Loading media…” forever.** A 503 from artifact blob fetch was swallowed. Store per-artifact failure and offer inline Retry, clearing failure while retrying. `ui/src/modules/proof/ProofPage.svelte:219` and `:829`. Test seeds a real PNG artifact, forces blob 503, retries the real endpoint, and checks decoded image naturalWidth=1. Browser red then green.

## Verification

- `ui/e2e/desktop-ux-automation.spec.ts`: 13 functional checks passed in `/tmp/automation-final.log`: 3 phone setup keyboard/modal flows, Personal Agents roving focus, 8 modules' list-503/Retry recovery, real Proof media recovery.
- Original four regressions failed before repair (`/tmp/automation-red.log`); media failed before repair (`/tmp/automation-states-red.log`, 9 other checks passed).
- Four visual scenarios initially hit an ambiguous test locator (sidebar also has Add workspace); corrected to scope the page empty state. Visual-only rerun status appended below.
- `npm run check` passed with 0 errors/0 warnings before final media patch; final check status appended below. No build or full suite claimed.
- Screenshots actually opened: native-light Mission Control/Scheduled Tasks; native-dark Run with Otto/Proof; Warm-dark Swarm/Workflows; phone-light Mission Control/Goal Loops/Workflows/Proof/Personal Agents; tablet-RTL Personal Agents/Scheduled Tasks. Evidence is predominantly empty/setup pages, not complete loaded-flow validation.

## Residual findings for a fresh next round

1. **P1, browser reproduced: Personal Agent form silently discards a new agent when no workspace exists.** Open Personal Agents on a fresh account → New agent → type a name → Save. `ui/src/modules/personal-agents/AgentEditSheet.svelte:126` skips creation if `ws.currentId` is null, then unconditionally `onclose()` at `:138`. Browser observation: `{dialogs:0, unsavedNameVisible:0, emptyStateVisible:true}`; screenshot `/tmp/otto-ux-screenshots/automation-residual-personal-save.png`. Remedy: gate first-run creation with workspace setup, and preserve form + actionable inline validation if its workspace disappears.
2. **P1, source-traced, not browser reproduced: Goal Loop Define with AI becomes an enabled no-op without workspace.** `ui/src/modules/loops/GoalDefineForm.svelte:51` early-returns for null wsId, while button `:192` only checks seed/repository. In research mode enter a goal then press Define with AI. Remedy: actionable workspace setup before entering definition, plus guard message if scope disappears.
3. **P2, source-traced: Mission Control initial loading is visually empty with zero-valued summary.** `ui/src/modules/mission-control/MissionControlPage.svelte:312` skips empty/error states while loading then renders an empty WorkItemList at `:338`; summary `:253` displays zero defaults. Delay workgraph endpoints to verify. Remedy: explicit loading treatment and honest pending summary values.
4. **P2, coverage gap: rich loaded and long-content paths remain under-tested.** Next review should seed dense Swarm graph/kanban, long Goal Loop cards, Workflow inspector and runs, scheduled report history, and Personal Agents Rooms; verify phone push navigation, RTL arrows, long names, and focus return. Current evidence does not support a 9.5 score.

## Final verification addendum

- Final `npm run check` exited 0: UI guards passed; svelte-check 0 errors/0 warnings; all TypeScript configs passed (`/tmp/automation-final-check.log`).
- Scoped `git diff --check` passed.
- Personally viewed repaired phone Proof and Run with Otto screenshots; readable copy, visible keyboard focus, contained action, no overlap.
- Corrected visual checks: **4/4 passed** in `/tmp/automation-visual.log`, producing 12 screenshots of all three setup pages in native light/dark, Warm dark, tablet RTL; overflow and action bounds pass. Together with functional run: **17/17 checks pass across the two final runs**.
- Personally viewed the repaired desktop native-light Workflows, Warm-dark Proof, and tablet-RTL Run with Otto screenshots: consistent empty-state hierarchy and one contained primary action.
