# Quality iteration 1 — lens: accessibility-responsive

Reviewer scope: Svelte 5 UI (`ui/src`), excluding `ui/src/modules/rooms/games/**`.
Read-only review (grep/sed/git only; no builds/tests run).

## Score

Rubric: start 10.0; blocker -1.0, major -0.3, minor -0.1, nit -0.03.

| Severity | Count | Deduction |
|---|---|---|
| blocker | 0 | 0.0 |
| major | 2 | 0.6 |
| minor | 7 | 0.7 |
| nit | 0 | 0.0 |

**Score: 10.0 - 0.6 - 0.7 = 8.7 / 10**

## Previous findings

None. This is iteration 1 for this lens.

## Areas covered

- Shared primitives: `Modal` (focus trap/return), `Tabs`, `DockedDrawer`, `shell/Drawer`, `ContextMenu` + `contextmenu.svelte.ts` (anchoring, keyboard open), `Toasts`, `FloatingBar`, `NotificationBell`, `Palette`
- Static guard surface (`ui/scripts/ui-guards.mjs`), `svelte-check --fail-on-warnings`, e2e axe helper (`expectAccessible`) and the `PAGES` sweep list. This was done so I would not report what the guards already enforce.
- Global CSS: `:focus-visible`, the forced-colors and reduced-motion blocks, colour tokens in all six theme/scheme blocks, plus contrast arithmetic for dimmed glyphs
- Floating UI clamping: every coordinate-positioned popup found by grep (BrowserPanel, vault GraphView tooltip, NamespacePicker, NotificationBell, GraphView ref popover, QueryEditor kbd popover, GraphSearchBar, Workflows popovers). All of them clamp and cap their height.
- Fixed widths versus phone and tablet: Toasts, Palette, Onboarding, Login, FirstRunCoach, CanvasPage, FocusView, DesignArena, GraphView detail, Terminal find, SchemaTab
- RTL: translateX centring and icon mirroring
- New and recent modules: Rooms (non-games: RoomPage, RoomChat, RoomAnnotations, RoomMedia, RoomScreens, RoomParticipants, RoomLobby, RoomHost), Workbench (Page, EditorTabs, FileList, QuickOpen, HistoryPanel, SendToButton), AWS LogsView/AwsDrawer, Kubernetes ResourceTable/ClusterWorkspace drawer, ClassroomsBox, usage ModelBreakdown, swarm RunGraph, git landed-marks (eb295dcc2), DB QueryEditor / DatabasePage tab strips, Brokers tab strip

## Areas NOT covered yet (next iteration)

