# Round 1 — UX review

Baseline: `196048df`. Review date: 2026-10-07. Source review only: no builds, browser runs, tests, production requests, or source edits were performed by this reviewer. Runtime capacity was reserved for the coordinator's fresh daemon build. This is a bounded review of the assigned areas, not certification of every Otto flow.

## Scope and evidence

Read `AGENTS.md` and the design guidelines for patterns, layout, accessibility and review. Inspected the following current source surfaces; prior reports were not treated as proof.

| Area | Examined | Positive evidence / remaining boundary |
| --- | --- | --- |
| Scheduled Tasks | `ScheduledTasksPage.svelte`: editor leave decisions, save ownership, delivery setup, run/report recovery | Dirty-form guard, preservation of edits made during Save, inline load recovery and visible delivery failures are present. Browser behavior remains unverified. |
| Workflows | `WorkflowsPage.svelte`: navigation, dirty graph/instructions, save/run/preflight, approval/output failures | Navigation guards and submitted-version save ownership are present; run errors and failed output reads have visible recovery. Did not audit every node editor. |
| Goal Loops | `LoopsPage.svelte`, `LoopDetail.svelte`: selection, routing, budgets and resume | Resume distinguishes a saved budget from a failed resume; list/detail failures use LoadState. Goal-form regressions have existing browser specs but were not executed here. |
| API | `ApiPage.svelte`, `EnvironmentsView.svelte`, selected store operations | Request tabs and automation leave paths are guarded. Environment editor is the missing branch (UX-01). |
| Database | `DatabasePage.svelte`, `QueryEditor.svelte`, selected database store methods | Query close keeps a bounded reopen stack; pending cell edits ask before discard; connection state has generation checks and draft persistence. No new defect asserted from this bounded read. |
| Brokers | `ReplayPanel.svelte` | Replay has meaningful numeric validation, destination confirmation, busy state, results and failure feedback. No claim that every broker tab was reviewed. |
| School Home | `ClassroomsBox.svelte`, `school/model.ts`, `school/actions.ts`, relevant scene keys | Delete explains permanent history loss, archive has recovery, actions check manage rights, stale session status is derived centrally. Complete fallback access and remembered room fail (UX-03/04). Design reviewer separately covers keyboard/focus and LoadState issues. |
| Team Performance | `ui/views/app.js`, `settings.js`, shared components, scan routes and browser-test inventory | Useful onboarding, paced scan stages, freshness, per-section Retry and keyboard tabs exist. Settings ownership and scan recovery remain incomplete (UX-02/05/06). |

## Findings

### UX-01 — P1: environment edits disappear on ordinary navigation

**Evidence:** `ui/src/modules/api/EnvironmentsView.svelte:41` stores rows/dirty locally; its only selection discard check is `:67–71`. `ui/src/modules/api/ApiPage.svelte:72–77` protects only an automation editor before switching views. `:401` closes the environment tab through `showRequest`, and `:408–409` conditionally mounts the environment component. `EnvironmentsView.svelte:148–155` also creates and seeds a new environment without consulting `dirty`.

**Scenario and trace:** edit a base URL or add several variables; click an existing request tab, close Environments, or create another environment. The component is unmounted or `seed()` replaces the rows; there is no save, retained draft or discard decision. The saved API environment remains unchanged, so returning cannot recover the typed values. Module navigation likewise has no environment leave guard.

**Fix direction:** give the environment editor the same explicit `approveLeave` integration as AutomationEditor, register the shared module/workspace leave guard, and guard creation/selection changes. Preserve the current draft on Keep or failed Save. Do not persist secret plaintext into localStorage to solve this.

**Acceptance:** dirty variables survive cancelled request-tab, close-tab, sidebar and New environment transitions; Save waits for success before leaving; failed Save keeps rows; confirmed Discard leaves; untouched views do not prompt. Include secret rename/value edits without asserting their plaintext in logs.

### UX-02 — P1: Team Performance refresh silently replaces an unsaved settings form

**Evidence:** `examples/plugins/team-performance/ui/views/app.js:536–563` empties and reconstructs the active view. `:431–434` invokes this via `refresh()` when a scan completes; `:490–494` does so on tab navigation. Settings reconstructs its state from the server in `ui/views/settings.js:26–45`, and reads the editable DOM only when Save is clicked. `settings.js:247–256` disables only Save, then refreshes the entire view after its requests finish.

