# Quality iteration 5: accessibility and responsive layout

Scope: the Svelte 5 UI in `ui/src`, now **including** the room games (`ui/src/modules/rooms/games/**`) and the
classroom (`ui/src/modules/home/boxes/ClassroomsBox.svelte`, `home/boxes/classrooms/*`), which iteration 4
excluded. Excluded: `target/`, `node_modules/`, `docs/reviews/`. This was a read-only review using grep, rg, sed and
git. No builds, tests or e2e runs.

## Score

Rubric: start at 10.0, then subtract 1.0 per blocker, 0.3 per major, 0.1 per minor and 0.03 per nit.

| Severity | Open | Deduction |
|---|---|---|
| blocker | 0 | 0.0 |
| major | 1 (5-01) | 0.3 |
| minor | 11 (5-02, 5-04 … 5-13) | 1.1 |
| nit | 3 (5-03, 5-14, 5-15) | 0.09 |

**Score: 10.0 − 0.3 − 1.1 − 0.09 = 8.51 / 10**

Iteration 4 scored 8.5. Six of its nine findings are fully fixed. Three are partly fixed (4-03, 4-04, 4-09); their
remaining sites are carried below as 5-01, 5-03 and 5-02. Most new findings come from the room games and the
classroom (now in scope), the new shared `ConfluencePagePicker`, the query builder and API-client list editors, and
the vault reading view.

## Previous findings

| ID | Status | Evidence |
|---|---|---|
| 4-01 impersonation banner is an assertive alert | **Fixed** | `shell/App.svelte:944` `role="region" aria-label="Impersonation"`; `:942` sr-only status speaks only `impAnnounce` (`:287-291`: 5 min, 1 min, expiry). |
| 4-02 ticking clocks in polite regions | **Fixed** | `ResultsGrid.svelte:1392` one always-mounted status with `runAnnounce` (`:243-264`, edges only), the `Running… {elapsed}s` text is outside any live region (`:1355`, `:1384`, `:1781`). `InsightsPage.svelte:740` sr-only status; elapsed is `aria-hidden` (`:765`). `AgentDrivingBar.svelte:81` and `Channels.svelte:568` put `rel()`/`RelTime` in `aria-hidden` spans. `AthenaView.svelte:573` announces state only. |
| 4-03 inline Edit swaps drop focus | **Partly fixed** | OverviewTab uses `product/inlineEdit.ts` (`focusFirstField`, `refocusEditTrigger`). PrDetail, WorkItemDetail, NotesTab, LearningsView and AgentDocuments are unchanged (FIXES.md "Not done"). Carried as **5-01**. |
| 4-04 New session provider grid keys | **Partly fixed** | `NewSession.svelte:260-265` acts only on a focused `[role="radio"]` card and resolves that card; `:545` one sr-only count status; per-card counts are `aria-hidden` (`:527`). The ± buttons are still inside `role="radiogroup"` (`:492-541`). Carried as nit **5-03**. |
| 4-05 room host controls drop focus | **Fixed** | `RoomParticipants.svelte:19-37` `decide()` + settled predicates for Give control, presentation and Remove; `RoomPage.svelte:173` one always-mounted Release/Request button. |
| 4-06 Skeleton busy/status wrappers | **Fixed at the cited sites** | `SidePane.svelte:123`, `SplitNode.svelte:222`, `App.svelte:1150`, `InsightsPage.svelte:779-780`, `SkillsBrowser.svelte:350-351`, `WorkItemDetail.svelte:361` pass `label`/`announce={false}` with no wrapper role. Two other wrappers remain: new finding **5-04**. |
| 4-07 schema tree keys from the key filter | **Fixed** | `SchemaTree.svelte:338` returns early for events from `input, textarea, select, button:not(.node-label)`; failed/empty/more rows are `role="group"` (`:658`, `:666`, `:670`). |
| 4-08 side-panel file tree hierarchy | **Fixed** | `panels/FileTree.svelte:564-583` `role="tree"`, `treeitem` rows with `aria-level`/`aria-setsize`/`aria-posinset`/`aria-selected`/`aria-expanded`, roving `tabindex` (`:396-398`), "Open folder" is `tabindex="-1"` and in the Shift+F10 row menu (`:401-407`). |
| 4-09 failure text inserted with its status region | **Partly fixed** | Always-mounted sr-only statuses in `TestCasesTab.svelte:612`, `RewriteTab.svelte:273`, `AnalysisTab.svelte:420`, `PrList.svelte:246`. `PlanTab.svelte:525,606` unchanged. Carried as **5-02**. |

