# Swarm + Mission Control — design review

Code review of `ui/src/modules/swarm/` and `ui/src/modules/mission-control/` against `docs/design/guidelines/`. Not a rendered light/dark/phone pass (no `npm`, no screenshots). Tokens are used; no hex, no native `confirm`/`prompt`/`alert`, shared `PageHeader` / `EmptyState` / `Modal` / `confirmer` / `ctxMenu` are in place. Scores are fitness against a 9.8 bar, not a count of rules already met.

| Module | Score | Defects |
|---|---|---|
| Swarm | 8.4 | 8 |
| Mission Control | 8.6 | 4 |
| **Total** | | **12** |

`RunGraph.svelte` is never imported. The Graph tab is the org chart (`AgentGraph`). Unscored.

---

## Swarm — 8.4

Lifecycle, abort confirm, budget sheet, rail accordion, runs error/empty, and phone run cards are solid. These are the gaps.

### 1. Planning Stop (and bulk actions) can be clipped off a phone

`ui/src/modules/swarm/KanbanBoard.svelte:469-496` and `:674-681`; clipped by `ui/src/modules/swarm/SwarmPage.svelte:1053-1057` (`.view { overflow: hidden }`).

The board toolbar and bulk bar are single non-wrapping rows. While planning, the row is “Planning… watch live in Runs” + Stop… + ⋯ + Add task. That is wider than 320px, and the pane hides overflow instead of scrolling. **Impact:** on a phone the control that stops planner agents is off-screen. The same clip hides Move / Assign / Delete after a multi-select. **Fix:** `flex-wrap` both bars; truncate the planning status (`min-width: 0`); keep Stop… `data-keep` on the first line.

### 2. Run budget and Max parallel sit off-screen on a phone

`ui/src/modules/swarm/SwarmPage.svelte:691-723` and `:1162-1187`.

The five view tabs, both budget meters, and Max parallel share one nowrap row. Phone CSS hides the scrollbar. Tablet already wraps this (`:1113-1124`); phone undoes it. **Impact:** a budget pause is explained in the header, but the meters and the parallel cap are only reachable by sideways-swiping the tab strip, with no cue. **Fix:** second row for `.budget-bars` + `.cap` at ≤640px; scroll only `.seg-tabs`.

### 3. A failed task load looks like an empty board

`ui/src/lib/stores/swarm.svelte.ts:546-579` swallows the error after one retry. `ui/src/modules/swarm/KanbanBoard.svelte:529-543` then renders five zero-count columns. `createTask` (`swarm.svelte.ts:586-593`) POSTs, then reloads; a failed reload never puts the new row in `tasksByProject`. **Impact:** the board says the project has no tasks. Add task can succeed on the server and still vanish from the UI. **Fix:** a `tasksError` on the store; `LoadState` + Retry in the board; on create failure after POST, insert the returned task locally.

### 4. A trigger can reply in every chat with no outward confirm

`ui/src/modules/swarm/SwarmSettings.svelte:247-281`.

Match chat and keyword default to blank (“any”). Reply and Auto-start default on. Save is “Add trigger”, not a confirm that names channel, chat, and who will see the reply. **Impact:** one save arms the swarm to start and post into Slack/Telegram for any message. **Fix:** require a chat when Reply is on, or confirm with where / what / who before save. Default Reply off.

### 5. “Verify now” can be stopped without the Stop pattern

`ui/src/modules/swarm/GoalsPanel.svelte:100-102`.

Plan-from-goal uses a danger `Stop…` plus `confirmer` (`KanbanBoard.svelte:319-331`). Goal verify uses a plain “Stop” and calls `stopVerify` immediately. **Impact:** one click kills an in-flight check; already-passed goals stay, in-progress ones don’t. **Fix:** `btn small danger`, label `Stop…`, same confirm copy as planning.

### 6. Goal chips fail type and contrast

`ui/src/modules/swarm/GoalsPanel.svelte:193-199` and `:223-226` and `:251-262`.

Status and kind are `text-transform: uppercase` (only `.section-title` may be). Metric keys are `--text-dim` at `opacity: 0.7`, under the 11px contrast floor in both schemes. **Impact:** “PASSED” / “metric” are harder to read than the rest of the app, and the measured value’s label is the faint part. **Fix:** sentence case via `StatusBadge`; delete the opacity.