- agents: SessionView, conversation/Composer, ConversationView. These were heavily reviewed earlier but not re-read here.
- Navigator/sidebar keyboard model, App.svelte phone shell, RightPanel/SidePane
- database: ResultsGrid / GridView keyboard grid model, MultiRunDialog (new, 1068 lines)
- kubernetes: PodHttpPanel (new), the monitor/* pages at phone width
- personal-agents (AgentAutonomy, AgentActivity, RoomsView), mcp AutoApprovePanel/Form, settings SecretsStore/AllowedIdsEditor/ActiveShareLinks
- design-hall and product design studios: canvas keyboard operability
- vault: FileTree tree keyboard model, LiveContext
- Live-region policy app-wide: regions inserted together with their text (`{#if}<p role="status">`)
- Touch-target sizes (17–22 px close glyphs). WCAG 2.2 2.5.8 is not in the project's 2.1 AA target, so this was not scored.

## Findings

### accessibility-responsive-1-01 — major — Closing an inline (non-modal) panel from its own ✕ drops keyboard focus to `<body>`

- `ui/src/lib/components/DockedDrawer.svelte:53`. In desktop (non-sheet) mode, `onclick={onclose}` unmounts the `<aside>` together with the focused ✕. The component has no focus-return logic for this mode; only the phone `Drawer` path uses `dialogFocus`. Callers: `modules/aws/AwsDrawer.svelte` (EC2 and RDS: `Ec2View.svelte:290`, `RdsView.svelte:228` set `detail = null`) and `modules/kubernetes/ResourceDrawer.svelte:270` (`ClusterWorkspace.svelte:173` `closeDrawer()` only clears selection and routes). The AwsDrawer header comment says the focus is handled by the shared DockedDrawer, but it is not handled in desktop mode. Esc pressed with focus inside the column has the same effect.
- `ui/src/modules/workbench/HistoryPanel.svelte:230` (`close` → `onclose` → `workbench.setPanel('history')` toggles the panel to `'none'`) and `ui/src/modules/workbench/WorkbenchPage.svelte:689` (Close placeholders): the side column is removed together with its focused button.
- `ui/src/modules/aws/LogsView.svelte:652`: `onclick={() => (detail = null)}` removes the whole `.detail` pane, including the focused button.
- Impact: after every close, a keyboard or screen-reader user restarts from the top of the document and loses their place in the resource list, event table or editor (WCAG 2.4.3).
- Fix: in `DockedDrawer`, record `document.activeElement` (or accept a `returnFocus` element) when `open` turns true, and in the close path `queueMicrotask(() => opener?.isConnected && opener.focus())`. The `Modal` pattern with `dialogFocusReturn()` can be reused for the non-modal column. Workbench: return focus to the matching toolbar toggle (`[data-testid="wb-history-toggle"]` / `wb-placeholders-toggle`). LogsView: return focus to the `tr.trow.sel` row.

### accessibility-responsive-1-02 — major — A room host gets no cue for admission, control or presentation requests on narrow layouts, and no announcement on any layout

- `ui/src/modules/rooms/RoomPage.svelte:40-41`: `narrowRoom = viewport.isPhone || viewport.isTablet`, and `peopleOpen = !narrowRoom`, so the People `<details>` starts collapsed.
- `RoomPage.svelte:149-151`: at ≤1024 px the `<aside>` holding People and Chat is `display:none` unless the "People & chat" view button is pressed. That button has no count, badge or unread state.
- `RoomPage.svelte:142-146`: the `room-status` bar shows only the viewer's role and who drives the terminal. It never mentions pending requests.
- `ui/src/modules/rooms/RoomParticipants.svelte:30,35`: "Waiting for admission", "Give control…" and "Allow presentation" appear only inside the participant list, with no live region. New chat messages (`RoomChat` `role="log"`) are likewise hidden while the Session view is active.
- Impact: a host on the Session view of a narrow hosted-room window, or on phone or tablet, does not see a guest knocking or asking for control. Screen-reader hosts are never told at any width. This is the access-control decision point of Rooms ("who can access what").
- Fix: render a host-only summary in the `room-status` bar (`role="status"`), for example "1 person waiting · 1 control request" with a "Review" button that switches `tab = 'conversation'` and opens `peopleOpen`. Add a count to the "People & chat" view button (`aria-label="People & chat, 2 requests"`). Announce new pending members through a polite live region that already exists in the DOM.

### accessibility-responsive-1-03 — minor — Inactive-tab close glyphs and the paused auto-fetch toggle fall below 3:1 non-text contrast in light themes

- `ui/src/modules/database/QueryEditor.svelte:1672-1685` `.qe-tab-close { color: var(--text-dim); opacity: 0.6 }` (full opacity only on hover or on the active tab)
- `ui/src/modules/brokers/BrokersPage.svelte:828-839` `.ctab-x { color: var(--text-dim); opacity: 0.6 }`
- `ui/src/modules/git/GitTabs.svelte:475-487` `.git-autofetch { color: var(--text-dim); opacity: 0.55 }` (the paused state of a toggle)
- Computed: native light `--text-dim #56565b` at 0.6 is 2.81:1 on `#fff` and 2.66:1 on `--surface-2`; warm light `#59554f` is 2.84 and 2.66; at 0.55 it is 2.43–2.55. Dark themes give 3.06–3.95.
- Impact: WCAG 1.4.11. On touch devices there is no hover, so the low-contrast state is the only state.
- Fix: drop the opacity and use `color: var(--text-dim)` (≥4.5:1 in every theme). Signal "inactive" by not showing the glyph's hover background, which is how TabBar does it. For the auto-fetch toggle, `aria-pressed` + `.on` (accent) already distinguish the state, so the 0.55 is not needed.

### accessibility-responsive-1-04 — minor — State is carried only by `aria-label` on role-less spans or by `title`, so it is prohibited or never reaches assistive technology

- `ui/src/lib/components/FloatingBar.svelte:718` and `:929`: `<span class="unread" aria-label="New reply">` sits inside buttons that have their own `aria-label` ("Open the Otto bar" / "Ask Otto"), so the unread state is never part of the accessible name.
- `ui/src/modules/database/DatabasePage.svelte:1153,1155`: connection-tab status glyphs are siblings of the tab button. The error dot has only `title={st.error}`, and the idle dot has `aria-label` on a generic span. A failed connection is visible only as colour plus hover.
- `ui/src/modules/proof/ProofPage.svelte:1026`: `<span class="tick" aria-label={c.passed ? 'Passed' : 'Missing'}>` is a generic span. Pass or miss is otherwise colour plus an icon only.
- `ui/src/modules/workbench/EditorTabs.svelte:46` and `FileList.svelte:147-148,188`: dirty and scope dots use the same pattern.
- ARIA 1.2 prohibits `aria-label` on `generic` (axe `aria-prohibited-attr`, serious). The e2e axe gate would fail on these surfaces if they were rendered during a scan.
- Fix: put the state into the owning control's name ("Open the Otto bar, new reply"; tab `aria-label`/`aria-describedby` "… — connection failed: {error}") or into visible or `.sr-only` text ("Passed"/"Missing", "Unsaved"). Alternatively give the glyph `role="img"`, as TabBar's `susp-dot` / `tab-unread` already do (`shell/TabBar.svelte:372-385`).

### accessibility-responsive-1-05 — minor — Workbench live validation and autosave keep interrupting a screen-reader user while they type

- `ui/src/modules/workbench/WorkbenchPage.svelte:172-189`: validation runs 400 ms after every edit and replaces `issue`. `:637` renders it as `role="alert"` (assertive), so each intermediate invalid JSON or YAML state is announced over the user's typing.
- `WorkbenchPage.svelte:613`: `.wb-save` is `aria-live="polite"` and cycles "Unsaved" → "Saving…" → "Saved · rev N" on every 800 ms autosave (`workbench.svelte.ts:47` `AUTOSAVE_MS = 800`).
- Fix: render the live-validation issue as plain text, or as `role="status"` only when it came from an explicit Format (`issueFromFormat`). Keep `role="alert"` for save failures (already a separate `.wb-inline-err`). Remove `aria-live` from `.wb-save`, or announce only transitions into or out of the error state.

### accessibility-responsive-1-06 — minor — Workbench ⌘P Quick Open moves an invisible selection and nests a button inside each option

- `ui/src/modules/workbench/QuickOpen.svelte:76-86`: the input has `aria-controls` but no `role="combobox"`, no `aria-expanded` and no `aria-activedescendant`. The options (`:90`) have no `id`. ArrowUp/ArrowDown change `index` (`:61-71`) and Enter opens `results[index]`, but assistive technology hears nothing.
- `:90-95`: `<li role="option">` wraps a focusable `<button>`, which is nested-interactive inside an option. Every row is also a Tab stop.
- Compare `shell/Palette.svelte:503-563` and `FloatingBar` (`role="combobox"` + `aria-activedescendant`; option children kept out of the tab order).
- Fix: `role="combobox" aria-expanded="true" aria-autocomplete="list" aria-activedescendant="wb-qo-{index}"` on the input, `id="wb-qo-{i}"` on each option, and move the click handler onto the `li` (with `onmousedown` preventDefault). Alternatively give the inner button `tabindex="-1"` and keep it out of the option.

### accessibility-responsive-1-07 — minor — Each DB query tab nests a "Close tab" button inside the `role="tab"` element, unlike every other tab strip

- `ui/src/modules/database/QueryEditor.svelte:1047-1116`: the `div role="tab"` contains `button.qe-tab-close` (`:1105-1115`). Tab content is presentational to assistive technology (axe `nested-interactive`, serious). The close button keeps the default `tabindex=0`, so every inactive tab adds a Tab stop and breaks the roving tabindex. Its name is a generic "Close tab" repeated N times.
- The repo's own convention puts the tab button and its ✕ side by side as siblings in a `role="presentation"` wrapper: `brokers/BrokersPage.svelte:456-482` (comment: "so the close control isn't nested inside an interactive role=tab"), `DatabasePage.svelte:1135-1160` and `shell/TabBar.svelte:317-393`.
- Fix: follow the BrokersPage structure (`div.qe-tab role="presentation"` > `button role="tab"` + sibling `button.qe-tab-close aria-label="Close {tabLabel(t, i)}" tabindex={active ? 0 : -1}`). Also add Delete/Backspace close on the tab, as TabBar does.

### accessibility-responsive-1-08 — minor — Listboxes wrap options in role-less `<li>` elements

- `ui/src/modules/design-hall/LinksPanel.svelte:269-283`: `<ul role="listbox">` > `<li>` > `<button role="option">`
- `ui/src/modules/panels/OutputsPanel.svelte:299-321`: `<ul role="listbox">` > `<li>` > `<button role="option">`
- The `li` keeps its implicit `listitem` role under a parent that is no longer a list. That breaks listbox → option ownership (axe `listitem` / `aria-required-children`). LinksPanel options are also all Tab stops with no arrow-key model.
- Fix: add `role="none"` to the wrapping `<li>` (or render the options directly). In LinksPanel, add roving tabindex with ↑/↓ like `OutputsPanel`'s `onArtifactKey`.

### accessibility-responsive-1-09 — minor — Workbench is not in the shared page sweeps, so axe, RTL and theme checks never run on it

- `ui/e2e/helpers.ts:8-44` `PAGES` lists every sidebar module except `workbench` (`ui/src/lib/sidebar.ts:120`). `pages.spec.ts`, `theme.spec.ts` and `rtl.spec.ts` iterate over `PAGES`, so Workbench never gets `expectAccessible`, the RTL overflow sweep or the light/dark sweep. Only `desktop-workbench-ownership.spec.ts:48` checks phone overflow, at one viewport. Findings 1-04, 1-05 and 1-06 sit on this unscanned surface.
- Fix: add `'workbench'` to `PAGES`, and add a small guard test that every `SIDEBAR_MODULES` id is in `PAGES` or on an explicit exemption list, so the next new module cannot drop out silently.
