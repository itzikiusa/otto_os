# Quality iteration 4: accessibility and responsive layout

Scope: the Svelte 5 UI in `ui/src`. Excluded: `ui/src/modules/rooms/games/**`, `ui/src/modules/home/school/**`,
`ui/src/modules/home/boxes/ClassroomsBox.svelte`, `target/`, `node_modules/` and `docs/reviews/`. This was a read-only
review using grep, rg, sed, git and `python3 -I` text scans. No builds, tests or e2e runs.

## Score

Rubric: start at 10.0, then subtract 1.0 per blocker, 0.3 per major, 0.1 per minor and 0.03 per nit.

| Severity | Open | Deduction |
|---|---|---|
| blocker | 0 | 0.0 |
| major | 3 (4-01, 4-02, 4-03) | 0.9 |
| minor | 6 (4-04 … 4-09) | 0.6 |
| nit | 0 | 0.0 |

**Score: 10.0 − 0.9 − 0.6 = 8.5 / 10**

Iteration 3 scored 8.1. All 13 findings it left open are fixed in the current code (table below). The findings in
this pass come from areas no earlier pass reviewed: live regions that hold a ticking clock, inline Edit ↔ editor swaps
across Product, Git PR and Mission Control, the New session provider grid's ± controls, the remaining room host
controls, the leftover Skeleton wrappers that the 3-04 fix did not reach, and keys typed inside the schema tree.

## Previous findings

| ID | Status | Evidence |
|---|---|---|
| 3-01 Home window radios | **Fixed** | `K8sBox.svelte:118`, `UsageBox.svelte:110`: roving `tabindex` + `onkeydown={radioKey}`. `HomePage.svelte:64` skips `[role="radiogroup"],[role="tablist"]`. |
| 3-02 SchemaTree hierarchy and keys | **Fixed** | `SchemaTree.svelte:688-692` `aria-level`/`aria-posinset`/`aria-setsize`; `.node-label` and caret `tabindex="-1"` (`:700`, `:714`); `onTreeKey` resolves `closest('[role="treeitem"]')` (`:342`); search and counts moved out of `role="tree"` (`:530`, `:647`). One residual is new finding 4-07. |
| 3-03 Room chat and admit/decline focus | **Fixed** | `RoomChat.svelte:73-79` one always-mounted submit; `:25-36` ⌘↵ and refocus after the echo. `RoomParticipants.svelte:19-35` `decide()` refocuses the member's new first control or the heading. The other host controls are new finding 4-05. |
| 3-04 Skeleton busy region, nested wrappers | **Fixed at the cited sites** | `Skeleton.svelte:27-37` has no `aria-busy` and names the region. The eight cited wrappers are gone. Other wrappers remain: new finding 4-06. |
| 3-05 radios without arrow keys | **Fixed** | Every cited group now has roving `tabindex` + `radioKey` (or its own handler: `NewSkillModal.svelte:119-128`). `radio-key` rule added to `ui/scripts/ui-guards.mjs`. |
| 3-06 list editors drop focus | **Fixed** | `AllowedIdsEditor.svelte:31-38`, `AgentAutonomy.svelte:45-53` (plus item-named buttons at `:236`, `:271`), `ActiveShareLinks.svelte:27-33`. |
| 3-07 canvas Delete drops focus | **Fixed** | `WorkflowCanvas.svelte:159-170` `afterDelete` focuses a neighbour; the always-mounted announcer is at `:443`; node and edge paths both call it (`:184`, `:221`). |
| 3-08 palette listbox children | **Fixed** | `Palette.svelte:508` status line outside the listbox (Retry lives there); `:547-548` hits are a `role="group"` with an `aria-hidden` header. |
| 3-09 palette plan mode | **Fixed** | `Palette.svelte:431` focuses the plan heading (`:628`, `tabindex="-1"`), steps are an `<ol>` (`:632`), Cancel refocuses the textarea (`:277-278`). |
| 3-10 connection test result | **Fixed** | `ConnectionForm.svelte:906-918` one always-mounted status chip; the detail panel (`:897`) has no live role. |
| 3-11 LiveStatus chatter | **Fixed** | `LiveStatus.svelte` has no `role=` attributes left (only the comment at `:11`). |
| 3-12 zoomed DB dashboard grid | **Fixed** | `DbDashboardBox.svelte:138` `minmax(min(380px, 100%), 1fr)`. |
| 1-03 auto-fetch contrast | **Fixed** | `GitTabs.svelte:509-527`: `.git-autofetch` no longer sets `opacity`. |

