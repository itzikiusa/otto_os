# Quality iteration 3: accessibility and responsive layout

Scope: the Svelte 5 UI in `ui/src`. Excluded: `ui/src/modules/rooms/games/**`, `ui/src/modules/home/school/**`
and `ui/src/modules/home/boxes/ClassroomsBox.svelte`. This was a read-only review using
grep, rg, sed, git and python3 text scans. No builds, tests or e2e runs.

## Score

Rubric: start at 10.0, then subtract 1.0 per blocker, 0.3 per major, 0.1 per minor and 0.03 per nit.

| Severity | Open | Deduction |
|---|---|---|
| blocker | 0 | 0.0 |
| major | 3 (3-01, 3-02, 3-03) | 0.9 |
| minor | 10 (3-04 … 3-12, plus the carried 1-03) | 1.0 |
| nit | 0 | 0.0 |

**Score: 10.0 − 0.9 − 1.0 = 8.1 / 10**

Iteration 2 scored 8.5. Iteration 2 left 12 findings open, plus the leftover part of 1-03. Iteration 2
fixed 12 of them. The drop in score comes from going one level deeper. This pass covers areas that no
earlier pass reviewed: Home box controls, the database schema tree, Rooms chat and admission, the
palette's search results and plan mode, list editors, the workflow canvas Delete key, and phone
overflow inside a zoomed box. It also covers one regression from the iteration 2 skeleton fix (3-04).

## Previous findings

| ID | Status | Evidence |
|---|---|---|
| 1-03: low-contrast close and auto-fetch glyphs | **Partly fixed, still open** | The Brokers `.ctab-x` (`BrokersPage.svelte:828-838`) no longer fades. `git/GitTabs.svelte:507-519`: `.git-autofetch { color: var(--text-dim); opacity: 0.55 }` is unchanged. In the paused state, `#56565b` at 55 % on `#f5f5f7` gives about 2.5:1 (WCAG 1.4.11). Counted as one open minor. |
| 2-01: WipPanel stage, unstage and discard drop focus | **Fixed** | `WipPanel.svelte:337-366` `settleFocus` refocuses the checkbox at the same index (`data-row-key`), else the last row, else the section head (`data-wp-head`). `:890` adds an always-mounted polite region. Stage and discard both call it (`:378`, `:414`). |
| 2-02: MultiRunDialog stage changes | **Fixed** | `MultiRunDialog.svelte:360-372` focuses `[data-stage-focus]` after `tick()` on each stage change. Targets are at `:407`, `:539`, `:656` and `:668`. |
| 2-03: collapsing the side panel drops focus | **Fixed** | `RightPanel.svelte:136-161` moves focus to the rail button `data-tab` of the last shown tab, but only when focus was inside the panel or on `<body>`. |
| 2-04: Notes "Saving…/Saved" chatter | **Fixed** | `RightPanel.svelte:463-476`: the footer is no longer a live region. Only the always-mounted `.notes-alert` `role="alert"` announces, and only on failure. |
| 2-05: terminal find status | **Fixed** | `Terminal.svelte:3048-3054`: one always-mounted `aria-live` + `aria-atomic` span. |
| 2-06: Assistant chat never says Otto replied | **Fixed** | `assistant/ChatView.svelte:96-112,222`: an edge-triggered announcer ("Otto is working" / "Otto replied") in an always-mounted region. |
| 2-07: GitTabs nested close, unnamed tablist, colour-only dirty state | **Fixed** | `GitTabs.svelte:228` names the tablist. `:252-257`: `button role="tab"` with ", uncommitted changes" in its name. `:277-280`: the close button is a sibling. |
| 2-08: disclosures without state | **Fixed** | `App.svelte:1267-1268`, `StructureView.svelte:800,1091`, `Navigator.svelte:1389,1422` and `WipPanel.svelte:792-793`. |
| 2-09: active session not exposed | **Fixed** | `Navigator.svelte:1574` `aria-current`. |
| 2-10: vault tree checkbox Tab stops | **Fixed** | `vault/FileTree.svelte:291` `aria-multiselectable`, `:340` `aria-selected`, `:370` checkbox `tabindex="-1"`, `:175` key toggles selection. |
| 2-11: prohibited `aria-label` on loaders, ahead/behind counts | **Fixed at the cited sites** | All 14 sites now carry `role="status"`. `RepoView.svelte:248-249` adds sr-only "N ahead/behind". The fix wraps `<Skeleton>` (itself a busy status region) in a second busy status region, which is the new finding 3-04. |
| 2-12: failed result-set tab colour-only | **Fixed** | `ResultsGrid.svelte:1385` sr-only ", failed". |

