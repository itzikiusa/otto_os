# Quality iteration 6, wave 0: accessibility and responsive layout

Scope: the Svelte 5 UI in `ui/src`, including the room games and the classroom. Excluded: `target/`,
`node_modules/`, `docs/reviews/`. This was a read-only review using rg, grep, sed and git. No builds, tests or
e2e runs.

## Score

Rubric: start at 10.0, then subtract 1.0 per blocker, 0.3 per major, 0.1 per minor and 0.03 per nit.

| Severity | Open | Deduction |
|---|---|---|
| blocker | 0 | 0.0 |
| major | 1 (6-01) | 0.3 |
| minor | 8 (6-02 … 6-09) | 0.8 |
| nit | 1 (6-10) | 0.03 |

**Score: 10.0 − 0.3 − 0.8 − 0.03 = 8.87 / 10**

Iteration 5 scored 8.51. Fourteen of its fifteen findings are fixed. One is partly fixed: 5-04's NotificationBell
site is still open and is carried in 6-02. The new findings come from three places: a class sweep of
live-region insertion (6-01), areas iteration 5 listed as not covered (FileTree APG, AutomationEditor,
design-hall site Inspector, Scene3D shortcuts), and new or recently changed components (JsonTree, MCP profiles,
the Jira IssueEditor, ConfluenceSpaceSelect, CapsBanner, ResetWarning).

## Previous findings (iteration 5)

| ID | Status | Evidence |
|---|---|---|
| 5-01 inline Edit swaps drop focus | **Fixed** | Shared `ui/src/lib/inlineEdit.ts`, used by `git/PrDetail.svelte:258,505,538`, `mission-control/WorkItemDetail.svelte:25`, `product/NotesTab.svelte:62,97,160,178`, `product/LearningsView.svelte`, `personal-agents/AgentDocuments.svelte:65,92,108,114`. |
| 5-02 PlanTab poll error inserted with its status | **Fixed** | `product/PlanTab.svelte:447` has one always-mounted sr-only status. The visible spans at `:530` and `:610` no longer carry a role. |
| 5-03 ± steppers inside the radiogroup | **Fixed** | `agents/NewSession.svelte:498` cards are `role="none"`, and `:518` steppers are `role="group"`. `:544` reparents them outside the radiogroup with `aria-owns`. |
| 5-04 busy wrappers around an announcing Skeleton | **Partly fixed** | `git/graph-commit-diff.svelte:547` passes `announce={false}`. `shell/NotificationBell.svelte:368` still wraps a labelled Skeleton in `aria-busy="true"`. Carried in **6-02**. |
| 5-05 game HUD live regions | **Fixed** | One always-mounted `srStatus` per game: `shooter/ShooterGame.svelte:512-539` and `kart/KartGame.svelte:46,77-79,381`, edges only. The objective note is `aria-hidden` (`ShooterGame.svelte:584`), and `GameMatch.svelte:139,204` has a lobby status. |
| 5-06 Pause focus and exit focus in games | **Fixed** | Resume takes focus (`KartGame.svelte:361`, `ShooterGame.svelte:471`). Escape opens the menu (`KartGame.svelte:267`, `ShooterGame.svelte:338`). `GamesHub.svelte:56` refocuses the selected title. |
| 5-07 overlays clip actions on short screens | **Fixed** | `.kart-overlay` (`KartGame.svelte:467`) and `.br-cover` (`ShooterGame.svelte:699`) use `justify-content: safe center; overflow-y: auto`. |
| 5-08 list editors lose focus on remove | **Fixed** | `QueryBuilder.svelte:505`. The API client uses `api/rowFocus.ts` (`RequestBuilder`, `EnvironmentsView`, `EnvSelector`), and `mcp/AllowlistsTab.svelte:76`. |
| 5-09 ConfluencePagePicker focus and scroll | **Fixed** | `lib/components/ConfluencePagePicker.svelte:90,96,104`. |
| 5-10 vault tags and embeds mouse-only | **Fixed** | Tags get `role=button` and `tabindex=0` after sanitizing (`lib/render/markdown.ts:329-330`). Embed titles are links. |
| 5-11 BrowserPanel comment focus | **Fixed** | `panels/BrowserPanel.svelte:437`. |
| 5-12 RoomRecap self-replacing controls | **Fixed** | `rooms/RoomRecap.svelte:51-55,60-75` has one consent button, one capture toggle, and an always-mounted status. |
| 5-13 KidCard close drops focus | **Fixed** | `home/boxes/ClassroomsBox.svelte:597-609` `refocusStage()`. |
| 5-14 diff "expand line" removes itself | **Fixed** | `git/DiffViewer.svelte:25` `refocusRow` (`diffRowFocus.ts`). |
| 5-15 Jira picker / CollectionsTree outcomes | **Fixed** | `agents/JiraIssuePicker.svelte:253`, `api/CollectionsTree.svelte:359`; both regions stay mounted. |