## Areas covered in this iteration

- Re-verified all 13 previously open findings (table above).
- App-wide scan of every `role="status|alert|log"` / `aria-live` element whose content holds a ticking value
  (`now()`, `rel()`, `RelTime`, elapsed counters, countdowns, poll stats), with each tick source traced:
  `lib/stores/now.svelte.ts` (1 s clock, `rel()` granularity), `ResultsGrid` elapsed effect, `App.svelte` impersonation
  countdown, `AthenaView` poll loop, `ClusterWorkspace` meta line (not chatty, as the text only changes on reload).
- Inline Edit ↔ editor swaps: a scan for `editing = true` / `startEdit` / `beginEdit` triggers in 13 files, each checked
  for focus handling (Product Overview title, description and Jira fields, `PrDetail`, `WorkItemDetail`, `NotesTab`,
  `LearningsView`, `AgentDocuments`, `EnvSelector` (fine: the trigger stays mounted), `DatabasePage` rename (fine: it
  has focus code)).
- Iteration 3's "not covered" items: the error-carrying status sites (`PrList`, product poll errors, `GroupsTab`
  (fine: a validation hint), `AutomationEditor`, `RunDetail`), the panels `FileTree`, the `NewSession` provider grid,
  Swarm `OrgTree` (fine: a full roving tree with RTL keys; reparenting has a keyboard path through the editor's
  "Reports to") and `AgentGraph`, `KanbanBoard` (fine: "Move to…" menus), Kubernetes `ClusterWorkspace` (fine:
  single-key shortcuts honour the `singleKeys` toggle and only fire with focus inside), AWS `SqsView`/`AthenaView`/
  `LogsView`/`EksView` rows (fine: keyboard rows), the Insights page.
