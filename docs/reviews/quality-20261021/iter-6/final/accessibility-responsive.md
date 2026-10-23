# Quality iteration 6, final review: accessibility and responsive layout

Scope: the Svelte 5 UI in `ui/src`, worktree `claude_ade-iter6`, branch `fix/quality-iter6-20261023` at
`2065f00e9`. That is the same tree wave 3 scored. The only later changes are untracked review files. Excluded:
`target/`, `node_modules/`, `docs/reviews/`. This was a read-only review using rg, grep, sed, git and small
read-only Python scanners in the scratchpad. No builds, tests, guard runs or e2e runs.

## Score

Rubric: start at 10.0, then subtract 1.0 per blocker, 0.3 per major, 0.1 per minor and 0.03 per nit, for each
confirmed open finding.

| Severity | Open | Deduction |
|---|---|---|
| blocker | 0 | 0.00 |
| major | 0 | 0.00 |
| minor | 4 (6f-01, 6f-03, 6f-04, 6f-05) | 0.40 |
| nit | 1 (6f-02) | 0.03 |

**Score: 10.0 − 0.40 − 0.03 = 9.57 / 10**

Earlier scores: wave 3 9.87, wave 2 9.87, wave 1 9.60, wave 0 8.87.

Why this is lower than wave 3: no fix landed after the wave-3 review, so both wave-3 findings are still open.
The fresh pass also confirmed three more problems in areas the earlier waves had swept by sampling:

- **6f-03:** focus after Move up/down in QueryBuilder and Appearance.
- **6f-04:** a second set of controls that remove themselves on activation.
- **6f-05:** the MetricChart tooltip, which flips with no floor.

None of the three is a regression from this iteration. Every wave-0 finding (6-01 … 6-10) and every wave-1 and
wave-2 finding is still fixed.

## Previous findings (wave 3)

| ID | Status | Evidence |
|---|---|---|
| 6w3-01 Four status regions are inserted with their text, three or more mounting lines into a branch; the guard misses two and baselines two | **Still open** (carried as 6f-01) | No change since `c2ba794ee`. `SessionView.svelte:979` still has `<span class="ui-ask-text" aria-live="polite">` under `{#if uiPrompt}` (`:973`), and `uiControl.announce()` still returns when `onScreen` (`lib/stores/uiControl.svelte.ts:217`). `App.svelte:198` `boot-sub role="status"`, `DiagramView.svelte:442` `trunc-banner role="status"` and `StructureView.svelte:640` `st-head role="status" aria-live` are unchanged. The guard is still `taken < 2` (`ui/scripts/ui-guards.mjs:1031`). The baseline still accepts DiagramView and StructureView (`ui-guards-baseline.json:553-556`). |
| 6w3-02 The assistant's limit line speaks a stale limit when a thread opens | **Still open** (carried as 6f-02) | `modules/assistant/cards/SystemLine.svelte:40` still has `use:announceShown={turn.text}`. It is rendered for every system card at `ChatView.svelte:203` and `TaskEntry.svelte:34,42`. `ChatView`'s thread-switch guard (`:96-113`) does not cover it. |

The wave-0/1/2 fixes were spot-checked and are still in place:

- the `announce`, `announceEdge` and `announceShown` module (`lib/announce.svelte.ts`), with an always-mounted
  `Announcer.svelte` in `App.svelte`;
- `focusAfterRowRemoval`, plus AutomationEditor's `movedArrow` (`api/automationInput.ts:58`) and AggregateBuilder's
  `move()` (`AggregateBuilder.svelte:162-173`), which move focus after a move;
- `caseOrder.svelte.ts` `move()`, which falls back off a disabled edge button;
- `viewportOwnsKey`, LinkPreview Escape, the FileTree roving `tabRow`, Inspector `go()`;
- `focusAppContent()` on the Close and Dismiss buttons swept in wave 3;
- the `app.css:844` reduced-motion override.

## Areas covered

- Re-verified both wave-3 findings (table above), at every listed location.
- **Inserted live regions.** A fresh scanner looked at the twelve lines after every `{#if}`, `{:else}`, `{#each}`,
  `{#await}` and `{#key}` opener for `role="status|alert|log"` or `aria-live`. It produced 114 hits, and every
  `status` or `aria-live` hit was read in context.
  - The untoned `role="alert"` hits are field, form or load errors. The repo policy allows these (the
    `inserted-alert` ratchet, with WebKit speaking an inserted alert).
  - Regions that are mounted before their text changes are fine: `NewSession:616`, `PrList:240`, `MemoryTab:194`,
    `WipPanel:885`, `RoomAnnotations:80`, `AgentDrivingBar:73`, `FloatingBar:821`, `SchoolList:118`,
    `LoadState:103`, and the `ModelBreakdown:168`/`UsagePage:942` day readouts, which update on each select change.
  - The remaining hits are 6f-01.