### 7. Graph click on an idle agent only toasts

`ui/src/modules/swarm/AgentGraph.svelte:200-204` and `:342`.

Org opens the editor from the name (`OrgTree.svelte:323`). Graph’s whole card is “Open session”. With no session it toasts and offers no Edit. The brief rail uses the same handler (`:381`). **Impact:** Graph looks like a second org view but cannot hire, edit, or reparent; the primary click is a dead end for anyone not already running. **Fix:** card menu — Edit agent, Open session (disabled with a reason when none), Add report. Don’t toast as the only result.

### 8. Preset failure looks like “only Blank exists”

`ui/src/modules/swarm/NewSwarm.svelte:18-20` and `:43-55`; load in `ui/src/lib/stores/swarm.svelte.ts:945-951` (`catch { this.presets = [] }`).

No loading row, no error, no Retry. **Impact:** a daemon blip shows a blank team as the only start, and Create still works, so the user never learns the presets failed. **Fix:** `LoadState` in the preset list; keep Blank selectable; Retry re-fetches.

---

## Mission Control — 8.6

List/detail selection, filter empty vs load error, stale-refresh banner, stop confirm, and approval actions match the checklist. These do not.

### 1. Graph status is color alone

`ui/src/modules/mission-control/WorkGraphView.svelte:99-103`.

Node fill is `statusColor()` (`lib.ts:110-111`). The only visible text is a truncated title. Needs-approval is a dashed ring; status is not. The accessible name includes status; the picture does not. **Impact:** failed vs running vs succeeded is hue-only, including on the light scheme where green/teal/grey sit close. **Fix:** status icon inside the circle (the unused `RunGraph` badge pattern) plus `STATUS_LABEL` on a second line. Focus stroke should be `--accent-text`, not `--accent` (`:156-159`).

### 2. First paint of Graph says the graph is empty

`ui/src/modules/mission-control/MissionControlPage.svelte:369-401`. The graph starts `{ nodes: [], edges: [] }` and is fetched only after the tab flip (`:194-198`). The branch shows `WorkGraphView` whenever `items.length > 0`, and that view treats zero nodes as empty (`WorkGraphView.svelte:51-52`). **Impact:** switching to Graph flashes “Nothing to graph” while the request is in flight, including when the list already has rows. A filter change also keeps the previous graph up with no busy state (`reload` does not clear `graph`). **Fix:** if `loading && graph.nodes.length === 0`, show the same skeleton as the list. Dim the svg and set `aria-busy` while a refetch is in flight.

### 3. On a phone the row title collapses to nothing

`ui/src/modules/mission-control/WorkItemList.svelte:76-151` and `:184-192`.

`.wi-meta` is `flex: 0 0 auto` and does not wrap. The title is the only `min-width: 0` child. ≤640px hides cost and time but keeps “Needs approval”, the status badge, and “Critical risk”. **Impact:** a 320px row with those chips gives the title 0px. The user sees badges and no name. **Fix:** at ≤640px, stack meta under the title (`flex-wrap` or a two-row grid) and let the title use the full row.

### 4. Phone/tablet detail covers the shell and the notch

`ui/src/modules/mission-control/MissionControlPage.svelte:552-567` (`position: fixed; inset: 0; z-index: var(--z-modal)`). Close is a 24px `.icon-btn` (`WorkItemDetail.svelte:359`).

The sheet is above the mobile top bar (`z-index: 5`) and BottomNav, and it does not pad `safe-area-inset-top/bottom`. There is no back control in `PageHeader`’s `leading` slot. **Impact:** on a notched phone the only exit sits in the status-bar band; the last timeline rows sit under the home indicator. **Fix:** pad the sheet with both safe-area insets; make Close a text “Back” button at `min-block-size: var(--hit-min)` on coarse pointers, placed below the top inset.

---

## Not defects

Paused / aborted / budget are named in the header (`SwarmPage.svelte:518-549`), abort confirms with who is stopped, and a budget pause routes through Raise budget rather than a resume that would immediately re-pause. Mission Control distinguishes load error, filter empty, and stale refresh. Both pages use real buttons, `confirmer`, and token colors. No light-vs-dark token split showed up in these files.
