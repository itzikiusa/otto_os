# Round 3 — automation reviewer 4/10

Worktree: `/Users/itziklavon/otto-ux-audit-20260925`. Scope: Mission Control, Run with Otto, Swarm, Goal Loops, Workflows, Scheduled Tasks, Personal Agents/Rooms, Proof. No commits, subagents, shared-component edits, original-checkout edits, real messages, or external publications. All changes are in the owned modules and Swarm store. Full five-round/ten-reviewer obligation remains with the parent; this report covers one fresh review.

## Independently verified baseline

Read protocol, parent notes, AGENTS.md, design guidelines and both preceding reports. Reran `desktop-ux-automation.spec.ts` and `desktop-ux-r2-automation.spec.ts` together: **35/35 passed**. This independently verifies every R1/R2 regression: setup states, list Retry, broken Proof media Retry, truthful required-count display, missing-workspace draft preservation, Mission Control loading/tabs, agent tabs including RTL, and Rooms failure/phone layout. The loaded five-theme matrix was regenerated and actual screenshots viewed rather than accepted from the old report.

## Reproduced and repaired in this round

1. **P1 — Rooms carried an unsent message into a different conversation.** Type into Design room, select Release room: the new room showed the Design draft. Red assertion expected an empty composer but received `Design-only draft`. `RoomsView.svelte:26` now keeps drafts, sending state and errors by room. Returning to the first room restores its draft.
2. **P1 — Rooms keyboard send bypassed the disabled button, permitting duplicate submissions.** Hold a POST, type more, press Control+Enter: request count became 2. `RoomsView.svelte:145` now guards the request inside the handler. Completion clears only the exact submitted draft for the captured room; newer text remains. Per-room errors cannot leak into another room.
3. **P2 — Swarm feed refresh failure erased loaded posts and masqueraded as an empty board.** A 503 from the board endpoint led to “Quiet board.” `lib/stores/swarm.svelte.ts:551` preserves last-good data, records loading/error, and ignores responses superseded by another request or swarm. `BoardFeed.svelte:120` renders LoadState with inline Retry. Red→green verifies visible old posts and Retry recovery.
4. **P2 — Swarm view arrows ran opposite the visible RTL tab order.** Focus Org in RTL and press ArrowLeft: Graph was not focused. `SwarmPage.svelte:252` now derives direction for horizontal arrows, retaining Home/End and vertical handling.
5. **P2 — Swarm phone feed composer left about 35px for its message input.** Actual Warm-light 375px screenshot exposed the unreadable field. `BoardFeed.svelte:254` uses a container query: the input gets a full row; recipient, type and Post stay below it. A bounds/width regression now requires more than 240px at 375px. The repaired screenshot shows the full-width input.
6. **P1 — Posting to Swarm erased text typed while the first POST was pending.** Delayed transport reproduced loss of the next message. `BoardFeed.svelte:79` snapshots the submitted draft and only clears it if unchanged when the request completes.
7. **P1 — Workflow navigation silently discarded unsaved standing instructions.** Edit Instructions, click another workflow: no confirmation appeared. `WorkflowsPage.svelte:474` includes instructions in the existing discard guard, including reselecting the active row. The instructions textarea now also has an explicit accessible label.
8. **P1 — A late workflow save switched the active editor back to the previous workflow.** Save Alpha instructions, open Beta, release Alpha response: Beta lost its active selection. `WorkflowsPage.svelte:526` applies saves to their own list row and updates the editor only when its id matches, respecting monotonically increasing versions. Shared by graph, instructions and restart-policy saves.
9. **P1 — Canvas edits made during Save became falsely “saved”.** Edit manual-trigger input, hold Save, type newer input, release response: Save became disabled although newer content had not been sent. `WorkflowsPage.svelte:531` snapshots the submitted graph, prevents overlapping saves, and clears dirty only when the active graph still equals that snapshot.
10. **P2 — Personal Agent save errors were above the visible part of a long phone form.** The original phone screenshot showed the form’s bottom and Save button with no error; the viewport assertion measured error top at **−130px**. `AgentEditSheet.svelte:48` reveals the rendered inline error after validation/server failure. Name and other fields remain intact; retry creates the paused local fixture agent.

No confirmed defect in the reviewed paths is intentionally left unrepaired. Failed first attempts caused by test setup are separated below from product defects.