- **Icon-only buttons.** A scanner checked every `<button>` with no text for `aria-label` and `title`. It found
  none missing. The only hits are an image named by its `alt`, `aria-hidden` scrubber markers with a chapter
  list, hidden submit buttons, and a tabindex −1 caret.
- **Clickable non-buttons.** 37 hits, all either a presentation backdrop, or a separator that gets `tabIndex=0`
  and key handlers from `use:splitter` (`lib/paneResizer.ts:142-152`), or a table row with `tabindex` and keys, or
  a row whose inner button is the keyboard path (`MonitorCluster:548`).
- **Custom dialogs.** `role="dialog"` and `alertdialog` outside Modal: `ApprovalCard` and `PageDialogCard` move
  focus to the card, and RefPopover uses `dialogFocus`. Tray, BarHost and the MockupAnnotations note were read.
- **Floating UI.** Every coordinate-positioned element was read:
  - BrowserPanel popover: clamped (`:428-430`);
  - SiteCanvas toolbar: clamped (`:472-473`);
  - vault graph panel: `max-width`/`max-height`;
  - NamespacePicker, NotificationBell, ContextMenu, RefPopover: as in wave 3;
  - MetricChart tooltip: 6f-05.
- **RTL.** Every physical `left`/`right`/`margin-*`/`border-*` declaration is either marked `ui-guards: allow`
  with a reason (physical canvas or projection space, or centred with translate) or is a mirrored arrow
  (`RunStageRail`).
- **Reduced motion.** The global override (`app.css:844`), no `.animate()`, no smooth-scroll literals, and a
  Stop layout control on the vault force graph (`GraphView.svelte:1383`, for WCAG 2.2.2).
- **Single-key shortcuts.**
  - The k8s workspace is scoped to focus in the workspace, can be turned off (`singleKeys`) and skips text
    fields (`ClusterWorkspace.svelte:313-405`).
  - The Scene3D viewport gates on focus.
- **Reorder and remove focus.** Every Move up/down control in `ui/src` was checked: 6f-03.
- **Self-removing controls.** Every inline Dismiss, Close and Cancel whose handler only clears its own condition:
  6f-04. Buttons in Modal footers are excluded, because Modal returns focus.
- **This iteration's diff.**
  - The new files (`Announcer`, `S3TransferBars`, `LinkListField`, `CommitDetailPane`, `RefChip`, `RefPopover`,
    `RefSideLists`, `RefSidebar`, `DoorBubbles`, `DraftEditor`, `JiraIssueSection`, `StoryTranscripts`, shooter
    `Knocked`, `MatchResults` and `PauseMenu`, the workflows `Run*` panels).
  - Fixed widths of 300 px or more added in the diff. Only `CommitDetailPane`'s `min-width: 360px` was found, and
    it is overridden on mobile.
  - The MetricChart and QueryBuilder diffs.
- **Phone widths** for `share/SharePage`, `personal-agents/AgentPage` and `swarm/RunInspector` (static: capped
  widths, wrapping tab strips, `overflow-x` wrappers). This was listed as not covered in wave 0.

## Areas not covered

- Runtime behaviour in WebKit. Two questions need a live screen reader or e2e and were not tested:
  - whether a keyed `{#each}` DOM move keeps focus on a moved row that is not at the list edge;
  - whether VoiceOver speaks each inserted `role="alert"`.
- Touch target size (WCAG 2.5.8) and focus-not-obscured (2.4.11). Neither was swept as a class.
- Contrast was checked only through the token guards (`color-literal`, `accent-text`, `focus-ring-token` all at
  0). No measured contrast ratios.
- The Git graph commit list keyboard model and the Navigator manual session order. Both are pending design
  decisions (wave 0) and are not scored.
- Everything outside `ui/src`. The Tauri shell's native menus are not in this lens.

## Findings

### accessibility-responsive-6f-01: minor (carries 6w3-01). Status regions are still inserted with their text, beyond the guard's two-line window and inside `{#each}`

Each of these regions mounts together with its words, so WebKit/VoiceOver does not speak it
(`lib/announce.svelte.ts:3-8`):

- `ui/src/modules/agents/SessionView.svelte:973-981`: under `{#if uiPrompt}` come a comment, then
  `<div class="ui-ask" role="group">`, then the mark span, then `<span class="ui-ask-text" aria-live="polite">`
  "… wants to drive Otto — …". This is a permission request with Allow and Deny. The store's toast fallback skips
  it when the session is on screen (`lib/stores/uiControl.svelte.ts:216-217`, `if (onScreen) return;`). So a
  screen-reader user who is looking at the session is never told that the agent is asking for control. The guard
  misses this site, because the region is on the third mounting line.