- Rooms: `RoomParticipants` host controls traced against the server's flag resets (`crates/otto-server/src/rooms/
  actions.rs:57,80`, `presentation.rs:31-33`), `RoomPage` control buttons and status lines.
- Skeleton wrappers across the app (`-B1 <Skeleton` scan for `role="status"` / `aria-busy` parents).
- The new iteration-3 code: `workflows/inspector/*` (labels present; `IdPicker` fallback), `product/jira/IssueEditor`,
  `WorkflowCanvas` delete path, `Palette` status line.
- Custom `role="dialog"` elements outside `Modal` (`NotificationBell`, `QueryEditor` shortcuts card, workflow
  popovers (clamped, `max-height` capped, focus moves in), browser `ApprovalCard`/`PageDialogCard` (focus moves in)).
- Floating UI clamping for coordinate-positioned elements (`SiteCanvas` toolbar clamped at `:469-473`; workflow
  popovers `:575-587`; `MockupAnnotations` editor).
- Phone/tablet widths: a scan of the 601 `.svelte` files changed in the last 3 weeks for fixed widths ≥ 360 px outside
  media/container queries. Every hit is capped (`max-width`), sits in a horizontal-scroll card, or has a container
  query (`ArtifactView.svelte:1075`, `LearnedPage.svelte:592`, `GraphView.svelte:4789`).
- Contrast: a scan for `--text-dim` text with `opacity ≤ 0.6` (only decorative separators and a drag handle remain).
  Focus visibility: `outline: none` on `:focus` without a `:focus-visible` replacement (only a bordered rename input
  and programmatic-focus containers).
- Reduced motion: `lib/motion.ts`, `app.css:700,844,953`, no `.animate(` calls, Home rotation (pause control and
  pauses on hover/focus).

## Areas not covered yet (next iteration)

- The Git graph commit list keyboard model (`GraphView.svelte`): needs a design decision (one Tab stop per commit in a
  virtualised list, the row name is only the subject). Not scored.
- `Scene3DViewport`/`Studio3D` single-key shortcuts (design-tool convention) and the Composer slash popup's open state.
  Both need a decision. Not scored.
- Navigator manual session order: the keyboard path is only "Move to top" (`Navigator.svelte:1584-1586`), while the
  favourites have ⌥↑/⌥↓. Worth a decision on whether to add Move up/down.
- Generic remove-button names in about 25 list editors (`Remove row`, `Remove variable`, `Remove step`… in
  `RequestBuilder`, `EnvironmentsView`, `QueryBuilder`, `MonitorSettings`, `workflows/inspector/Loop`/`ReviewRun`) and
  focus after removing the last row in index-keyed lists. Not traced one by one.
- `vault/NoteView`, `design-hall/site/SiteCanvas` keyboard editing, the `MonitorFleet`/`MonitorCluster` tables in
  depth, `api/RequestBuilder` and `scheduled-tasks` pages.
- Touch target sizes (WCAG 2.2 2.5.8). Outside the 2.1 AA target, so not scored.

## Findings

### accessibility-responsive-4-01: major. The impersonation banner is an assertive alert that re-announces its countdown every second

- `ui/src/shell/App.svelte:930`: `<div class="provider-banner impersonation-banner" role="alert">`. Inside it, `:936-939`
  renders `{fmtImpCountdown(impSecsLeft)} remaining`.
- `:276-282`: `impSecsLeft` is `$derived.by(() => { … void now(); // reactive tick … })`. `now()` is the shared 1 s clock
  (`lib/stores/now.svelte.ts:22-28,41-43`), so the text changes every second for the whole 30-minute session
  (`IMP_DURATION_MS`, `:273`).
- `role="alert"` is `aria-live="assertive"` with an implicit `aria-atomic="true"`. Each second, VoiceOver interrupts
  whatever it is reading and speaks the whole banner again: "Acting as … (you are …) 29:41 remaining Stop
  impersonating". While an admin impersonates a user, a screen reader cannot read anything else in the app
  (WCAG 4.1.3; the same nuisance class as 3-11, but assertive and permanent).
- Fix: drop `role="alert"` from the banner (a plain region, or `role="region" aria-label="Impersonation"`). Keep the
  countdown visible but non-live. Announce only thresholds through a separate always-mounted polite region:
  "5 minutes of impersonation left", "1 minute left", "Session may have expired".

### accessibility-responsive-4-02: major. Polite status regions hold a 1-second clock, so they talk for as long as the work runs

Each of these is a `role="status"` region (implicitly `aria-atomic="true"`) whose text changes every second or every
poll, so the whole region is spoken again each time:

- `ui/src/modules/database/ResultsGrid.svelte:1752-1755` (main grid running overlay) and `:1327-1330` (mini grids):
  `<div class="rg-overlay" role="status" aria-live="polite">… Running… {elapsed}s … Cancel`. `elapsed` ticks once a
  second (`:220-235`, `setInterval(tick, 1000)`). Every query that runs longer than a second becomes "Running… 3s
  Cancel", "Running… 4s Cancel", … in the Database Explorer, the app's main data surface.
- `ui/src/modules/insights/InsightsPage.svelte:756-759`: the "Generating the report" banner is `role="status"` and holds
  `{Math.floor(pollElapsed / 60)}:{…} elapsed`. `pollElapsed` reads `now()` (`:473`). The banner's full text (a
  two-sentence explanation plus two button names) is repeated every second for the "few minutes" the run takes.
- `ui/src/lib/components/AgentDrivingBar.svelte:73-80`: `<span class="adb-now" role="status" aria-live="polite">` holds
  `Last: {last.label} · {rel(last.at)}`. `rel()` follows the 1 s tick for the first minute (`now.svelte.ts:80-93`), so
  after each agent action it is announced 55 more times ("· 6s ago", "· 7s ago", …), then once a minute.
- `ui/src/modules/settings/Channels.svelte:565-566`: the per-channel health detail `role="status"` contains
  `since <RelTime iso={h.since} />`. It is re-spoken every second for a minute and then every minute while Settings
  is open, once per unhealthy channel.
- `ui/src/modules/aws/AthenaView.svelte:569-576`: the status bar `role="status"` holds the state, scanned bytes, cost and
  `fmtMs(execMs)`. `poll()` rewrites `execMs` on every poll (`:327-329`), so the bar is re-read each poll. The region
  also contains the Copy-id button and the Results/History `Tabs` (`:578-588`), so every re-read includes those names.
- The repo's own rule for this is in `ConversationView.svelte:361-368` and the 3-11 fix: announce edges, not progress.
- Fix: take the ticking text out of the live region. ResultsGrid: make the overlay a plain region with an always-mounted
  sr-only status that says "Running query" once and "Query finished in 12 s" / "Query cancelled" at the end. Insights:
  keep `role="status"` on a short sentence only ("Generating the report") and mark the elapsed span
  `aria-hidden="true"` or move it outside the region. AgentDrivingBar and Channels: render `rel()`/`RelTime` outside
  the status element (or `aria-hidden`), so only the label change is spoken. Athena: announce only state changes
  (QUEUED → RUNNING → SUCCEEDED/FAILED) in a dedicated sr-only region and move the tabs out of the status element.

### accessibility-responsive-4-03: major. Inline Edit buttons swap themselves for an editor, and neither Edit, Save nor Cancel puts focus anywhere

The pattern is `{#if editing}<editor>…<Save><Cancel>{:else}<Edit button>{/if}` with no `focus()` on either edge
(grep for `.focus(`, `autofocus` and `focusOnMount` finds 0 hits in each file listed). Pressing Edit destroys the
focused button and the new field is not focused. Save or Cancel then destroys the focused button again. Both times
focus falls to `<body>` (WCAG 2.4.3), and a keyboard or screen-reader user has to Tab from the top of the page to find
the field they just opened:

- Product story Overview, the main story-editing surface: the title (`ui/src/modules/product/OverviewTab.svelte:503-511`
  `beginEditTitle`/`cancelEditTitle` only flip state; `:918-951` swap the `Edit title` icon button for the input),
  the description (`:552-557`, `:1235-1258`), and every Jira field editor. The `editBtn` snippet (`:866-876`)
  calls `jira.beginEdit`, which only sets `editingField` (`product/jira/issueEditorState.svelte.ts:336`), and
  `cancelEdit` only clears it (`:340-343`). Reporter, priority and estimate are each swapped this way (`:1529-1575`).
  Every one of those buttons is also named just "Edit field" (`:871`), so a screen reader can't tell them apart.
- `ui/src/modules/git/PrDetail.svelte:251-256` (`startEdit`), `:519-538`: the PR's Edit button becomes the description
  textarea, and Cancel (`:526`) and the successful save (`:272`) unmount the editor.
- `ui/src/modules/mission-control/WorkItemDetail.svelte:402-415`: `{#if !editing}<button>Edit</button>{/if}` then the
  Goal/Result/Risk fields.
- `ui/src/modules/product/NotesTab.svelte:148-192`, `ui/src/modules/product/LearningsView.svelte:400,524` (icon buttons
  named only "Edit"), `ui/src/modules/personal-agents/AgentDocuments.svelte:106-113`.
- Fix: one shared helper or action. On begin, `await tick()` and focus the first field (or add `use:focusOnMount`,
  which `MockupAnnotations.svelte:347` already uses). On Cancel or after a save, `await tick()` and focus the Edit
  button again (keep a `bind:this` or query by a `data-edit-for` key). Name the field buttons after their field:
  `aria-label="Edit {fieldName}"`.

### accessibility-responsive-4-04: minor. In New session, the provider grid's arrow and +/- keys act on the wrong card and can wipe a batch

- `ui/src/modules/agents/NewSession.svelte:472-480`: the `role="radiogroup"` grid has `onkeydown={onProviderKeydown}`,
  and each card's `count-ctl` buttons ("One less/more {p} session", `:494-516`) sit inside it. Keydown on those buttons
  bubbles to the grid.
- `:252-288`: `onProviderKeydown` uses the PRIMARY `provider`, not the focused card. On a focused "One more codex
  session" button, ←/→/↑/↓/Home/End call `selectProvider(next, true)`. That sets `counts = { [p]: 1 }` (`:244-246`),
  which throws away the batch the user was building with ±, and moves focus to another card. `+`/`-` call
  `bump(providers[idx], …)`, which bumps the primary provider, not the card whose button has focus.
- `:505-507`: each card has its own `<span class="count" aria-live="polite">{countOf(p)}</span>`. A change speaks a bare
  number ("2") with no provider, and four live regions sit in one grid.
- The radiogroup also owns non-radio buttons, so the APG radio model (arrows move between radios only) doesn't hold.
- Fix: in `onProviderKeydown`, return early unless `e.target` is a `[role="radio"]` card, and resolve the card from
  `e.target` for `+`/`-`. Move the ± controls out of the `role="radiogroup"` element (sibling per card, or a separate
  group). Replace the per-card live counts with one always-mounted sr-only status: "2 codex sessions, 1 claude
  session".

### accessibility-responsive-4-05: minor. The remaining room host controls destroy the focused button once the server echoes

3-03 fixed admit and decline only. The other controls in the same list are removed by the echo of their own action:

- `ui/src/modules/rooms/RoomParticipants.svelte:63`: "Give control…" is shown only while `member.control_requested`.
  `GrantControl` clears it (`crates/otto-server/src/rooms/actions.rs:76-80`), so the button that the confirm dialog
  returned focus to is unmounted.
- `:65-66`: "Allow presentation" (`presenter_requested`) is replaced by "Stop presenting access" (`presenter_allowed`);
  `grant_present` sets one and clears the other (`crates/otto-server/src/rooms/presentation.rs:31-33`). Pressing either
  one unmounts it.
- `:67`: "Remove…" removes the whole `<li>`.
- `ui/src/modules/rooms/RoomPage.svelte:162-163`: "Release control" (`!host && canType`) is replaced by "Request control"
  once the driver changes.
- Each time focus falls to `<body>` in the room window, mid-call (WCAG 2.4.3).
- Fix: route these through the same `refocusMember` mechanism `decide()` uses (`RoomParticipants.svelte:19-35`), with
  focus going to the member's first remaining control, or the People heading after Remove. In RoomPage, render one
  always-mounted control button whose label and action change (Release ↔ Request), as RoomChat now does for
  Send/Retry.

### accessibility-responsive-4-06: minor. Six more Skeleton wrappers nest status regions or mark them busy

The 3-04 fix made `Skeleton` an un-busy, named status region and removed the eight cited wrappers. These wrappers were
not touched:

- `ui/src/shell/SidePane.svelte:120-121`: `<div class="sp-skeleton" role="status" aria-busy="true" aria-label=…>
  <Skeleton …/>`. This is exactly the 3-04 pattern: a busy status around Skeleton's own status, so its delayed
  "Loading" text is deferred until busy clears and is never spoken.
- `ui/src/modules/agents/SplitNode.svelte:213` (`role="status" aria-label="Loading Database"` around `<Skeleton>`), and
  `ui/src/shell/App.svelte:1136` (`.rp-loading role="status"` around `<Skeleton>` in the compact shell): nested status
  regions.
- `aria-busy="true"` parents of an announcing Skeleton: `ui/src/modules/insights/InsightsPage.svelte:772` (and `:773`
  announces a second Skeleton at the same time), `ui/src/modules/skills-lab/SkillsBrowser.svelte:350`,
  `ui/src/modules/mission-control/WorkItemDetail.svelte:361`.
- Fix: drop the wrapper's `role`/`aria-busy`/`aria-label` and pass `label="…"` to Skeleton (as P07 did). Where two
  skeletons load together, give the second one `announce={false}`.

### accessibility-responsive-4-07: minor. Arrow keys inside the schema tree's Redis key filter (or on a failed row's Retry) jump to the first database

- `ui/src/modules/database/SchemaTree.svelte:647`: `onkeydown={onTreeKey}` is on the `role="tree"` element. The tree
  also renders `<RedisKeyFilter>` rows (`:657-658`, an `<input aria-label="Filter keys by prefix">` at
  `RedisKeyFilter.svelte:31-39`) and the failed-row Retry button (`:651-655`).
- `:335-352`: keydown from that input bubbles to `onTreeKey`. `row = active.closest('[role="treeitem"]')` is `null`
  (the filter row is not a treeitem), so `cur = -1`. ↑/↓ then `preventDefault()` and focus `items[0]`. Pressing ↓ in
  the key filter (for example, to reach the keys below it) throws focus to the top of the tree, and the caret never
  moves inside the input.
- The input, the Retry button and the `.node-empty`/`.node-more` text are still non-treeitem content owned by
  `role="tree"` (axe `aria-required-children`), which is the residual part of 3-02.
- Fix: in `onTreeKey`, `if ((e.target as HTMLElement).closest('input, textarea, select, button:not(.node-label)'))
  return;`. Render the key filter, the failed-row Retry and the empty/"more" notes as `role="treeitem"`s of their own
  (or as a `role="group"` under the parent node).

### accessibility-responsive-4-08: minor. The side-panel file tree conveys its folder hierarchy only by indentation

- `ui/src/modules/panels/FileTree.svelte:526-566`: rows are flat `<button class="tree-row">`s with
  `padding-inline-start: {8 + node.depth * 14}px` (`:535`) and `aria-expanded` for folders (`:536`). The list has no
  `role="tree"`/`treeitem`, no `aria-level`, no `aria-setsize`/`aria-posinset`, and no `role="group"`.
- A file three folders deep is announced exactly like a top-level file (WCAG 1.3.1). ↑/↓ (`onTreeRowKey`, `:253-269`)
  walk the flat list, but nothing tells the user where the level changes. Each row is also its own Tab stop, plus
  the "Open folder" button for every folder (`:560-563`).
- The vault `FileTree` (`aria-level` at `vault/FileTree.svelte:342`) and the fixed `SchemaTree` already show the
  pattern.
- Fix: make `.tree-list` `role="tree"` with roving `tabindex` and the rows `role="treeitem"` with `aria-level={depth +
  1}`, `aria-setsize`/`aria-posinset` and `aria-expanded`. Move "Open folder" into the row's context menu or behind
  Enter with a modifier. At minimum, add `aria-level` and an sr-only "folder, level N" to each button.

### accessibility-responsive-4-09: minor. Load-more and progress-poll failures appear in status regions inserted together with their text

The repo's own rule (`Skeleton.svelte:14-18`, the 3-10 fix) is that a live region inserted together with its text is
usually silent in VoiceOver/WebKit. These failure messages mount that way:

- `ui/src/modules/git/PrList.svelte:241`: `{#if moreError}<span class="pr-more-err" role="status">Couldn’t load more: …`.
  The focused "Load more" button only relabels itself to "Retry" (`:239`), so a failed load-more is silent.
- `ui/src/modules/product/TestCasesTab.svelte:647`, `PlanTab.svelte:525,606`, `RewriteTab.svelte:312`:
  `{#if pollFailures >= POLL_FAIL_SHOWN && lastPollError}<span class="poll-error" role="status">Couldn’t check progress:
  …`. The same applies to `AnalysisTab.svelte:539` (`<div class="poll-note" role="status">`). A stalled run never
  tells a screen-reader user that progress checks are failing.
- Fix: keep one always-mounted status element per surface, empty by default, and set its text on failure (as
  `ConnectionForm.svelte:906-918` now does). For the PR list, also put the error text in the button's
  `aria-describedby`.