## Areas covered in this iteration

- Re-verified all nine iteration-4 findings against the current code (table above).
- **Room games (new scope):** `GamesHub.svelte`, `GameMatch.svelte`, `GameGuest.svelte`, `kart/KartGame.svelte`,
  `shooter/ShooterGame.svelte` and the HUD components. Checked: control names, `aria-pressed` pickers, every
  `role="status"`/`aria-live`/`role="alert"` and what drives its text (HUD refresh is every 80-90 ms:
  `ShooterGame.svelte:411`, `KartGame.svelte:309`), focus on start/exit/pause, overlay stacking (`z-index: 6` covers
  in the `isolation: isolate` play area), overlay sizing on phone heights (`min-height: 420px`, `overflow: hidden`),
  touch controls (`pointer: coarse` only), reduced motion (`KartGame.svelte:512`; the shooter has no CSS
  animations).
- **Classroom (new scope):** `ClassroomsBox.svelte` (stage `role="application"` with a key surface and a visible
  `:focus-visible` ring at `:714`, reduced-motion pass to the scene at `:87-89,189`, wheel zoom only when focused),
  `SchoolList` (the accessible companion with real buttons, paging, `:focus-within` reveal), `NeedsYouQueue`,
  `KidCard` (sized with `min()`/`max-height` and `overflow-y: auto`), `HoverTag` placement (`placeOverlay`, clamped by
  `place()` plus a ResizeObserver re-clamp).
- New or recently changed shared components: `ConfluencePagePicker` (combobox), `McpProfilePicker` (toggle chips,
  focus ring), `SessionMcpProfiles` (uses `Modal`), `mcp/ProfileEditor`/`ProfilesTab`.
- Iteration 4's "not covered" list: list-editor remove buttons and focus after removal (`QueryBuilder`,
  `RequestBuilder`, `EnvironmentsView`, `EnvSelector`, `AllowlistsTab`, `workflows/inspector/Loop`), `vault/NoteView`,
  `design-hall/site/SiteCanvas` (keyboard path exists through the Layers list in `LeftPanel.svelte:194`; the canvas
  toolbar is named), and `scheduled-tasks` (field errors use `role="alert"`; the next-fires preview reflects typing).
- Custom `role="dialog"` popovers: `GraphView` ref popover (clamped at `:2560-2567`, `max-height` capped, uses
  `dialogFocus`), `BrowserPanel` comment popover (clamped at `:424-431`; Esc/Cancel refocus), MissionControl detail
  sheet (`pushModal` + trap on phone/tablet).
- Coordinate-positioned elements in the 623 `.svelte` files changed in the last three weeks (`ContextMenu`,
  `NamespacePicker`, `NotificationBell`, `GraphView`, `BrowserPanel`, `MockupAnnotations` are clamped; the rest are
  canvas nodes inside scroll containers).
- Remaining `aria-busy="true"` sites app-wide (10 files) checked for an announcing Skeleton inside.
- `RoomRecap` consent and capture controls, `DiffViewer` long-line expander, `JiraIssuePicker`, `CollectionsTree`
  match count.

## Areas not covered yet (next iteration)

- The Git graph commit list keyboard model (`GraphView.svelte`) still needs a design decision; not scored.
- `Scene3DViewport`/`Studio3D` single-key shortcuts and the Composer slash popup's open state; not scored.
- Navigator manual session order (only "Move to top" from the keyboard); needs a decision.
- `panels/FileTree` APG gaps: ←/→ do not move to the parent or first child, there is no Home/End, and the roving tab
  stop can be a virtualised-out row after a mouse scroll. These are deeper tree-model work and were not traced.
- Whether keyed `Move up`/`Move down` buttons (QueryBuilder, workflows inspector) keep focus after the DOM move in
  WebKit. That needs a runtime check, not a read.
- The `MonitorFleet`/`MonitorCluster` tables, `api/AutomationEditor`, `design-hall/site/Inspector`.
- Touch target sizes (WCAG 2.2 2.5.8) and focus-not-obscured (2.4.11) as a sweep. Both are outside the 2.1 AA target.
  5-06 cites one obscured-focus case only because the overlay also fails to take focus.