**Scenario and trace:** start a scan, open Settings, change time off, aliases or the estimator instructions, and let the scan finish. Refresh destroys those unsaved controls and reloads server values. Switching tabs/scope has the same effect. A second variant: edit another field while Save is awaiting its requests; the completion refresh removes the newer edit even though it was not in the submitted payload. There is no draft owner, dirty check or leave hook in these paths.

**Fix direction:** keep a settings draft independent of DOM rendering and keyed to its account, with a submitted snapshot and a dirty revision. Refresh read-only metrics without reconstructing a dirty editor. Explicit user navigation should preserve the draft or offer Save/Discard/Keep. Reconcile only the submitted revision after Save.

**Acceptance:** type in Settings during a delayed scan and during a delayed Save; both newer edits remain afterward. Exercise tab/scope navigation and Jira people refresh. Retry a failed Save without losing any inputs.

### UX-03 — P2: School's List fallback inherits the 3D seating cap

**Evidence:** `ui/src/modules/home/school/model.ts:398–416` selects at most 36 front-row and six background sessions into `room.kids`. `:434–439` counts all sessions, including unseated ones. `ui/src/modules/home/boxes/ClassroomsBox.svelte:716` renders the accessible/list rows from that capped array, and `:738` only prints the remaining count.

**Scenario and trace:** a workspace has seven engine sessions; the newest needs input. The room's summary counts that attention state, but the seventh session has no kid, list row, open action or menu in either School mode. The fallback cannot deliver the action promised by the summary. The same applies beyond 36 regular sessions.

**Fix direction:** retain a complete session collection for the accessible list while keeping the 3D geometry capped. Show all unseated sessions in List or provide a clearly labeled overflow action opening that complete list. Preserve the needs-you identity and Open session action.

**Acceptance:** seed 37 regular and seven background sessions, with the overflow members needing input. Both are discoverable and actionable in List and via the 3D overflow route. The 3D geometry remains bounded.

### UX-04 — P2: mounting School clears the saved room before restoring it

**Evidence:** `ui/src/modules/home/boxes/ClassroomsBox.svelte:139` initializes `view` to corridor. `:213–216` immediately persists `room: null` for that initial view. The old room is only read after asynchronous `mountSchool()` resolves at `:173–176`. `ui/src/modules/home/home.svelte.ts:290–294` replaces the box config synchronously through `patchView`.

**Scenario and trace:** enter a room, leave Home, and return (or reload). On remount, the persistence effect clears the previous room while assets load. By the time the mount completion tries to restore the room, the reactive box config contains null. The user lands in the corridor despite the explicit remembered-room behavior in the source.

**Fix direction:** capture the saved room before the first persistence effect; suppress room persistence until initial restoration has completed or the user intentionally navigates. A failed scene load must not erase the preference.

**Acceptance:** enter a non-default room, remount/reload with delayed asset loading, and verify the room is restored. Switching intentionally to the corridor persists corridor. List-mode visits do not inadvertently clear a saved 3D room.

### UX-05 — P2: scan connectivity failures leave an indefinitely fresh-looking running state

**Evidence:** `examples/plugins/team-performance/ui/views/app.js:418–420` swallows every scan-status read failure and returns without changing presentation. `:423–425` updates the disabled Scan control, label and progress only after successful reads. The running status markup at `:398–410` has no last-success timestamp or reconnecting state.

**Scenario and trace:** start a scan, then lose plugin/daemon connectivity. The last stage and “Scanning…” remain, with Scan disabled, for arbitrarily many failed polls. The view communicates running progress without knowing whether the scan is alive; there is no visible recovery explanation.

**Fix direction:** retain the last known stage but mark it stale after a bounded freshness interval, display last update and reconnecting/error text, and offer an explicit status Retry. Clear the stale state on the next successful read. Keep polling without fabricating failure or completion.

**Acceptance:** fail consecutive status reads after a running response, verify stale/reconnecting copy, then restore a terminal response and verify recovery and the enabled Scan control.

### UX-06 — P2: a long Team Performance scan has no Stop action