## Areas covered in this iteration

- Re-verified all fifteen iteration-5 findings (table above).
- Class sweeps across all of `ui/src`:
  - status and live regions inserted together with their text (`{#if …}<x role="status">`);
  - `aria-busy` sites, and hand-rolled loading placeholders;
  - `svelte-ignore a11y_*` suppressions (22 files; the DiffViewer gutter, Markdown, SessionView and PageHeader cases are deliberate and keyboard-reachable);
  - every custom `role="dialog"` popover (WorkflowsPage Node/Runs popovers have focus-in, Esc and return focus, and are clamped; the QueryEditor shortcuts popover is clamped);
  - coordinate-positioned floating UI (vault graph tooltip, vault link preview, NamespacePicker, NotificationBell, GraphView ref popover and ContextMenu all clamp);
  - large fixed widths and minimum widths (every wide table sits in an `overflow-x` wrapper; Onboarding, Login and FirstRunCoach cards cap at the viewport);
  - global reduced motion (the `app.css:844` blanket override plus `lib/motion.ts` for script motion);
  - generic elements carrying `aria-label` (40 sites; mostly harmless container labels or names inside buttons, so not scored);
  - drag-and-drop sources and their keyboard alternatives (15 files; vault FileTree, Kanban, OrgTree, HomeBox, DatabasePage via the connection form folder field, and design-hall LeftPanel all have one).
- Items from iteration 5's "not covered" list:
  - `panels/FileTree` APG gaps (6-06);
  - `api/AutomationEditor` (6-03);
  - `design-hall/site/Inspector` (6-05);
  - `Scene3DViewport` single-key shortcuts (6-08);
  - the Composer slash popup (a listbox driven by `aria-activedescendant` from the textarea, `Composer.svelte:401-439`; the active option scrolls into view; not scored, because a textarea cannot take `role="combobox"`/`aria-expanded`);
  - `MonitorFleet` tables (`role="table"` with `aria-sort`, roving row tabindex and named drill buttons: clean).
- New or recently changed components:
  - `JsonTree`, `ConfluenceSpaceSelect`, `CapsBanner`, `brokers/ResetWarning`, `mcp/ProfilesTab` and `mcp/ProfileEditor`;
  - `product/jira/IssueEditor`;
  - the `classrooms/SchoolBar`, `HoverTag` and `KidCard` components;
  - the shooter HUD (`Vitals`, `TeamPanel`);
  - `vault/LinkPreview` and `previewStore`;
  - `lib/dialogFocus.ts` and `Modal` (focus in, trap, Escape, return: clean).

## Areas not covered yet (next iteration)

- The Git graph commit list keyboard model (`GraphView.svelte`) still needs a design decision; not scored.
- Navigator manual session order: only "Move to top" works from the keyboard (`shell/Navigator.svelte:1586-1587`).
  That technically achieves any order, but needs a design decision; not scored.