## Findings

### accessibility-responsive-5-01: major (carried from 4-03). Five more inline Edit swaps drop focus on open, Cancel and save

OverviewTab now follows the `inlineEdit.ts` contract. These still use `{#if editing}<editor>{:else}<Edit>{/if}` with
no `.focus()` anywhere in the file (an rg for `focus(`/`focusOnMount`/`inlineEdit` finds nothing):

- `ui/src/modules/git/PrDetail.svelte:251-256` `startEdit()` only sets `editMode = true`. `:520-538` swap the Edit
  button for the description textarea. Cancel (`:524`, `() => (editMode = false)`) and a successful save (`:272`)
  unmount the focused button. Focus falls to `<body>` three times in one edit.
- `ui/src/modules/mission-control/WorkItemDetail.svelte:84-91` `beginEdit`/`cancelEdits` only flip `editing`;
  `:402` `{#if !editing}<button …>Edit</button>{/if}`.
- `ui/src/modules/product/NotesTab.svelte:52-57` and `:148-192`: the per-note Edit/Delete row is
  `{#if editingId !== n.id}`, so Edit, Save and Cancel each unmount themselves.
- `ui/src/modules/product/LearningsView.svelte:400,524`: icon buttons named only `aria-label="Edit"`, with the same swap.
- `ui/src/modules/personal-agents/AgentDocuments.svelte:106,112`: `Cancel` sets `editing = false` and `Edit {label}`
  sets `editing = true`, with no focus move.
- Impact: WCAG 2.4.3. A keyboard or screen-reader user has to Tab from the top of the page to reach the field they
  just opened, and again to find their place after saving. On the PR page that means going back through the whole
  PR header.
- Fix: reuse `modules/product/inlineEdit.ts` (move it to `lib/` if two modules need it). Put
  `use:focusFirstField` on each editor container and `data-edit-for={key}` on each trigger, and call
  `refocusEditTrigger(key)` after Cancel or a successful save. Name LearningsView's buttons
  `aria-label="Edit {l.title}"`.

### accessibility-responsive-5-02: minor (carried from 4-09). PlanTab's poll error is still a status region inserted with its text

- `ui/src/modules/product/PlanTab.svelte:524-527` and `:604-607`:
  `{#if pollFailures >= POLL_FAIL_SHOWN && lastPollError}<span class="poll-error" role="status">Couldn’t check progress: …`.
  The other four product tabs now use an always-mounted sr-only status. In this one, a stalled plan run stays silent
  in VoiceOver/WebKit, by the repo's own rule (`Skeleton.svelte:14-18`).
- Fix: copy `RewriteTab.svelte:273`. Add one always-mounted `<span class="sr-only" role="status">` whose text is set
  only when the failure threshold is crossed, and drop `role` from the visible spans.

### accessibility-responsive-5-03: nit (carried from 4-04). The provider radiogroup still owns the ± buttons

- `ui/src/modules/agents/NewSession.svelte:490-541`: the `role="radiogroup"` element contains, per card, a
  `role="radio"` button and a `.count-ctl` with two plain buttons ("One less/more {p} session"). The keyboard
  behaviour is fixed (`:257-265`). The structure still breaks the radiogroup's "owns radios only" model, so VoiceOver
  reports "radio group, 12 items" and lists the steppers among the choices.
- Fix: give each card a `role="none"` wrapper and put the steppers in a sibling `role="group"
  aria-label="{p} sessions"` outside the radiogroup element (CSS grid can keep the visual layout).

### accessibility-responsive-5-04: minor. Two more busy wrappers around an announcing Skeleton (4-06 class)

- `ui/src/modules/git/graph-commit-diff.svelte:546-548`: every pending file in a commit diff renders
  `<div class="df-pending" aria-busy="true"><Skeleton rows={2} height={16} /></div>`. The Skeleton has no `label`
  and announces by default (`Skeleton.svelte:13,35-38`). A commit with N pending files mounts N unnamed "Loading"
  status regions, each inside a busy ancestor that defers it. The result is either silence or N identical
  "Loading" announcements, depending on the screen reader.
- `ui/src/shell/NotificationBell.svelte:368-371`: `<div class="panel-empty" aria-busy="true">` around
  `<Skeleton … label="notifications" />`. This is the exact 3-04/4-06 pattern: the delayed "Loading notifications"
  is deferred until busy clears and the Skeleton unmounts first.