## Areas covered in this iteration

- Re-checked all 13 earlier findings that this lens left open (table above).
- Every `role="radiogroup"` / `role="radio"` group in the app, classified by keyboard model (roving tabindex, arrow keys, the shared `lib/radioKey.ts`). Also the window-level ArrowLeft/Right handler on the Home page.
- The Database Explorer `SchemaTree` tree model: roles, levels, Tab stops and the `onTreeKey` focus maths. For comparison, the vault `FileTree` (which has `aria-level`) and the panels `FileTree`.
- Rooms: `RoomChat` send flow, `RoomParticipants` admit, decline and role controls, and `RoomLobby`.
- Live regions inserted together with their text: an app-wide scan found 180 sites. Traced in depth: `Skeleton`/`LoadState`, `LiveStatus` against `ConversationView`'s announcer, `ConnectionForm` test results, the `IssueAccounts`/`GitAccounts` test results (these are fine because "Testing…" mounts first), `FindInPage` and Composer Stop (fine because `restoreComposerFocus`).
- Focus after row deletion: `AllowedIdsEditor`, `AgentAutonomy` goals and rules, `ActiveShareLinks` revoke, MCP `AllowlistsTab` (keyed by index, so only the last row drops focus), `TabBar` session close (fine, because the fallback session's terminal takes focus at `SessionView.svelte:184-189`).
- `WorkflowCanvas` keyboard model (node nudge, Delete, edge buttons). `Scene3DViewport` key handling.
- Command `Palette`: combobox, listbox contents, search error and Retry, English plan mode, Escape and Cancel. `ShortcutsOverlay`.
- Phone and tablet layouts: a scan of every fixed `width`/`min-width`/`auto-fill minmax()` of 380 px or more outside a media or container query, plus every `auto-fill`/`auto-fit` track of 300 px or more. Product `OverviewTab` and `DesignArena` (container query at 960 px), `ActiveShareLinks` phone table, `CanvasPage` and `FocusView` side panels, and Home zoom.
- Floating UI clamping: coordinate-positioned popups (`BrowserPanel` comment popover clamped at `:390`, vault graph tooltip clamped at `:1208-1211`, `NotificationBell`, `NamespacePicker`).
- Contrast: a scan for `--text-dim` text with opacity of 0.6 or less (only `.git-autofetch` remains). Reduced motion: the global block at `app.css:844`, and smooth-scroll literals (guarded by `ui-guards`). Icon-only buttons without `aria-label`/`title` (0 hits).
- Session `TabBar`, `HistoryPage` status text, GraphView commit rows, `ModelBreakdown` chart.

## Areas not covered yet (next iteration)

- The remaining sites from the 180-site scan of conditionally inserted `role="status"` regions. Only those listed above were traced. Error-carrying ones are worth checking: `PrList.svelte:241`, `TestCasesTab`/`RewriteTab`/`PlanTab` poll errors, `GroupsTab.svelte:497`, `AutomationEditor.svelte:405` and `RunDetail.svelte:148`.
- The Git graph commit list. Every commit row is a Tab stop in a virtualised list with no ↑/↓ navigation, and the row's `aria-label` is only the subject (`GraphView.svelte:3374`), which drops sha, author and date. A keyboard-model decision is needed; not scored.
- `Scene3DViewport` single-key shortcuts (`:814-860`) fire on hover, without focus (WCAG 2.1.4). The viewport has `tabindex="-1"`, so it is unreachable by Tab. This is a design-tool convention, so not scored.
- Panels `FileTree` hierarchy is shown by indentation only (`panels/FileTree.svelte:530-541`, buttons with `aria-expanded` but no level).
- The `NewSession` provider radiogroup holds non-radio count buttons, and its `aria-live` count spans speak a bare number (`NewSession.svelte:476-520`).
- Kubernetes ClusterWorkspace, AWS SqsView/AthenaView and the Insights page in depth. Swarm OrgTree/AgentGraph keyboard.
- Touch target sizes (WCAG 2.2 2.5.8). Not scored against the 2.1 AA target.

## Findings

### accessibility-responsive-3-01: major. The Home box window pickers can't be changed from the keyboard, and the arrow keys switch the whole Home view instead

- `ui/src/modules/home/boxes/K8sBox.svelte:115-117` and `ui/src/modules/home/boxes/UsageBox.svelte:107-109`: `<div role="radiogroup">` with `<button role="radio" aria-checked tabindex={w === win ? 0 : -1}>`. Only the checked radio is in the Tab order. Neither file has a `keydown` handler, and neither uses the shared `lib/radioKey.ts`. The python classification found `roving=1 radioKeydown=0 groupKeydown=0 radioKey=False` for both.
- So the unchecked windows (`-1`) can't be reached with Tab or with the arrow keys. Choosing another window is mouse-only (WCAG 2.1.1, Level A).
- It gets worse. `ui/src/modules/home/HomePage.svelte:59-72` is a `svelte:window` keydown handler. It skips only INPUT/TEXTAREA/SELECT/contenteditable and `role="slider"`. An ArrowLeft/Right that a screen-reader user presses on the focused radio (the APG radio key) runs `home.prev()`/`home.next()`, and the whole Home space slides away.
- The correct pattern is already in the repo: `kubernetes/monitor/MonitorOverview.svelte`, `MonitorCluster.svelte`, `MonitorFleet.svelte` and `aws/MetricsPanel.svelte` use `onkeydown={radioKey}` with roving tabindex. `radioKey` calls `preventDefault()`, so the Home handler (`e.defaultPrevented` check at `:60`) stays out of the way.
- Fix: add `onkeydown={radioKey}` to each radio in both boxes (import `lib/radioKey`). Also add `t?.closest('[role="radiogroup"],[role="tablist"]')` to the HomePage skip list, so no future composite widget in a box loses its arrows to the view switcher.

### accessibility-responsive-3-02: major. The Database Explorer schema tree hides its hierarchy from screen readers, puts a Tab stop on every row, and arrow keys jump to the top after a click

- `ui/src/modules/database/SchemaTree.svelte:637-640`: the tree renders as a flat `{#each visibleFlat}` (windowed). Each row is `role="treeitem"` (`:676-681`) with `data-depth` and visual `padding-inline-start` only. The file has no `aria-level`, `aria-setsize`/`aria-posinset` or `role="group"` (grep: 0 hits). A table under a schema under a database is announced at the same level as the database (WCAG 1.3.1). For comparison, `vault/FileTree.svelte:342` sets `aria-level={n.depth + 1}`.
- `:695-702`: each treeitem contains `<button class="node-label">` with the default tab index (only the caret button at `:685` has `tabindex="-1"`). Every rendered schema node is an extra Tab stop, and the treeitem holds an interactive child (axe `nested-interactive`).
- `:330-341`: `onTreeKey` computes `cur = items.indexOf(document.activeElement)` over the `treeitem` divs. After a click, or after a Tab onto a row, focus is on the inner `.node-label` button, so `cur` is `-1`. ArrowDown and ArrowUp then go to `items[0]`, the top of the tree. ArrowLeft, ArrowRight, Enter, Space and ContextMenu do nothing (`if (cur < 0) return`).
- `:520`: the `role="tree"` element also contains the search input, the scope `<select>` and the row-count checkbox (`:521-560`). A tree may own only treeitems and groups.
- Impact: the main navigation surface of the DB Explorer. Screen-reader users lose the database → schema → table structure, and mouse-then-keyboard users who click a table and press ↓ are sent to the first database.
- Fix: put `aria-level={depth + 1}`, `aria-setsize` and `aria-posinset` (from the parent's child list) on each treeitem. Give `.node-label` `tabindex="-1"`, or drop the inner button and let the treeitem's Enter call `onClick`. In `onTreeKey`, resolve `cur` from `(document.activeElement as HTMLElement).closest('[role="treeitem"]')`. Move the search, scope and counts controls outside the `role="tree"` element, and keep the tree as one roving-tabindex widget.

### accessibility-responsive-3-03: major. Sending a Room chat message, or admitting or declining a participant, destroys the focused button

- `ui/src/modules/rooms/RoomChat.svelte:52-53`: `{#if pending}<button>Retry message</button>{:else}<button>Send</button>{/if}`. `submit()` (`:10-14`) sets `pending`, which unmounts the Send button that was just activated. When the echo arrives (`:15`), `pending = null` unmounts "Retry message" and remounts a disabled Send. The file has no `focus()`/`tick()` restore.
- The textarea (`:50`) has no Enter or ⌘Enter handler, so a keyboard user must Tab to Send for every message. After each one, focus falls to `<body>` (WCAG 2.4.3). Typing the next message means navigating back into the room panel.
- `ui/src/modules/rooms/RoomParticipants.svelte:35-39`: in `{#if member.admission === 'pending'}`, "Admit view only", "Admit — may be granted control" and "Decline" are replaced by the `{:else}` controls once the server echoes the admission. The focused button is destroyed. This is the host request flow that 1-02 made discoverable.
- Fix: in RoomChat, keep one always-mounted submit button and change its label and `type` (Send ↔ Retry message), then return focus to the textarea after a send (`textarea.focus()` once `delivered`). Add ⌘/Ctrl+Enter to submit, as Composer does. In RoomParticipants, after `admit`/`reject`, focus the member's new first control or the People heading (`tabindex="-1"`).

### accessibility-responsive-3-04: minor. Skeleton loading text is announced inside an `aria-busy` live region, and the iteration 2 fix nests a second busy status around it

- `ui/src/lib/components/Skeleton.svelte:28-29`: `<div class="skeleton-list" aria-busy="true" role={announce ? 'status' : undefined}>`. The "Loading …" text is inserted after 150 ms into the same element that declares itself busy. Under ARIA 1.2 `aria-busy`, assistive technology defers a busy live region's changes until busy is false. The skeleton is unmounted while still busy, so the comment's goal at `:14-18` (a delayed text so VoiceOver announces it) is defeated.
- The iteration 2 fix (2-11) then wrapped default-`announce` Skeletons in another `role="status" aria-busy="true"` element. That gives two nested busy status regions:
  - `assistant/AssistantPage.svelte:196,216`, `assistant/MemoryTab.svelte:176`, `assistant/TasksTab.svelte:91`
  - `personal-agents/AgentPage.svelte:438,701`, `swarm/SwarmPage.svelte:631`, `product/OverviewTab.svelte:1341`
  - `mission-control/MissionControlPage.svelte:379,410` (status without busy, still nested)
- `lib/components/LoadState.svelte:89-93` does it correctly: the status region is not busy, it holds the sr-only text, and it renders `<Skeleton announce={false}>`.
- Fix: in Skeleton, drop `aria-busy` from the status element (put it on the row container, or leave it off). At the eight wrapper sites, either remove the wrapper's `role`/`aria-busy`/`aria-label` and pass `label="tasks"` etc. to Skeleton, or keep the wrapper and pass `announce={false}`.

### accessibility-responsive-3-05: minor. Most `role="radio"` button groups have no arrow-key model, and every option is a separate Tab stop

- 13 files declare `role="radiogroup"` with `button role="radio"` children, but have no roving tabindex and no arrow handling. The python classification found `roving=0 radioKeydown=0 groupKeydown=0`:
  - `database/MongoFilterBar.svelte:94-96`, `database/DbAssistantPanel.svelte:161`
  - `kubernetes/ClusterWizard.svelte:275`, `aws/AccountWizard.svelte:288,306`
  - `design-hall/NewDesignModal.svelte:109`, `design-hall/LinksPanel.svelte:273-276`, `design-hall/studio3d/StatesBar.svelte:124`
  - `settings/Appearance.svelte:311,343`, `lib/design/scene3d/Inspector.svelte:270,291`, `lib/design/scene3d/Scene3DViewport.svelte:1011`
  - `lib/components/FloatingBar.svelte:967-977`
  - the Workspace group in `agents/NewSession.svelte:436-459`
- A screen reader announces "radio button, 1 of 3", but ↑/↓/←/→ do nothing. The APG radio contract (one Tab stop, arrows move and select) is not met, and the repo already ships the helper (`lib/radioKey.ts`, used by 4 files).
- Fix: on each radio, `tabindex={checked ? 0 : -1}` plus `onkeydown={radioKey}`. A `ui-guards` rule ("`role="radio"` without `radioKey`") would stop this happening again.

### accessibility-responsive-3-06: minor. Removing a row in list editors drops focus to `<body>`, and the remove buttons don't say what they remove

- `ui/src/modules/settings/AllowedIdsEditor.svelte:53-73`: keyed `{#each ids as id (id)}`. "Remove {name}" filters `value`, which destroys the `<li>` holding the focused button. There is no focus code.
- `ui/src/modules/personal-agents/AgentAutonomy.svelte:212,247`: "Remove goal" and "Remove rule" filter keyed lists (`g.id ?? new-i`), which destroys the focused row. Every row's button has the same name: "Remove goal" ×N, "Remove rule" ×N. The rule input is named just "Rule" (`:238`). "Add goal"/"Add rule" (`:193`, `:226`) don't focus the new empty input either.
- `ui/src/modules/settings/ActiveShareLinks.svelte:41-58,120`: after the confirm, `shares = shares.filter(...)` removes the row. Focus returns from the dialog to the Revoke button and then falls to `<body>`.
- Fix: the same rule WipPanel now follows (`settleFocus`). After removal, focus the same index's remove button, else the previous one, else the list's Add control or heading. Name the buttons after their item: `aria-label="Remove goal: {g.text}"`, `aria-label="Rule {i + 1}"`. Focus the new input after Add.

### accessibility-responsive-3-07: minor. Deleting a workflow step or connection with the keyboard drops focus out of the canvas

- `ui/src/modules/workflows/WorkflowCanvas.svelte:188-196`: Delete or Backspace on a focused `.node-main` removes the node from `graph.nodes`. The keyed `{#each graph.nodes as n (n.id)}` (`:388`) destroys the focused button. `:219-228` does the same for a focused `.edge-hit` path.
- The file has no `focus()` call (grep: only CSS `:focus` rules). Focus goes to `<body>`, so a keyboard user editing a graph is thrown out of the `role="application"` region after every delete, and nothing announces the removal.
- Fix: before removing, pick a neighbour: the next node in DOM order, or for an edge its source node's button. Focus it after `tick()`. Announce "Deleted {label}" through a polite sr-only region in the canvas.

### accessibility-responsive-3-08: minor. The command palette's listbox contains headers, an empty message and an alert with a Retry button

- `ui/src/shell/Palette.svelte:503`: `<div role="listbox" id="pal-listbox">` is driven by the combobox's `aria-activedescendant` (`:495-500`). Its children include `<div class="pal-empty">No matching commands</div>` (`:523`), `<div class="pal-section-header">Searching…/Results</div>` (`:545-547`), and `<div role="alert" class="pal-section-header">Search failed … <button>Retry</button></div>` (`:550-553`).
- A listbox may own only `option`/`group` (axe `aria-required-children`, serious). The Retry button is a Tab stop inside a composite widget that is otherwise navigated by activedescendant. "No matching commands" and "Searching…" are never announced to the combobox user, because `aria-activedescendant` is `undefined` when there are no rows (`:500`).
- Fix: render the search hits as `<div role="group" aria-label="Results">` inside the listbox, with the header as the group's label. Move the empty, searching and error text (with Retry) out of the listbox into an always-mounted `role="status"` line under the input, and set its text ("No matching commands", "3 results", "Search failed").

### accessibility-responsive-3-09: minor. Palette plan mode: the proposed plan arrives silently, and Cancel drops focus

- `ui/src/shell/Palette.svelte:425-428`: the `'plan'` outcome sets `plan = out.plan` and returns. No focus moves and nothing is announced. The plan block (`:630-647`) is plain `div`s. A user who pressed ⌘↵ in the textarea gets no signal that "Proposed plan — confirm to execute" appeared below.
- `:642`: Cancel runs `plan = null`, which unmounts the block and the focused Cancel button. `dialogFocus` (`lib/dialogFocus.ts:49-80`) only wraps the next Tab, so focus sits on `<body>` inside an `aria-modal` dialog.
- Fix: when a plan arrives, `await tick()` and focus the plan title (`tabindex="-1"`, make it a heading), or announce "Plan with N steps ready" in a polite region. Make the steps an `<ol>`. On Cancel, return focus to the textarea.

### accessibility-responsive-3-10: minor. Connection test results are inserted together with their text, so the first result is often not announced

- `ui/src/modules/connections/ConnectionForm.svelte:414-415`: each Test sets `testResult = null` before the call. The outcome then mounts two new live regions at once: `{#if testResult && …}<div class="test-panel" role="status" aria-live="polite">` (`:895-905`) and the footer `{#if testResult}<span class="test-result" role="status">` (`:908-912`).
- The repo's own rule (`Skeleton.svelte:14-18`, `Terminal.svelte:3045-3047`): a live region inserted together with its text is usually silent in VoiceOver/WebKit. So the result of the main "Test connection" action is not reliably spoken, and when it is, it can be spoken twice.
- Compare `settings/IssueAccounts.svelte:277-281` and `git/GitAccounts.svelte:394-397`, where the region mounts with "Testing…" and the result replaces it in place.
- Fix: keep one always-mounted `role="status"` (sr-only, or the footer chip with empty text) that shows "Testing…" and then "Connected" / "Test failed: …". Make the detail panel a plain region with no live role.

### accessibility-responsive-3-11: minor. The agent's live status line announces every tool step, on top of ConversationView's announcer

- `ui/src/modules/agents/conversation/LiveStatus.svelte:55-61`: the working line is `role="status"` (implicitly polite) and contains `{doing}`. `doing` (`:33-36`) changes with every pending tool call ("Reading …", "Running …", "Writing the response", "Thinking"). During a long agent turn, a screen reader speaks each step and talks over the user.
- `:63-` the waiting block is also `role="status"`. It repeats "{agent} is waiting for you" plus the question text.
- `ConversationView.svelte:361-368,981` already has an edge-only announcer, documented as "only the moments that matter" (working / finished / waiting). LiveStatus (`:915-917`) undoes that. This is the same chatter class as 1-05 and 2-04.
- Fix: drop `role="status"` from the working and stale lines (keep them as plain visible text). Keep the waiting block un-live too, because ConversationView announces it. Optionally expose the current step on demand via `aria-describedby` on the composer.

### accessibility-responsive-3-12: minor. A zoomed "DB dashboard" Home box scrolls sideways on every phone

- `ui/src/modules/home/boxes/DbDashboardBox.svelte:137-139`: `.zoomed .grid { grid-template-columns: repeat(auto-fill, minmax(380px, 1fr)); }`. There is no media or container override.
- Zoom is offered on phones (`HomeBox.svelte:172`, no phone guard). The zoom stage pads 16 px (`HomePage.svelte:463-467`) and the box body 12 px (`HomeBox.svelte:331-337`), so a 390 px phone has about 334 px for a 380 px track. `.dbd { overflow-y: auto }` (`:116-120`) makes x overflow scroll too. Every widget is cut off on the right behind a horizontal scroll (AGENTS.md: no horizontal scroll on phone).
- Every other large `auto-fill` track in the app has a phone override (MonitorOverview `:430`, MonitorFleet `:1111`, GitPage `:889`, Lobby `:797`).
- Fix: `minmax(min(380px, 100%), 1fr)`.

### accessibility-responsive-1-03 (carried, still open in part): minor

- `ui/src/modules/git/GitTabs.svelte:507-519`: `.git-autofetch { color: var(--text-dim); opacity: 0.55; }` is still about 2.5:1 against `--bg` in the light theme when auto-fetch is paused (WCAG 1.4.11). The Brokers part is fixed.
- Fix: drop the opacity. The `aria-pressed` state and the `.on` accent colour already distinguish on from off.