- Whether keyed `Move up`/`Move down` reorders (`git/RecoveryTools.svelte:163,170-171`, `QueryBuilder`) keep focus
  across the DOM move in WebKit, and when the moved row's button becomes disabled at the list edge. This needs a
  runtime check.
- The force-directed vault graph's auto-running layout animation and its pause control (WCAG 2.2.2); not traced.
- Touch target size (2.5.8) and focus-not-obscured (2.4.11), as a sweep.
- `share/` (remote/mobile) pages, `swarm/RunInspector` and `personal-agents/AgentPage` on phone widths, beyond the
  static fixed-width sweep.

## Findings

### accessibility-responsive-6-01: major. About 35 status messages are live regions inserted together with their text, so they are never announced

The repo's own rule (`lib/components/Skeleton.svelte:14-18`, `lib/components/Toasts.svelte:18`) says that a live
region inserted together with its text is usually silent in VoiceOver/WebKit. Iterations 4 and 5 fixed single sites
of this class (4-09, 5-02, 5-05, 5-12). A sweep finds it is still the default way modules report saves, errors and
connection state. Each line below is `{#if cond}<el role="status">text</el>{/if}` (or `aria-live` the same way):

- Save confirmations:
  - `modules/settings/InsightsSettings.svelte:188,195` ("Saved")
  - `modules/settings/Users.svelte:525,660` ("Saved")
  - `modules/settings/Onboarding.svelte:262` ("Root account created.")
  - `modules/settings/GitBackup.svelte:172-173`
  - `lib/components/ResourceAccess.svelte:197`
  - `lib/components/AccountPicker.svelte:97`
  - `modules/usage/OttoUsage.svelte:160`
- Unsaved-changes notices: `modules/settings/Daemon.svelte:230`, `modules/settings/ContextSoul.svelte:265`.
- Errors and warnings:
  - `modules/brokers/TopicDetail.svelte:748` (consume error)
  - `modules/brokers/GroupsTab.svelte:497` (reset error)
  - `modules/brokers/ResetWarning.svelte:18-19` (new component: the "reset would be refused" warning)
  - `modules/api/AutomationEditor.svelte:409` (run error)
  - `modules/proof/ProofPage.svelte:1263`
  - `modules/database/MultiRunDialog.svelte:385`
  - `modules/usage/UsagePage.svelte:1017` ("Spend unavailable")
  - `modules/vault/VaultPage.svelte:572` (backlinks failure)
- Progress:
  - `modules/git/RecoveryTools.svelte:133` ("Working…")
  - `modules/api/HistoryList.svelte:198`
  - `modules/api/ApiPanel.svelte:44`
  - `modules/api/ResponseViewer.svelte:392`
  - `modules/agents/conversation/Composer.svelte:449` ("Uploading…")
  - `modules/browser/ReaderView.svelte:231` (mark-mode hint)
  - `modules/git/PrMergeModal.svelte:233,253`
  - `modules/api/ApiPage.svelte:416`
  - `modules/database/DatabasePage.svelte:1426` (label-only spinners)
- Rooms and games:
  - `modules/rooms/RoomMedia.svelte:46` ("Connecting voice and screens…")
  - `modules/rooms/RoomPage.svelte:162,166` ("Reconnecting… Terminal access is paused.")
  - `modules/rooms/RoomRecap.svelte:81,90` (speech capture paused)
  - `modules/rooms/RecapPanel.svelte:111`
  - `modules/rooms/games/GameMatch.svelte:147` ("Match ended": the lobby status at `:204` unmounts in the same render, so nothing speaks)
  - `modules/rooms/games/shooter/ShooterGame.svelte:601` (pick-up and event toasts)

Impact (WCAG 4.1.3): a screen-reader user gets no confirmation that Save worked, no warning that a consumer reset
will be refused, and no notice that the room has dropped terminal access while reconnecting.