**Evidence:** `examples/plugins/team-performance/ui/views/app.js:362–376` starts the scan; `:398–410` draws only progress; `:423–424` disables the sole Scan action while running. Server scan routes at `examples/plugins/team-performance/server.js:2750–2797` provide start and status; the route inventory has no scan cancellation operation. A source search over scan UI/server found no scan Stop/cancel path.

**Scenario:** a user selects the wrong scope or starts a long, throttled full rescan and wants to stop it. They can only wait or stop the whole plugin, losing normal app-level control. The product's long-running-work guideline explicitly requires reachable Stop/Cancel.

**Fix direction:** add cooperative scan cancellation at safe boundaries (including pacing/backoff and estimator dispatch), retaining already persisted results, with an honest stopped state and a reachable Stop button. Do not merely abort the status fetch and label the server job stopped.

**Acceptance:** stop during pacing and between projects, verify no further work dispatches, committed results remain readable, UI reaches a stopped state, and a later scan starts successfully. This needs backend and contract coverage as well as browser evidence.

## Provisional score

Scored only for this source-inspected scope. Five dimensions × 2; the runtime gaps prevent a validated 9.8+ claim.

| Dimension | /2 | Reason |
| --- | ---: | --- |
| Task completion and control | 1.55 | Strong automation controls; plugin scan cannot be stopped. |
| Recovery and status honesty | 1.50 | Inline recovery in core screens; scan polling can silently stale. |
| Draft and work preservation | 1.25 | Established guards are strong, but two directly reachable form-loss paths remain. |
| Navigation and continuity | 1.55 | Deep links and selection memory exist; School restoration and overflow fail. |
| Accessible alternatives and interaction clarity | 1.65 | Real controls and fallback intent are present; School fallback is incomplete; actual keyboard/visual behavior not validated here. |
| **Total** | **7.50 /10** | **Provisional source-based assessment, not a measured app-wide score.** |

## Acceptance matrix and runtime gaps

Existing tests below are regression anchors, not claims of passing executions in this review.

| Flow | Existing anchor | Add / execute next |
| --- | --- | --- |
| Environment draft ownership | `ui/e2e/desktop-review6-api-environment.spec.ts` (delayed Save/selection); `desktop-api-automation-leave.spec.ts` (guard pattern) | UX-01 navigation/create/failed-Save cases. |
| Plugin settings | `examples/plugins/team-performance/test/browser.e2e.test.js:314` (time-off Save) | UX-02 delayed scan and Save, account/scope transitions; retain inputs after failure. |
| School list / actions | `ui/e2e/desktop-home-classrooms.spec.ts` | UX-03 cap overflow and needs-you discovery, keyboard open, no-WebGL fallback. |
| School continuity | `ui/e2e/desktop-home-classrooms-3d.spec.ts` (scene, actions, Escape/Enter, dark) | UX-04 remembered room with delayed mount; coordinate separate design keyboard findings. |
| Plugin scan recovery/control | Plugin browser/server e2e tests | UX-05 failed polling/reconnect; UX-06 cooperative cancellation and restart. |
| Scheduled task forms | `ui/e2e/desktop-review5-scheduled-draft.spec.ts`, `desktop-scheduled-tasks.spec.ts` | Run fresh baseline; keep delayed-save/navigation/delivery recovery cases. |
| Goal Loops | `ui/e2e/desktop-goal-draft-ownership.spec.ts`, `desktop-goal-loop-form.spec.ts` | Run fresh baseline; verify failed resume retains the already saved budget. |
| Workflows | `ui/e2e/desktop-workflow-recovery.spec.ts`, `desktop-workflow-runview.spec.ts` | Run fresh baseline; draft leave, preflight, failed output Retry, run Stop. |
| Database / Brokers | `desktop-database-changes.spec.ts`, `brokers-mobile.spec.ts`, `brokers-sweep.spec.ts` | Pending cell-edit discard, close/reopen SQL, failed loads, replay confirmation/outcome. |

Still needed: fresh browser execution against the updated daemon; light/dark desktop and phone screenshots; keyboard-only and VoiceOver spot checks; zoom/RTL; plugin iframe navigation and error-state accessibility; real School WebGL context loss and delayed assets. No visual-layout, contrast, performance or runtime-pass claims are made by this report.