## Deeper behavior exercised

- Rooms: delayed old-room reply during navigation; 201-message pagination (second request after the 200th cursor); per-room draft restoration; duplicate shortcut protection; newer draft preservation; existing load Retry and phone push/back.
- Workflows: unsaved instructions, save-response races, newer canvas edits, validation with actionable broken-step identification, disabled cron trigger editing, durable loop checkpoints/version pinning, run timeline/step zoom/cancel, live expanded-step preservation, concurrent viewed/started runs, human-approval resume and error-step collapse behavior.
- Proof: required versus optional counts; reason minimum; simulated 403 approval rejection retains the reason; local retry records the waiver; done contracts, snapshots, image evidence, structured API/DB/Kafka evidence, failure semantics, CI policies, exports and PR-consistency evidence using synthetic/local data.
- Run with Otto and Scheduled Tasks: isolated fake-provider pipeline, approval/rejection, PR draft gating, creation/model/provider controls, local reports and schedule validation. No real publication.
- Swarm: loaded graph/board/feed, stale refresh recovery, RTL tabs, phone composer, session panel, org expansion/editor keyboard focus in the five themes (final outcome recorded below).

## Scores after repair

Scores assess the inspected UI and verified behavior, not untested production integrations. Contract-faithful fixtures are valid UI evidence; lack of a production mutation does not impose a score ceiling. L = layout/readability, I = interaction, A = accessibility, S = states/recovery, R = responsiveness.

| Family / assessed variants | L | I | A | S | R |
|---|---:|---:|---:|---:|---:|
| Mission Control — setup/loading/list/graph/detail; five-theme matrix | 9.3 | 9.3 | 9.2 | 9.5 | 9.3 |
| Run with Otto — launcher/loaded pipeline/detail/approval and draft; five themes | 9.3 | 9.4 | 9.1 | 9.4 | 9.3 |
| Swarm — Org/editor, Graph, Board, Feed, session panel; five themes | 9.3 | 9.4 | 9.2 | 9.4 | 9.4 |
| Goal Loops — long list, research draft/edit/launch/detail; five themes | 9.3 | 9.3 | 9.1 | 9.3 | 9.3 |
| Workflows — editor/instructions, loaded runs/live approval/error recovery; five themes | 9.3 | 9.4 | 9.1 | 9.5 | 9.3 |
| Scheduled Tasks — loaded list/create/validation/local runs/report; five themes | 9.3 | 9.4 | 9.1 | 9.4 | 9.3 |
| Personal Agents — create/error/retry, paused result, loaded detail tabs; five themes | 9.3 | 9.4 | 9.3 | 9.5 | 9.4 |
| Rooms — loaded conversation, pagination, async switching/drafts, phone push/back | 9.3 | 9.4 | 9.2 | 9.5 | 9.4 |
| Proof — contract/evidence/media recovery/waiver/reason/rejection/export | 9.3 | 9.5 | 9.2 | 9.5 | 9.3 |

These are not a blanket 9.5 certification. Main forms and recovery are substantially stronger, while complex keyboard-only graph manipulation, multi-agent coordination interventions, budget-resume loops and large live room/workspace races still need direct interaction evidence. Dense long identities on narrow panels and the accessibility of complex graphs deserve further attention; the present visual matrix alone does not establish those flows.

## Reproducible commands and outcomes

Run from worktree `ui/`. Every Playwright command uses:

```sh
OTTO_E2E_SLOT=ux3automation OTTO_E2E_PORT=7853 OTTO_E2E_PW_PORT=5353 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/release/ottod
```

Only one invocation ran at a time; waited for process exit/teardown before the next.