Fix: add one shared announcer, for example `lib/announce.svelte.ts` exporting `announce(text, { assertive? })`.
It should drive one always-mounted sr-only polite region (and one assertive region) in the shell, the same way
`Toasts.svelte` keeps its region mounted. Call it at the state edge, and drop `role` from the visible lines. Where
a module needs a local region, keep it mounted and empty, as `PlanTab.svelte:447` does. Then add a `ui-guards`
ratchet for `{#if …}` followed by an element with `role="status"` or `aria-live` on the same line, so the class
stops regrowing.

### accessibility-responsive-6-02: minor (carries 5-04). Hand-rolled loading placeholders are busy status regions with no text

- `shell/NotificationBell.svelte:368-371` (carried): `<div class="panel-empty" aria-busy="true">` around
  `<Skeleton … label="notifications" />`. The Skeleton's delayed "Loading notifications" is deferred while busy, and
  the panel unmounts before busy clears.
- `modules/agents/conversation/ConversationView.svelte:868`: `<div class="skeleton" role="status" aria-busy="true"
  aria-label="Loading the conversation">` holds only empty `.sk` divs. A status region announces content changes,
  not its name. It has no text and stays busy for its whole life, so opening a session's conversation is silent.
- `modules/workbench/WorkbenchPage.svelte:572`: the same pattern ("Loading file", three empty spans).
- `modules/agents/conversation/PreviewBody.svelte:201`: the same pattern ("Loading the file").
- Fix: replace the three hand-rolled blocks with `<Skeleton label="the conversation" />` (or `LoadState`), which
  mounts the region empty and adds the text after its grace period. Drop `aria-busy` from the NotificationBell
  wrapper.

### accessibility-responsive-6-03: minor. AutomationEditor and AggregateBuilder reorder by index key, so focus stays on the slot and not the moved item. Removal drops focus

- `modules/api/AutomationEditor.svelte:314` is `{#each steps as step, i (i)}`, and `moveStep` (`:146-153`) swaps
  array entries. The focused "Move step 3 up" button stays in slot 3, which now shows the *other* step. Pressing it
  again swaps the same two steps back, so a keyboard user can't move a step more than one place. The same problem
  applies to `database/AggregateBuilder.svelte:185` (`stages … (i)`) with `move()` at `:152-158` and buttons at
  `:195-196`.
- Removal: "Remove step" (`AutomationEditor.svelte:328`), "Remove check" (`:360`, assertions keyed `(ai)` at `:337`),
  "Remove saved value" (`:382`, keyed `(ei)` at `:369`) and "Remove stage" (`AggregateBuilder.svelte:197`). Removing
  the last row destroys the focused button. Neither file calls `focus(` (the only rg hit is a CSS
  `:focus-visible`). The `api/rowFocus.ts` helper added for 5-08 in the same folder is not used here.
- "Remove check" and "Remove saved value" carry the same name on every row of every step.
- Fix: give steps, assertions, extracts and stages a stable client id and key the `{#each}` by it. After a move,
  `await tick()` and focus the moved item's same button (or its other arrow, when the edge disables the one that was
  pressed). After a removal, use `focusAfterRowRemoval`. Name the buttons after their row: "Remove check on
  {as.path || KIND_LABEL[as.kind]} in step {i + 1}".

### accessibility-responsive-6-04: minor. Two reorderable lists are drag-only

- `modules/product/TestCasesTab.svelte:840-857`: test cases reorder only through HTML5 drag events on the card
  (`onDragStart`/`onDrop` at `:107-139`, which persist through `persistOrder`). No button, menu item or key moves a
  case (rg finds `reorderTestcases` only in `persistOrder`). The handle is
  `<span class="drag-handle" title="Drag to reorder">&#8942;&#8942;</span>` (`:857`). It is not `aria-hidden`, so
  screen readers read out two "vertical ellipsis" glyphs on every card.