- `ui/src/App.svelte:195-198`: `{:else if auth.phase === 'loading'}` → `<div class="boot">` → `boot-mark` →
  `<div class="boot-sub" role="status">Connecting to the Otto daemon…</div>`. It is silent when the phase goes
  from offline back to loading. The guard misses this site too.
- `ui/src/modules/database/DiagramView.svelte:438-442`: `{#if graph?.truncated}`, three comment lines, then
  `<div class="trunc-banner" role="status">`. This is a static note. The baseline accepts it
  (`ui-guards-baseline.json:554`).
- `ui/src/modules/database/StructureView.svelte:636-640`: `{#if database.objectLoading}`, a comment, then
  `<div class="st-head" role="status" aria-live="polite" aria-label="Loading structure">`. The baseline accepts it
  (`:555`).
- `ui/src/modules/assistant/NeedsYouRail.svelte:90-94`: `{#each limited as l …}<p class="limit" role="status">`
  "{provider} limit reached …". A limit that arrives while the rail is open is inserted with its text, so it is
  silent. The guard does not look at `{#each}` openers.

Fix:

1. `SessionView.svelte:979`: remove `aria-live`, and put
   `use:announceShown={`${agentWho} wants to drive Otto: ${uiAskWhat}`}` on `.ui-ask`. Or make
   `uiControl.announce()` call `announce()` when the session is on screen, instead of returning.
2. `App.svelte:198`: remove the role, and `announce('Connecting to the Otto daemon…')` on the `loading` edge in
   the existing effect (`:83-97`).
3. `DiagramView.svelte:442`: remove the role.
4. `StructureView.svelte:640`: let a `Skeleton label="the structure"` be the region, and update the e2e spec
   that reads the role.
5. `NeedsYouRail.svelte:91`: remove the role, and announce a limit only when it is new (`announceEdge` over the
   joined provider list).
6. Guard: skip close-only lines when counting `taken` (or raise the window to 3), add `{#each` to
   `BRANCH_OPENER`, then run `--update-baseline --only inserted-live-region` so the baseline is empty.

### accessibility-responsive-6f-02: nit (carries 6w3-02). The assistant's limit line speaks an old limit from history when a thread opens

- `ui/src/modules/assistant/cards/SystemLine.svelte:38-40` has `use:announceShown={turn.text}`, and the action
  speaks on mount (`lib/announce.svelte.ts` `announceShown` → `run(param)`).
- `SystemLine` is rendered for every system card in the timeline (`ChatView.svelte:203`) and in task entries
  (`TaskEntry.svelte:34,42`). So opening or switching to a thread speaks its newest past limit, for example
  "Claude usage limit reached …" from days ago.
- This contradicts ChatView's own rule (`:96-113`): "opening an idle thread doesn't announce".
- Fix: remove the action from `SystemLine`. In `ChatView`, announce a `limit` card only when it is appended to the
  current thread after the first render of that thread id (reuse the `announcedFor` guard).

### accessibility-responsive-6f-03: minor. Move up/down in QueryBuilder and Appearance does not manage focus, so the pressed button is disabled at the list edge and focus falls to `<body>`

The codebase already has the right pattern in three places:

- `AutomationEditor.moveStep` with `movedArrow` (`api/AutomationEditor.svelte:129-141`, `api/automationInput.ts:55-62`);
- `AggregateBuilder.move` (`database/AggregateBuilder.svelte:162-173`);
- `caseOrder.move` (`product/caseOrder.svelte.ts:94-117`, which notes "A Move button that just went disabled
  (top/bottom) can't hold focus").

These lists skip it:

- `ui/src/modules/database/QueryBuilder.svelte:1162-1163` (SELECT list, `moveSelect` `:438-444`) and `:1262-1263`
  (ORDER BY, `moveOrder` `:561-567`).
  - Problem: the buttons are `disabled={i === 0}` / `disabled={i === … .length - 1}`, and the move functions only
    swap the array. They have no `tick()`, `focus()` or `announce()`.
  - Example: pressing "Move up" on row 2 makes it row 1, and its now-disabled button drops focus.
  - Every row's buttons are named a generic "Move up" / "Move down", unlike AggregateBuilder's
    "Move stage {i + 1} ({op}) up".
- `ui/src/modules/settings/Appearance.svelte:484-499`: the sidebar item buttons, and `:449-465` the section
  buttons. `moveSidebarItem` (`:68-76`) and `moveSidebarSection` (`:78-82`) only write the store, and the buttons
  disable at the edges (`disabled={i === 0}`, `disabled={si <= 0}`).
- Impact: WCAG 2.4.3 (focus order). A keyboard user who moves an item to the top or bottom loses their place.
  The move is also never announced.
- Fix: give the rows `data-*` ids. After the swap, `await tick()`, then focus the moved row's arrow, or its other
  arrow when `movedArrow` says the edge disabled it, and `announce('Moved to position n of m')`. Name the
  QueryBuilder buttons after their row ("Move column {i + 1}: {selectLabel(it)} up", "Move sort {i + 1} up").

### accessibility-responsive-6f-04: minor. More controls remove themselves on activation with no focus move

These are inline, not in a Modal footer. Each handler only clears the condition that renders the button, so focus
falls to `<body>` (WCAG 2.4.3). This is the 6-09 class. Wave 3 fixed the sites it swept with `focusAppContent()`
(`lib/announce.svelte.ts:42-45`). These sites were missed:

- `ui/src/modules/mcp/TokensPanel.svelte:195`, `:210`: "Dismiss" sets `createdToken` / `rotatedToken = null`,
  which unmounts the whole `token-once` card (`:186`, `:201`).
- `ui/src/modules/usage/OttoUsage.svelte:169`: "Dismiss error" sets `error = ''`.
- `ui/src/modules/vault/VaultPage.svelte:485`: "Dismiss" sets `vault.noteOpenError = null` and removes the
  `note-open-error` banner (`:482`).
- `ui/src/modules/database/QueryBuilder.svelte:1123`: the import-error "Dismiss" icon button sets
  `importError = null`.
- `ui/src/modules/rooms/RoomPage.svelte`:
  - `:191` "Dismiss" (`error = ''`);
  - `:210` "Dismiss" inside the invite Modal body (`inviteError = ''`);
  - `:186` "Take back control": `{#if host && !canType}`, and the echo makes `canType` true. The comment at
    `:187-188` fixes this exact problem for Release/Request, but not here.
  - `:183` "Reconnect": the banner unmounts once `connected` (noted, but not scored, in wave 3).
- `ui/src/modules/aws/S3TransferBars.svelte:41` "Cancel" → `dl?.ctrl.abort()` (`S3Browser.svelte:770`). The
  download bar unmounts when `dl` clears.
- `ui/src/modules/settings/Appearance.svelte:474-480`: the "Favorite {label}" toggle moves the row between the
  Favorites section and its own section. These are separate `{#each sec.modules}` blocks (`lib/sidebar.ts:322-333`
  removes favorites from their group), so the focused button is destroyed and recreated somewhere else.

Fix:

- For Dismiss and Cancel, call `void focusAppContent()` after clearing. In a Modal, focus the dialog's first
  field instead.
- For "Take back control" and "Reconnect", use one always-mounted button whose label and action swap (as
  `:187-189` does), or `focusAppContent()` on the edge.
- For the Favorite toggle, `await tick()` and focus `[data-sidebar-row="{id}"] .sb-star` in its new section.

### accessibility-responsive-6f-05: minor. The MetricChart tooltip flips at 55% of the chart with no floor, so on a narrow chart it runs off either edge and the values are clipped

- `ui/src/lib/components/MetricChart.svelte:276-277`: `tipLeft = hover.px / W * 100` and
  `tipFlip = hover.px > W * 0.55`. The tip at `:347` is `position: absolute` with `min-width: 120px`,
  `max-width: 240px` and `translateX(8px)`, or `translateX(calc(-100% - 8px))` when flipped (`:439-456`). The
  comment at `:274-275` claims it "never leaves the chart's own box (which is what the parent's overflow clips
  to)". But:
  - not flipped, its right edge reaches `0.55W + 248`, which leaves a chart narrower than about 550 px;
  - flipped, its left edge reaches `0.55W − 248`, which goes negative below about 450 px.
- MetricChart is used on phone and tablet widths and in narrow side panels (`usage/OttoUsage`,
  `aws/MetricsPanel`, `kubernetes/monitor/MonitorFleet`). There, the right-hand value column (`.val`) of a hover
  or keyboard readout near the middle is cut off by the parent's overflow.
- This breaks the AGENTS.md floating-UI rule: never flip without a floor, and clamp into view. Keyboard users get
  the same tip from `onKey` → `hoverAt` (`:241-262`). Screen-reader users get the `sr-only` readout, so the loss
  is visual only.
- Fix: measure the tip (`bind:clientWidth`) and set
  `left = clamp(px + 8 or px − 8 − tipW, 4, W − tipW − 4)` in px instead of a percentage plus a translate. Or cap
  `max-width` at `calc(100% - 16px)` and clamp the same way.