- Baseline: `npx playwright test e2e/desktop-ux-automation.spec.ts e2e/desktop-ux-r2-automation.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r3-automation-prior-results` → **35 passed**, 3.2m; `/tmp/otto-ux-r3-automation-prior.log`.
- Initial red: `npx playwright test e2e/desktop-ux-r3-automation.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r3-automation-red-results` → 2 passed, 7 failed. Six failures were intended behavior regressions; phone composer fixture initially omitted selecting the swarm (phone intentionally starts on its list). Corrected that fixture without removing the width/bounds assertion. Log `/tmp/otto-ux-r3-automation-red.log`.
- Additional red: same spec with `--grep 'phone composer|finishing a post|completing Save' --output=/tmp/otto-ux-r3-automation-red2-results` → 3 failed. Post/new-draft and Save/new-edit regressions failed as intended. The phone run had collected the earlier fixture before its correction, again timed out at Feed. Width defect was independently visible in the rendered prior screenshot. `/tmp/otto-ux-r3-automation-red2.log`.
- First repair run: same spec with `--output=/tmp/otto-ux-r3-automation-green-results` → **10 passed, 1 failed**. The sole failure was the newly tightened Personal Agent error viewport assertion (top −130px); repaired afterward. `/tmp/otto-ux-r3-automation-green.log`.
- Final broad command/outcome and WebKit outcome appended below after completion.
- UI guard initially caught a new scoped `.btn` selector; changed it to target the local composer button. No baseline increased. Subsequent guard passed.
- One initial guard invocation and one Playwright invocation used repository root instead of `ui/`; they failed before checks/tests and were rerun from `ui/`. No files or results from those failed invocations are counted as verification.

## Evidence viewed and next depth

Actually viewed with `view_image`, initially regenerated by the baseline and then refreshed by final verification where relevant:

- `/tmp/otto-ux-r2-automation-screens/warm-light-375-swarm-Feed.png` (original cramped input).
- `native-light-1440-workflow.png`, `native-dark-1440-mission-control.png`, `warm-dark-1024-rooms.png`, `pro-dark-dark-1440-proof.png` in the same directory.
- `warm-light-375-workflow-run.png`, `warm-light-375-agent-schedules.png`, `native-light-1440-run-with-otto.png`, `native-dark-1440-scheduled-tasks.png`.
- `/tmp/otto-ux-r3-automation-screens/phone-swarm-composer.png` (repaired full-width input).
- `phone-agent-save-retry.png` before error-reveal repair; `proof-waiver-recorded.png` after denied/retried approval. Final views appended below.

R4 should independently recheck these regressions, then emphasize real isolated Swarm task assignment/coordination intervention and budget resume, graph keyboard operation/large graphs, workflow instruction-vs-graph concurrent PATCH ordering, workspace changes during pending mutations, room pagination/live-event overlap, schedule-load errors and larger per-agent schedule histories. Test native Tauri window/VoiceOver behavior separately from browser WebKit. No screen-reader session, full measured contrast sweep or full Cartesian theme×viewport×RTL matrix is claimed. The browser fixtures do establish the tested flows without unsafe production actions.

## Final verification addendum

Broad pertinent verification:

```sh
npx playwright test e2e/desktop-ux-r3-automation.spec.ts e2e/desktop-ux-r2-automation.spec.ts e2e/desktop-workflow-recovery.spec.ts e2e/desktop-workflows-live.spec.ts e2e/desktop-workflow-runview.spec.ts e2e/desktop-proof-packs-v2.spec.ts e2e/desktop-scheduled-tasks.spec.ts e2e/desktop-run-with-otto.spec.ts e2e/desktop-swarm-session-panel.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r3-automation-final-results
```

**86 passed in 4.2m**, exit 0. `/tmp/otto-ux-r3-automation-final.log`; `/tmp/otto-ux-r3-automation-final-results/.last-run.json` is `{"status":"passed","failedTests":[]}`. All 13 functional R3 tests then present passed, including the corrected phone selection fixture, newly visible agent error, delayed-room response and 201-message conversation. The later five Org theme checks run separately below.

Final `npx tsc -p tsconfig.e2e.json --noEmit` passed (exit 0, empty `/tmp/otto-ux-r3-automation-types-final.log`). Final `node scripts/ui-guards.mjs` passed (exit 0, `/tmp/otto-ux-r3-automation-guards-final.log`). Scoped `git diff --check` passed. Full npm check/build are intentionally left to the parent; no global-gate success is claimed here.

Copied all 61 regenerated R2 loaded-fixture screenshots into `/tmp/otto-ux-r3-automation-screens/loaded/` to preserve this round’s evidence. Additionally opened and visually inspected the final repaired `phone-agent-save-retry.png` (error now fully visible above retained Name), `loaded/warm-light-375-swarm-Feed.png` (full-row message field), `loaded/native-dark-1440-swarm-Feed.png` (desktop remains balanced), and `loaded/pro-dark-dark-1440-loops.png` (loaded long cards). All evidence uses synthetic fixture accounts/content.