- `modules/git/GitTabs.svelte:238-247`: repo tabs are `draggable` and the code says "mirrors shell/TabBar" (`:107`).
  But `onRepoTabKey` (`:88-105`) has no ⌥⇧←/→, and there is no Move left/right menu item. `shell/TabBar.svelte:85-88,
  134-135` has both.
- Impact: WCAG 2.1.1 (keyboard) and 2.5.7 (dragging movements). The order of generated test cases, and of open
  repositories, can't be changed without a mouse.
- Fix: in GitTabs, copy TabBar's `moveTab` plus the ⌥⇧←/→ handler and its menu items. In TestCasesTab, add "Move
  up"/"Move down" to each case's actions (or ⌥↑/⌥↓ on the focused card) that reorder `orderedIds` and call
  `persistOrder()`. Keep focus on the moved case, and mark the handle glyph `aria-hidden="true"`.

### accessibility-responsive-6-05: minor. The design-hall site Inspector swaps its whole panel on every drill-in, back and delete without moving focus

- `modules/design-hall/site/Inspector.svelte`: the panel is `{#if block && bdef && section}` (block view) /
  section view / page view. rg finds no `focus(` or `tick` in the file. Every one of these unmounts the focused
  control and drops focus to `<body>`, far from the inspector, on the far side of the canvas:
  - the item rows (`:453`, `:463`) that drill into a block;
  - the back link (`:354`);
  - "Remove block" (`:363`, which also reselects the section);
  - "Add" item (`:449`, which selects the new item);
  - "Delete section" (`:586`).
- The link-list editor (`:313-317`) is keyed by index. Its rows have the same "Link label", "Link URL" and "Remove
  link" names on every row, and removing the last row drops focus (5-08 class).
- Fix: give each inspector view's heading `tabindex="-1"` and focus it (or the first field, with `focusFirstField`
  from `lib/inlineEdit.ts`) after the selection changes from within the inspector. On back, focus the item row of the
  block just left. Use `rowFocus.ts` for links, named "Remove link {l.label || i + 1}".

### accessibility-responsive-6-06: minor. The side-panel file tree can lose its only Tab stop, and ←/→/Home/End are incomplete

- `modules/panels/FileTree.svelte:396-398`: `tabRow` is `activeRow` while that path is in `visibleNodes`. But rows
  are windowed past 300 entries (`:407-411`, `ListWindow` with 20 rows of overscan), and only
  `visibleNodes.slice(treeRange.start, treeRange.end)` is mounted (`:566`). After the user opens a large folder,
  focuses a row and then scrolls away with the mouse, the active row unmounts. No mounted treeitem then has
  `tabindex="0"`, and the spacer divs are not focusable, so Tab skips the whole tree.
- `onTreeRowKey` (`:253-271`) only expands on → and collapses on ←. Per the APG tree pattern, ← on a closed folder or
  a file does not move to the parent, → on an open folder does not move to its first child, and Home/End do nothing.
  `lib/design/scene3d/Hierarchy.svelte:165-190` already implements the full model in this codebase.
- Fix: compute `tabRow` among the mounted rows (fall back to the first row in `treeRange`), or scroll `activeRow`
  back into range when the tree gets focus. Port Hierarchy's ←/→ parent/child and Home/End handling.

### accessibility-responsive-6-07: minor. The vault page preview opens on focus but can't be dismissed

- `modules/vault/LinkPreview.svelte:29`: a `role="tooltip"` card up to 320×360 px (`:17-23`) is shown on keyboard
  focus of every inline wikilink (`NoteView.svelte:483` `onfocusin={onReadOver}`) and on structured-note chips
  (`StructuredNote.svelte:66,104` `onfocus`). It hides only on blur, scroll or note change (`NoteView.svelte:484-485`,
  `:230-234`). rg finds no `Escape` in `NoteView`, `StructuredNote`, `LinkPreview` or `previewStore`. The card covers
  the text below the focused link, and the only way to hide it is to move focus away (WCAG 1.4.13, "dismissible").