- Fix: drop `aria-busy` from both wrappers. In the commit diff, pass `announce={false}` to the per-file Skeletons and
  announce once for the diff ("Loading N files' changes") from one always-mounted status.

### accessibility-responsive-5-05: minor. Game HUD live regions tick every second, and the result screens mount silently

- `ui/src/modules/rooms/games/shooter/ShooterGame.svelte:542`: `<p class="br-objective-note" role="status">` holds
  `` `Round ${obj.round + 1} in ${Math.ceil(obj.timer)}s — …` `` or `` `Helicopter inbound ${mm(obj.timer)} …` `` /
  `` `Board the helicopter! ${mm(obj.timer)}` ``. `hud` is rebuilt every 80 ms (`:411-433`), so the whole sentence is
  spoken again every second through every Zombies pre-round and helicopter phase (the 4-02 class).
- `ShooterGame.svelte:603-610`: `<div class="br-respawn" role="status">` holds
  `` `Respawning in ${Math.ceil(me.respawn)}…` `` and also contains the four class buttons and the class blurb. Each
  second of a TDM/Domination respawn re-reads the countdown plus every button name (as Athena's status bar did before
  4-02).
- `ShooterGame.svelte:619` and `kart/KartGame.svelte:409`: the results overlays (`aria-live="polite"`) and
  `KartGame.svelte:403` `.kart-notice` (`{#if notice}<div aria-live="polite">`) are inserted together with their text,
  so "You win!" / "Victory Royale!" and item notices are not announced (`Skeleton.svelte:14-18` rule).
- `ui/src/modules/rooms/games/GameMatch.svelte:166`: the lobby's connection line ("Connection lost. Reconnecting…")
  and the members' Ready/Not ready states (`:183`) are plain text, so a status change reaches no one (WCAG 4.1.3).
- Fix: keep countdown digits out of live regions (an `aria-hidden` span), and announce edges through one
  always-mounted sr-only status per game: "Round 3 starts in 10 seconds", "Helicopter inbound", "Respawning". Make the
  respawn status a short sentence and move the class picker out of it. Add an always-mounted status that receives
  the result headline when `phase` becomes `done`. Give the lobby a polite status for connection and readiness
  changes.

### accessibility-responsive-5-06: minor. In the games, Pause doesn't move focus into its overlay, and leaving a game drops focus

- `ui/src/modules/rooms/games/kart/KartGame.svelte:385` (and `shooter/ShooterGame.svelte:529`): `Menu` toggles
  `menu`. The Paused overlay (`KartGame.svelte:419-421`, `ShooterGame.svelte:637-640`) is `position: absolute; inset:
  0; z-index: 6` with a 90 % opaque surface (`KartGame.svelte:437`, `ShooterGame.svelte:657`). `.kart-top`/the tools
  have no z-index, so the overlay covers the focused `Menu` button. Focus stays on a hidden control, and the overlay's
  Resume / Leave buttons are not focused. Neither game binds Escape to the menu (rg finds no `Escape` under
  `rooms/games/`).