WebKit follow-up: `npx playwright test e2e/desktop-ux-r3-automation.spec.ts --project=iphone-portrait --workers=1 --grep 'phone composer|personal agent: failed save|org expansion' --output=/tmp/otto-ux-r3-automation-webkit-results` → **7 passed**, 33.7s, exit 0; `/tmp/otto-ux-r3-automation-webkit.log`, `.last-run.json` passed. The five Org variants use explicit 1440px Native light/dark + Pro Dark, 375px Warm light, and 834px Warm dark RTL viewports within WebKit. This is browser-engine coverage, not a physical iPhone/native Tauri claim.

Actually viewing those screenshots exposed an additional **P2 tablet toolbar clipping** issue at 834px RTL: Max parallel and run-budget controls extended beyond the left content edge while the page itself did not report document overflow. This illustrates why bounds-only smoke is insufficient. Additional repair and final checks recorded below. The first modal captures also caught the 160ms opening fade mid-frame; changed screenshots to `animations: 'disabled'` and regenerated instead of treating the translucent frame as a production defect.

11. **P2 — Tablet Swarm toolbar clipped budget/parallel controls.** `SwarmPage.svelte` now makes `.switcher` an internal horizontal scroll container at any narrow pane width, rather than only in phone mode, and keeps its row from shrinking vertically. Keyboard focus scrolls Max parallel into view; focusing Org scrolls back. The regression at 834px RTL checks the scroll containment plus viewport bounds of both controls. Initial isolated tablet attempt timed out during `page.goto` (environment/setup, no product assertion reached); rerun with `--timeout=90000` reached the intended assertion and failed with overflow-x `visible` instead of `auto`. Logs: `/tmp/otto-ux-r3-automation-tablet-red.log`, `/tmp/otto-ux-r3-automation-tablet-red2.log`. Final result below.

Final tablet/visual repair verification: `npx playwright test e2e/desktop-ux-r3-automation.spec.ts --project=iphone-portrait --workers=1 --timeout=90000 --grep 'phone composer|org expansion' --output=/tmp/otto-ux-r3-automation-webkit-final-results` → **6 passed**, exit 0, `.last-run.json` passed. Log `/tmp/otto-ux-r3-automation-webkit-final.log`. This final run verifies the final Swarm CSS at all five theme/viewport variants, including keyboard scrolling to Max parallel and back to Org in RTL. No process remains running in the assigned slot.

Reopened final `native-dark-1440-swarm-agent.png`, `warm-dark-834-swarm-agent.png`, and `warm-dark-834-org.png`; sheets are opaque and readable after animation stabilization, and the tablet toolbar has a visible internal scrollbar. Also viewed `native-light-1440-org.png`, `warm-light-375-org.png`, and `pro-dark-dark-1440-org.png`. The Warm-light phone org leaves useful room for every agent without page overflow; desktop keeps a quiet full-width hierarchy. Final UI guards, E2E types and scoped whitespace checks passed again after the last repair.

Important variant scores (to avoid the family rows concealing a narrower weak area):

| Variant | L | I | A | S | R |
|---|---:|---:|---:|---:|---:|
| Swarm Org/editor — Native light/dark and Pro Dark desktop | 9.4 | 9.4 | 9.3 | 9.3 | 9.4 |
| Swarm Org/feed — Warm light 375px phone | 9.3 | 9.4 | 9.2 | 9.4 | 9.4 |
| Swarm Org/editor — Warm dark 834px RTL tablet | 9.2 | 9.4 | 9.3 | 9.3 | 9.3 |
| Personal Agent create/error/retry — Native light phone | 9.3 | 9.4 | 9.3 | 9.5 | 9.4 |
| Rooms long conversation — Warm light phone / Warm dark RTL tablet | 9.3 | 9.4 | 9.2 | 9.5 | 9.4 |
| Workflow loaded run — Warm light phone | 9.2 | 9.4 | 9.1 | 9.5 | 9.3 |
| Proof contract — Pro Dark desktop / Warm light phone | 9.3 | 9.5 | 9.2 | 9.5 | 9.3 |

Tablet Swarm is still denser because app navigation plus the swarm rail leave a narrow detail pane; scrolling now makes all budget controls usable, but a fuller tablet rail-collapse design merits the next reviewer’s product judgment. This is a usability refinement opportunity, not the earlier inaccessible-control defect left open.