- The trigger does not reference the tooltip (`aria-describedby`), so a screen-reader user hears nothing of it.
- Fix: while `linkPreview.current` is set, a window `keydown` Escape listener calls `linkPreview.hide()` (without
  moving focus). Give the card an id and set `aria-describedby` on the anchor while it is shown.

### accessibility-responsive-6-08: minor. 3D viewport shortcuts fire while the pointer merely hovers over the viewport

- `lib/design/scene3d/Scene3DViewport.svelte:818`: `const active = hovering || (host?.contains(document.activeElement)
  ?? false)`. The handler is a window listener (`:897`). With the mouse resting over the viewport, keys typed while
  focus is elsewhere still act on the scene, unless the target is a text field:
  - W/E/R switch gizmo mode and F frames the scene (WCAG 2.1.4, single-character shortcuts);
  - Delete/Backspace (`:841-848`) remove the selected object;
  - Escape deselects.
- Scenario: a user selects an object with Enter on a Hierarchy row (`Hierarchy.svelte:183-185`), leaves the
  mouse over the canvas, and presses Backspace. The selected object is deleted. Users of speech or switch input,
  whose pointer position is incidental, hit this most often.
- Fix: gate on focus only (`host.contains(e.target as Node)`, the stage already has `tabindex="-1"` and receives
  pointer-down focus), or bind the handler on `host` instead of `window`.

### accessibility-responsive-6-09: minor. More controls remove themselves on activation with no focus move

Each of these unmounts the focused control as its own result. None of the files moves focus, so focus falls to
`<body>` (WCAG 2.4.3, the 5-13/5-14 class):

- `modules/mcp/ProfilesTab.svelte:187-189`: `{#if !selected.is_default}<button>Make default</button>{/if}`
  disappears once the profile becomes the default.
- `modules/home/boxes/classrooms/SchoolBar.svelte:30-31`: a breadcrumb button navigates up and becomes the
  non-interactive current crumb (`<span aria-current>`).
- `lib/components/JsonTree.svelte:155`: the last "show N more" button unmounts when `hiddenCount` reaches 0. This is
  in the DB JSON result view.
- `shell/CapsBanner.svelte:31`: "Dismiss" removes the whole banner. Its name is also a generic "Dismiss".
- `lib/components/ConfluenceSpaceSelect.svelte:64-68`: "Retry spaces" is replaced by a Skeleton, then by the select.
- `modules/rooms/games/GameMatch.svelte:146-147`: when the match closes mid-game, the focused game canvas unmounts
  and the "Back to games" panel replaces it without taking focus.
- Fix: after the state change, `await tick()` and focus the natural successor:
  - ProfilesTab: the profile's ⋯ button;
  - SchoolBar: the stage (`refocusStage`) or the new last crumb;
  - JsonTree: the last newly revealed row;
  - CapsBanner: the main content landmark, and rename the button to "Dismiss permissions notice";
  - ConfluenceSpaceSelect: the select;
  - GameMatch: "Back to games".

### accessibility-responsive-6-10: nit. The Jira IssueEditor's multi-select has no group name, and the user field misses its first focus

- `modules/product/jira/IssueEditor.svelte:46-56`: the checkbox list for array fields is a plain
  `<div class="field-multiselect">`. Every other branch names its control `aria-label={ef.name}`, but here the
  options are announced without the field they belong to.
- `:18,26-27`: `use:focusFirstField` runs once on mount. For a user field whose assignees are still loading, the
  editor holds only a Skeleton at that moment, so nothing is focused, and the `<select>` that arrives later is not
  focused either.
- Fix: `<div class="field-multiselect" role="group" aria-label={ef.name}>`. Re-run `focusFirstField` when
  `assignablesLoading` turns false (an `$effect` on the container), or put `focusOnMount` on the select.