- `ui/src/modules/rooms/games/GamesHub.svelte:52,64`: `exit()` clears the setup. The header's `{#if playing}Back to
  games` button, the overlay's "Back to games"/"Leave race" and GameMatch's "Leave" all unmount with the game, and
  neither `GamesHub` nor `GameMatch` calls `focus()` (rg count 0). Focus falls to `<body>` on the chooser.
- Fix: when `menu` opens, `await tick()` and focus the overlay's Resume button. Bind Escape on the canvas to open the
  menu (the shooter must first let Esc release pointer lock, then open the menu on the next Esc). Close the menu back
  onto the canvas (already done for Resume). After `exit()`, `await tick()` and focus the selected game's title
  button (`.games-titles button[aria-pressed="true"]`).

### accessibility-responsive-5-07: minor. Game result and pause overlays clip their action buttons on short screens

- `KartGame.svelte:431` `.kart-play { min-height: 420px; overflow: hidden }` and `:437` `.kart-overlay { position:
  absolute; inset: 0; justify-content: center }` have no `overflow-y`. The shooter matches (`ShooterGame.svelte:651,
  657`).
- Kart results content: 48 px padding + kicker (~19) + h2 (~27) + table header (~26) + 8 rows × (34 px portrait + 8
  padding + 1 border) = 344 + actions (~30) + 4 × 12 px gaps, about 540 px (`kartSeats` total = 8,
  `games/setup.ts:8`). The shooter table shows up to 8 rows plus the player's own or team rows (`:631`). On a phone
  in landscape, or in fullscreen on a phone, the play area is its 420 px minimum or the viewport height. The centred
  content overflows both edges inside `overflow: hidden`: the kicker and title are cut at the top, and **Race again /
  Back to games** at the bottom can't be reached by pointer. On the guest page (`GameGuest.svelte`, no PageHeader),
  those overlay buttons are the only in-game exit.
- Fix: `.kart-overlay, .br-cover { overflow-y: auto; justify-content: safe center; }`. At ≤ 640 px, also make the
  result table compact (smaller portraits) or cap it with its own scroll so the actions stay visible.

### accessibility-responsive-5-08: minor. Query builder and API-client list editors lose focus when a row is removed

- `ui/src/modules/database/QueryBuilder.svelte:1114`, `:1212`, `:1371`: the SELECT, ORDER BY and WHERE/HAVING lists
  are keyed by item id. "Remove column" (`:1136`), "Remove sort" (`:1236`), "Remove condition" (`:1412`) and "Remove
  group" (`:1375`) remove their own row, so the focused button is destroyed on **every** removal. The file has no
  `.focus(` call. Focus falls to `<body>` in the middle of the builder (WCAG 2.4.3, the 3-06 class).
- `ui/src/modules/api/RequestBuilder.svelte:1180,1193` (headers/query, keyed by index), `:952` (form fields),
  `api/EnvironmentsView.svelte:294,317`, `api/EnvSelector.svelte:164,177`, `mcp/AllowlistsTab.svelte:112,124`: with
  index keys, removing the **last** row destroys the focused button. None of these files calls `.focus(`.
- The names are generic and repeat on every row ("Remove row", "Remove variable", "Remove field", "Remove entry"),
  so a screen-reader user can't tell which header or variable a button removes without reading back through the row.
- Fix: after removal, `await tick()` and focus the same control on the row now at that index, or the previous row's,
  or the "Add …" button when the list is empty (as `AllowedIdsEditor.svelte:31-38` does). Name each button after its
  row: `aria-label="Remove header {row.key || i + 1}"`, `"Remove condition on {column}"`.

### accessibility-responsive-5-09: minor. The new ConfluencePagePicker drops focus on pick and on Clear, and does not scroll its active option into view

- `ui/src/lib/components/ConfluencePagePicker.svelte:93-133`: `{#if value}<picked + Clear>{:else}<input
  role="combobox">{/if}`. Pressing Enter on an option calls `pick()` → `onchange(id, title)` → `value` is set and the
  focused combobox input unmounts. Pressing **Clear** calls `onchange(null, null)` and unmounts the focused Clear
  button. The file has no `focus` call, so in both cases focus falls to `<body>`. This component is new and shared
  (product publish, test cases, workflow publish). It sits inside Modals, so focus leaves the dialog's flow.
- `:83-84`: ↑/↓ move `active` through `hits`, but nothing scrolls the active `<li>` into the
  `max-height: min(240px, 40vh)` list (`:144-147`). After about eight results, the highlighted option is out of sight
  for sighted keyboard users.
- Fix: after `pick`, `await tick()` and focus the Clear button. After Clear, `await tick()` and focus the input (give
  both a `bind:this`). In `onkey`, after changing `active`, call
  `document.getElementById(`${listId}-${active}`)?.scrollIntoView({ block: 'nearest' })`.

### accessibility-responsive-5-10: minor. Vault reading view: #tags and note embeds are mouse-only

- `ui/src/lib/render/markdown.ts:193` renders vault tags as `<span class="tag" data-tag=…>`, with no `tabindex` and
  no role. `ui/src/modules/vault/NoteView.svelte:180-185`: a click on `span.tag` runs `vault.searchTag(tag)`, styled
  as a clickable pill (`:668-675`, `cursor: pointer`). `onReadKeydown` (`:209-217`) handles Enter/Space for `span.tag`,
  but a non-focusable span can never be the keydown target, so that branch is unreachable.
- `markdown.ts:141` `div.note-embed`: clicking an embed opens its source note (`NoteView.svelte:186-193` via
  `data-embed-path`). The div is not focusable either, so keyboard users can't open an embedded note's source
  (WCAG 2.1.1).
- Fix: render tags as `<button type="button" class="tag" data-tag=…>#tag</button>` (or an `<a href="#">` like
  internal links) and give the embed title a real link to the source note. The existing delegated click and keydown
  handlers then work as written.

### accessibility-responsive-5-11: minor. Adding a browser comment closes the popover without restoring focus

- `ui/src/modules/panels/BrowserPanel.svelte:434-438`: `closePopover()` (Esc and Cancel) hands focus back to the
  take-over toggle. But the success path, `addAnnotation()` (`:455-463`, also reached through ⌘↵ in `popoverKeydown`,
  `:465-467`), sets `popover = { …, open: false }` directly. The focused "Add comment" button or textarea unmounts
  and focus falls to `<body>` after every comment.
- Fix: end `addAnnotation()` with `void tick().then(() => takeoverBtnEl?.focus())`, or call a shared
  `closePopover()` after recording the annotation.

### accessibility-responsive-5-12: minor. Room recap consent and capture buttons replace themselves on the server echo

The 4-05 class, in the recap banner of the same room window:

- `ui/src/modules/rooms/RoomRecap.svelte:59-60`: `{#if consented}<span>You consented</span><button>Withdraw
  consent</button>{:else}<button>I consent to capture</button><button>Do not consent</button>{/if}`. `consented` is
  derived from the server snapshot (`:22`), so pressing either button unmounts it once the echo arrives.
- `:62-63`: "Pause capture" ↔ "Start capture" are in separate `{#if room.recap.state === 'capturing'}` branches, so
  the host's focused button is destroyed when the state flips.
- `:54`: `{#if … 'finalizing'}<p role="status">Finishing …` is a status inserted with its text (silent).
- The file has no `focus` call. Focus falls to `<body>` mid-call (WCAG 2.4.3).
- Fix: render one always-mounted consent button whose label and action change (I consent ↔ Withdraw consent), plus
  one capture toggle (Start ↔ Pause), as `RoomPage.svelte:173` now does for control. Keep an always-mounted sr-only
  status for "Recap finishing" and "Capture paused".

### accessibility-responsive-5-13: minor. Closing the classroom's kid card drops focus

- `ui/src/modules/home/boxes/ClassroomsBox.svelte:662-664` mounts `KidCard` only while
  `selKid && view.kind !== 'corridor'`. `KidCard.svelte:32` "Close details" calls `onclose` → `selected = null`, which
  unmounts the card and its focused button. Detention, Kick out… and Release from detention (`KidCard.svelte:38-49`)
  also end with the kid no longer selected or present. Neither file moves focus (the only `focus` in ClassroomsBox is
  the pointer-down stage focus at `:363`). Focus falls to `<body>` and leaves the 3D stage's key surface.
- Fix: in `onclose` (and after detention or kick), `await tick()` and focus `stage` (it is already a focusable
  `role="application"` with a visible ring), or the kid's row in `SchoolList` when the stage is hidden.

### accessibility-responsive-5-14: nit. The diff's "expand line" button removes itself

- `ui/src/modules/git/DiffViewer.svelte:1362-1369`: `{#if content.length > LINE_CUT && !expandedLines.has(key)} …
  <button class="line-cut-btn" onclick={() => expandedLines.add(key)}>… expand line</button>{:else}…{/if}`. Activating
  it unmounts the focused button, and focus falls to `<body>` in a long diff.
- Fix: after `expandedLines.add(key)`, `await tick()` and focus the row (`[data-rk]`) or the nearest focusable
  control on that line.

### accessibility-responsive-5-15: nit. Search-as-you-type outcomes in the Jira picker are not announced

- `ui/src/modules/agents/JiraIssuePicker.svelte:239-259`: results replace a Skeleton, and the "No issues found." text is a
  plain `<div>`. A screen-reader user who types a query hears "Loading" (from Skeleton) and then nothing, with no
  result count and no "no results". `api/CollectionsTree.svelte:356` announces its match count, but through
  `{#if tokens.length > 0}<span role="status">`, inserted together with its text, so the first count is silent.
- Fix: one always-mounted polite status per picker: "{n} issues" / "No issues found". In CollectionsTree, keep the
  status mounted and empty when there is no filter.
