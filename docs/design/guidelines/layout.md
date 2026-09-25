# Layout

This doc covers:

- the shell anatomy
- the sidebar
- the page chrome (`PageHeader` and `PageBody`)
- the five page archetypes, and Home as the desktop
- responsive rules
- windows
- the floating command bar

---

## 1. Shell anatomy

```
┌─● ● ●──────────┬─────────────────────────────────────────────────────────────┐
│ workspace ▾     │ PageHeader (46px, window drag region)                       │
│ Search      ⌘K  │ [icon] Title  badge  [inline tabs]   [secondary…][Primary][⋯]│
│                 ├─────────────────────────────────────────────────────────────┤
│ WORK            │                                                             │
│  ▌Home          │ PageBody                                                    │
│   Agents     2  │  (full width, or readable 1200px column, left-aligned)      │
│    • session    │                                                             │
│ AUTOMATE        │                                                             │
│   Swarm         │                                                             │
│ BUILD …         │                                                 ┌─────────┐ │
│ INFRASTRUCTURE… │                                                 │ toasts  │ │
│ INSIGHT …       │                                                 └─────────┘ │
│ ⚙ Settings  🔔  │                                                             │
├─────────────────┴─────────────────────────────────────────────────────────────┤
│ Status bar 24px: ● 2 working · live · ⎇ main · loopback                       │
└────────────────────────────────────────────────────────────────────────────────┘
```

The parts of the shell:

- **Sidebar.** `shell/Navigator.svelte` when expanded, `shell/Rail.svelte` when
  collapsed (icons only; ⌘1 toggles between them). Both use `.sidebar-material`.
- **Backdrop and glass.** The window paints the ambient backdrop; the sidebar,
  the `PageHeader` row and the status bar are glass over it; the content column
  is opaque `--bg`. See
  [foundations.md §7](./foundations.md#7-translucency-vibrancy-and-the-ambient-backdrop).
- **Page chrome.** `PageHeader` plus `PageBody` on every module page. The one
  exception is **Agents**, whose session `TabBar` is its top row; it has a
  right panel (⌘J) for Browser, Outputs and similar.
- **Overlays.** The floating bar (§7, ⌘K on desktop), the `Palette` sheet
  (phone/tablet, ⌘I), the `?` `ShortcutsOverlay`, `Modal` sheets, the
  global `ContextMenu` and `Toasts` are all mounted once in `shell/App.svelte`.
- **Notification bell.** It lives in the sidebar. It used to float over pages
  and cost every page a 42 px "bell gutter", which is now removed. Don't
  reintroduce floating page-level chrome — the one exception is the shell's
  floating command bar (§7), which docks out of the way.
- **Phone.** A mobile top bar (44 px, title from `moduleLabel()`), the content,
  and `BottomNav` (56 px). The `Drawer` holds the Navigator.

## 2. Sidebar

Every module is **one entry** in `SIDEBAR_MODULES` (`ui/src/lib/sidebar.ts`).
Rail, Navigator, BottomNav, the Settings → Appearance customiser, the phone
title and the ⌘K "Go to" commands all come from it. Don't hand-list modules
anywhere else.

```ts
// ui/src/lib/sidebar.ts
{
  id: 'scheduled-tasks',      // route id: router.go(id)
  icon: 'calendar',           // IconName; unique across modules (the Rail is icons-only)
  label: 'Scheduled Tasks',   // what users see; also the phone title
  group: 'automate',          // SidebarGroupId
  feature: 'scheduled_tasks', // RBAC gate at 'view' (omit = ungated); or featureAny: [...]
  keywords: 'cron recurring job report cadence hourly daily', // ⌘K fuzzy terms
}
```

- **Groups** (`SIDEBAR_GROUPS`, rendered in this order by default, macOS
  source-list style):

  | Group id | Label |
  |---|---|
  | `work` | Work |
  | `automate` | Automate |
  | `build` | Build |
  | `infra` | Infrastructure |
  | `insight` | Insight |
  | `plugins` | Plugins (runtime plugins only) |

  Put a new module in the group whose verb fits it. **A new group needs design
  review.** Five sections is the budget.
- **Favorites** is a user-made section, always rendered **first** (star
  icon, label "Favorites"), and only while it holds at least one module the
  user can see. A module is favorited from its row's context menu ("Add to
  Favorites"), the star toggle in "Customize sidebar", Settings → Appearance,
  or ⌘K on its page. A favorite is listed **only** in Favorites — it leaves
  its own section and returns to its saved slot there when unfavorited.
  Favorites keep RBAC: an id the user can't see is skipped, never shown.
  Stored per device in `ui.sidebarFavorites`; `sidebarSections()` builds the
  layout (Navigator, Rail, BottomNav and Settings all use it). Favorites
  reorder by drag at any time (not only while customizing) and with ⌥↑ / ⌥↓.
- **Section order** is the user's (`ui.sidebarGroupOrder`, resolved by
  `resolveGroupOrder()`: unknown ids are ignored and new sections append in
  default order). Sections move with the header's context menu ("Move section
  up/down") or, while customizing, the header's arrows and grip. Favorites
  never moves.
- **Section headers** fold (a chevron on hover, stored per device in
  `ui.sidebarCollapsedGroups`). The section that holds the active route is
  pinned open. The Rail draws a thin separator between sections instead.
- **User order and hiding** only rearrange modules *within* their section
  (`moveWithinGroup`); the one crossing is favoriting (a drop onto a favorite
  favorites the dragged module at that slot). Hidden modules stay reachable
  with ⌘K. "Reset to default" clears order, hiding, folds, favorites and
  section order.
- **Active item:**
  - an `--accent` tint at 16%
  - `--text` at weight 600
  - a 3 px `--accent` bar on the inline-start edge
  - an `--accent` icon

  A nested active row (a focused session under Agents) is quieter: an 11%
  tint and no bar. Never use a solid fill or a non-accent colour for
  selection.
- **The active row stays visible.** The Navigator scrolls it into view on
  every route change.
- **⌘K "Go to"** commands are generated from `availableModules()`, with the
  section as dim `detail` text (`Command.detail`).
  A new module gets its Go-to command for free. Don't register one by hand.
- **Labels:** as short as possible, one to three words. A module name is a
  proper noun and is written in Title Case everywhere it appears ("Scheduled
  Tasks", "Mission Control"). Acronyms stay as they are (AWS, API, MCP).
  Everything else in the UI is sentence case ([content.md](./content.md#3-capitalisation-and-punctuation)).
  Don't write a tagline in the label. Explain jargon in the page's subtitle or
  empty state instead.
- **Routes without an entry** (Database and Brokers under Connections,
  Settings, Walkthroughs) resolve their display name through `moduleLabel()`.
  Never show a raw route id like "Mission-Control".

## 3. Page chrome

`PageHeader.svelte` and `PageBody.svelte` live in `ui/src/lib/components/`, and
every top-level module page (except Agents, which keeps its TabBar) uses them.

### 3.1 `PageHeader`: the unified toolbar

The page title and the toolbar share **one fixed 46 px row**, as in a macOS
unified toolbar. The row is also the window's drag surface in Tauri:
`startWindowDrag` ignores mousedowns on controls. When the sidebar is collapsed,
the row pads itself past the traffic lights.

```
[leading] [icon] Title [badge] subtitle…  [tabs (inline)]  [secondary…] [primary] [⋯]
```

| Prop | Type | Use |
|---|---|---|
| `title` | `string` | Required. The module name, or the selected item's name on a detail page. It is also the accessible name and the tooltip. |
| `icon` | `IconName` | Optional dim icon before the title. Most pages omit it; the sidebar already shows it. |
| `subtitle` | `string` | One dim line on the title's baseline (`--fs-s`), so the title sits at the same height on every page. It yields first when space runs out (ellipsis; full text in the tooltip). Hidden on phone. |
| `badge` | snippet | A status pill or count next to the title. |
| `leading` | snippet | A back button (phone detail views). |
| `crumbs` | `{label, onclick}[]` | A breadcrumb trail for sub-pages (`Kubernetes / Monitor / Fleet`). |
| `titleContent` | snippet | Replaces the title text, for example with a switcher button. `title` stays the accessible name. |
| `actions` | snippet | Right-aligned controls. They overflow into ⋯. |
| `tabs` | snippet | A segmented control or tab row. |
| `tabsPlacement` | `'inline' \| 'below'` | `inline` (default) sits after the title; `below` is a second row. On phone the tabs always go below. |
| `class` | `string` | A root class for page-specific tweaks. |

**How actions overflow.** Controls rendered as direct children of the `actions`
snippet never wrap to a second line. When space runs out, the lowest-priority
ones fold into a "⋯" menu (the global, viewport-clamped `ctxMenu`):

- Collapse order is `data-overflow` ascending (a number, default `0`, where
  lower collapses first), then right to left.
- **These never collapse:** `.primary` buttons, anything with `data-keep`, and
  anything that contains a `select`/`input`/`textarea`.
- A collapsed control becomes a menu row labelled by `data-label`, then
  `aria-label`, then `title`, then its text. Its icon comes from `data-icon`.
  Choosing the row `.click()`s the original, so handlers and disabled state
  still come from the original.
- A wrapper element (a button group) becomes one menu row per button.

```svelte
<PageHeader title={pack ? pack.title : 'Proof Packs'}>
  {#snippet leading()}
    {#if viewport.isPhone && pack}
      <button class="icon-btn" onclick={back} aria-label="Back to list" title="Back to list">
        <Icon name="chevronLeft" size={16} />
      </button>
    {/if}
  {/snippet}
  {#snippet badge()}
    {#if pack}<ProofStatusChip status={pack.status} risk={pack.risk_score} />{/if}
  {/snippet}
  {#snippet actions()}
    {#if pack}
      <button class="btn small" data-icon="plus" onclick={addArtifact}><Icon name="plus" size={12} /> Add artifact</button>
      <!-- collapses first -->
      <button class="icon-btn" data-overflow="-2" data-icon="trash" data-label="Delete pack"
              onclick={removePack} aria-label="Delete pack" title="Delete pack"><Icon name="trash" size={14} /></button>
      <!-- never collapses -->
      <button class="btn small primary" onclick={assemble}><Icon name="refresh" size={12} /> Assemble</button>
    {/if}
  {/snippet}
</PageHeader>
```

Rules:

- **Exactly one `PageHeader` per page**, as the first child of the page root.
  Don't build a second header row inside the body. Agents is the only
  exception.
- **At most one `.primary`** in the header. While a page's list is empty, the
  primary "New …" moves into the page `EmptyState`, and the header leaves it
  out.
- **Every icon-only action** has `aria-label` and `title` (the same text) and a
  `data-icon`. Set `data-label` when the menu row needs different text.
- **Destructive actions** (Delete, Remove) get the lowest `data-overflow`
  (collapse first) and never sit next to the primary action.
- **Keep it to five controls or fewer.** More than about five controls in the
  header means the page is doing too much. Move the rest to the ⋯ menu or into
  the content.
- Put **tabs inline** for 2–4 short tabs that switch the whole page (Insights:
  Reports | Health). Put them **below** for longer sets.
- **Don't** add filters, search fields or view options to the header by
  default. They belong to the list pane they filter. A page-wide search field
  is fine, but mark it `data-keep`.

### 3.2 `PageBody`: the one content-width rule

| Prop | Default | Use |
|---|---|---|
| `width` | `'full'` | `'full'` for tool, dashboard and list pages. `'readable'` caps the column at `--page-readable` (1200 px), **left-aligned** with the header title. |
| `padded` | `true` | `false` for pages that manage their own panes (split views). |
| `fill` | `false` | Pins the content to the pane height with no page scroll. Use it for bodies whose children scroll themselves (split panes, boards, editors). |

```svelte
<PageHeader title="Daemon" subtitle={`ottod ${version} · API v${apiVersion}`} />
<PageBody width="readable">
  …
</PageBody>
```

- **Never centre a narrow island** (a 940 px column with dead space on both
  sides). Readable pages are left-aligned.
- Pages that scroll use `PageBody`'s scroller. Don't nest a second full-height
  scroller inside it.

## 4. Page archetypes

Every module page is one of these five. Pick one before you design anything.

| Archetype | It is… | Today |
|---|---|---|
| **List/detail** | a collection of objects where you open one | Swarm, Proof, Workflows, Connections, History, AWS, Brokers, Scheduled Tasks, Personal Agents, Product |
| **Workbench/editor** | a working surface: text, code, queries, a terminal | Agents, Vault, Git, API, Database, Skills Lab |
| **Dashboard** | read-mostly summaries of live state | Home, Mission Control, Usage, Insights |
| **Settings form** | configuration you edit and save | Settings sections, MCP policies, Daemon, Appearance |
| **Canvas/studio** | a spatial or visual artefact you compose | Canvas, the proposed Design Hall studios |

Which one to pick:

- Is the main thing a **set of named objects**? → **list/detail**.
- Does the user **type or manipulate content** in it for minutes at a time? →
  **workbench**.
- Does the user mostly **glance** at it to see state? → **dashboard**.
- Is it mainly **fields and toggles**? → **settings form**.
- Is the object **2D/3D space** the user arranges? → **canvas/studio**.

### 4.1 List/detail

```
PageHeader: title = selected item (or module), item actions, primary
┌ list pane 260–320px ───┬ detail ─────────────────────────────┐
│ section-title  [+]     │                                     │
│ filter chips           │  selected item                      │
│ ▌item (selected)       │                                     │
│  item                  │                                     │
└────────────────────────┴─────────────────────────────────────┘
```

- **Never open on an empty "pick one" pane when there are items.** Restore the
  last selection or fall back to the first item, with `lib/lastSelection.ts`:

  ```ts
  import { initialSelection, rememberSelection } from '../../lib/lastSelection';

  // once the list has loaded (desktop/tablet only; on phone the list IS the page)
  const id = initialSelection('proof', proof.packs, (p) => p.id);
  if (id) open(id);
  // whenever the selection changes
  $effect(() => { if (detail?.pack.id) rememberSelection('proof', detail.pack.id); });
  ```
- **Empty list:** hide the list pane entirely. A page-variant `EmptyState`
  owns the page and carries the one "New …" call to action. A list that is
  empty because of a filter keeps its pane so the filter can be changed, and
  says "No proof packs match."
- **Nothing selected** (after a delete, say): show an `EmptyState` or, better,
  a small summary of the collection (counts, recent activity). Never a bare
  "Select an item" line.
- List rows:
  - title in `--text`
  - one meta line in `--text-dim`
  - status at the trailing edge
  - selected row in `--accent-soft`, hover in `--hover`

  Raw IDs (ULIDs) don't belong in rows; put them in the detail view or a
  tooltip.
- The "New" affordance lives **once**: an icon `+` in the list-pane header, or
  the primary action in the page header. Not both, and not a third copy in the
  empty state while items exist.
- **Phone:** push navigation. The list is full height. Opening an item replaces
  it, and `PageHeader`'s `leading` slot shows a back button.

### 4.2 Workbench/editor

- `PageBody fill padded={false}`. The panes own the layout: resizable splits,
  internal scrolling, and no page scroll.
- In-pane toolbars are compact (`.btn.small`, `.icon-btn`, 14 px icons) and
  **size themselves with container queries, not viewport media queries**. A
  toolbar that hides its labels at `@media (max-width: 1500px)` loses them on
  every 14" MacBook even when the pane has room.

  ```css
  .qe-toolbar-wrap { container-type: inline-size; }
  @container (max-width: 560px) { .btn-label { display: none; } }
  ```
- Dense by design: mono for code and IDs, `--fs-s` for secondary panes.
  Keyboard-first (see [patterns.md → Keyboard](./patterns.md#9-keyboard-first)).
- Every icon-only tool has a tooltip (`title`) and an `aria-label`. A row of
  unlabelled icons is only acceptable when each has a tooltip and the set is
  conventional (bold/italic, zoom).

### 4.3 Dashboard

- A grid of cards (`.card`: `--surface`, `--border`, `--shadow-card`).
- Every card has:
  - a title row (label, plus an optional "open" affordance to its module)
  - one primary figure or list
  - its **own** loading, empty and error states, so one failing box never
    blanks the page
- KPI figures: `--fs-xl`/`--fs-2xl`, `tabular-nums`, **one** font family per
  row (don't mix mono `$0.00` with sans counts). In widgets, figures sit number
  over label, split by `--separator` hairlines, not in filled tiles.
- **Don't seed empty cards.** A default layout contains only kinds that have
  data. Offer "Add widget" for the rest.

#### Home: the desktop

Home is the one "desktop" in Otto (`modules/home/`). Other modules don't grow
widget systems of their own; contribute a Home widget kind (`kinds.ts`)
instead.

```
PageHeader: Home  [01 Overview][02][03] + ⋯           [30s]  [+ Add widget]
┌ ambient backdrop, full-bleed ─────────────────────────────────────────────┐
│ Good afternoon, Dana                                                      │
│ Thursday 24 September · 1 needs you · 2 running                           │
│ ┌ Needs you ─┐ ┌ Running ───┐ ┌ Up next ───┐ ┌ Recent ────┐               │
│ └────────────┘ └────────────┘ └────────────┘ └────────────┘               │
│ ┌ widget ──────────────┐ ┌ widget ───────────────────────────┐            │
│ └──────────────────────┘ └───────────────────────────────────┘            │
└───────────────────────────────────────────────────────────────────────────┘
```

- **The backdrop** is the ambient image, painted full-bleed and fixed to the
  viewport, so it runs on into the toolbar and sidebar glass. Everything on it
  is an opaque card at elevation 1. Text straight on the backdrop is `--text`
  only (the greeting and today's line).
- **Today** (`HomeToday.svelte`, data in `today.svelte.ts`): four glance cards,
  built from data Otto already has.
  - *Needs you*: assistant tasks waiting on you, pending MCP approvals,
    sessions waiting on input, work items awaiting approval, unread warnings.
  - *Running*: running assistant tasks, working sessions and in-flight
    workflow runs.
  - *Up next*: the assistant's reminders due in the next 24 hours (the typed
    `Reminder` slot), then today's notices.
  - *Recent*: Design Hall artifacts and pull requests from the work graph.

  Each card shows at most three rows plus "N more", and has its own loading and
  empty state. An empty card is a centred, one-line "nothing here" with a quiet
  mark, never a blank box. On a phone an empty card collapses to one line.
- **Widgets** are the existing live boxes on a 12-column grid (`COLS = 12`, row
  72 px, gap 12 px), up to 8 per space. They keep drag-to-reorder (the grip),
  resize in grid units (the corner, or arrow keys on it) and zoom to fill. Their
  header is quiet: icon tile and title; refresh, zoom and the menu surface on
  hover or focus (always visible without hover).
- **Spaces 01–04** are Home's views (`MAX_VIEWS = 4`). The toolbar shows them as
  numbered segmented tabs (the active one also shows its name), then `+` (add)
  and `⋯` (rename, add, delete). ←/→, swipe and ⌘K "Home: space 01 · …" switch
  them; they can cycle every 30 s. The active space and the names live in
  `lib/stores/spaces.svelte.ts`, shared with the floating bar's 01–04 and synced
  across windows; Home owns what a space contains.
- **An empty space** invites setup without clutter: one page `EmptyState` with
  the single "Add widget" action and a row of quick-add chips for the first few
  widget kinds. Examples are generic (agents, usage, clusters), never tied to a
  particular product or customer.

### 4.4 Settings form

- `PageBody width="readable"`.
- Group fields into sections under `.section-title`. Order them from most-used
  to rarest, and put danger-zone actions (Delete, Reset) last, visually
  separated.
- Fields use `.field`: a label above, the control, then `.hint`. Toggles and
  checkboxes put the label to the right (`.checkbox-row`).
- **Saving:**
  - A single toggle or select applies immediately and shows a quiet
    confirmation (inline "Saved", or `toasts.success` if the effect happens
    off-screen).
  - A multi-field form has an explicit **Save** as its primary action, which
    stays disabled until something changes.
  - A page doesn't mix both models.
- Long settings navigation (Settings has 26 sections) is itself a list/detail
  page. **TBD:** grouping Settings sections the same way as the sidebar
  (Personal / Workspace / Integrations / Admin).

### 4.5 Canvas/studio

The target is the Design Hall mockup's Site Studio.

```
PageHeader: crumbs / title · status pill · version   [device ▢▢▢] [Fit]   [secondary] [Publish ▾]
┌ left panel ───┬ stage (full-bleed, neutral pasteboard) ────┬ right panel ─────┐
│ Pages|Layers| │     ┌──────── artboard ────────┐            │ Design|Agent|Links│
│ Blocks        │     │ floating contextual bar  │            │ inspector / chat │
│               │     └──────────────────────────┘            │                  │
├───────────────┴─────────────────────────────────────────────┴──────────────────┤
│ version strip 40px: v10 (agent) · v11 · v12 (you) …            Compare History ▸│
└────────────────────────────────────────────────────────────────────────────────┘
```

- **The stage is full-bleed** on a neutral pasteboard (`--bg` or `--term-bg`
  with a subtle dot grid). Chrome never covers the artboard except for the
  **floating contextual toolbar** anchored to the selection. That toolbar is a
  small `--surface` pill with `--shadow`, and it is clamped into the stage.
- Side panels are 240–320 px, can be collapsed, and use segmented tabs at the
  top.
- **Versions are the undo model.** Every applied change (the user's or an
  agent's) creates a version chip, and restoring creates a *new* version.
  Nothing is lost silently.
- This is the other archetype where the ambient backdrop may show (around the
  pasteboard, never under the artboard) and where the floating command bar
  (below) may appear.

## 5. Responsive rules

Breakpoints live in `lib/stores/viewport.svelte.ts` (`PHONE_MAX = 640`,
`TABLET_MAX = 1024`) and are mirrored in `app.css` (`--bp-phone-max`,
`--bp-tablet-max`):

| Mode | Width | In code |
|---|---|---|
| phone | ≤ 640 px | `viewport.isPhone`, `@media (max-width: 640px)` |
| tablet | 641–1024 px | `viewport.isTablet`, `@media (max-width: 1024px)` |
| desktop | ≥ 1025 px | `viewport.isDesktop` |

- Use **only these two media-query breakpoints**. The tree still has about 17
  stray ones (760, 900, 1500…); don't add more. For anything whose width
  depends on a split pane (Agents splits, right panel, database panes), use a
  **container query**.
- **No horizontal page scroll** at any width. Wide content (tables, graphs,
  code) scrolls inside its own container. `expectNoHorizontalOverflow` in
  `ui/e2e/helpers.ts` asserts this.
- **Phone:**
  - one header row, then content, then bottom nav
  - list/detail becomes push navigation
  - `PageHeader` hides the subtitle and moves tabs below
  - no floating chrome over the content
  - the drawer opens only on demand

  Aim for at least 70% of the first screen to be content.
- **Tablet:** the full shell with the Rail. **TBD:** default to the collapsed
  Rail at 641–1024 px, since the expanded sidebar costs about 22% of the width.
- **Touch targets** on phone: at least 36 px hit area. Grow the padding, not
  the glyph.
- Fixed `min-width` values of 700 px or more on content force page scroll on a
  tablet. Put the element in a horizontal scroller instead.

## 6. Windows and pop-outs

- Otto is **one main window** plus real native windows. `apps/desktop/src-tauri`
  already opens secondary windows (`windows.rs`, the same vibrancy; e2e in
  `desktop-multiwindow.spec.ts`).
- When a surface needs to sit beside the main window (a terminal, a
  doc, a preview), **pop it out into a native window**. Never build draggable
  in-app windows, an MDI area, or a taskbar.
- Sheets (`Modal`) are for short, blocking tasks. Anything the user works in
  for minutes is a page, a pane or a window, not a modal.

### Side by side

Two sections of the sidebar can share the content column, like a split view:
`[sidebar] [main pane │ side pane]`. It is a split, not an inner window: no
title bar of its own, no dragging, no overlap, never more than two panes.

- **Entry points.** A sidebar row's context menu ("Open side by side", or
  "Show in side pane" while one is open), ⌥-click on a row, `⌘\` (a module
  picker; `⌘\` again closes the pane) and the ⌘K "Open <Module> side by side"
  / "Close side pane" / "Swap panes" commands. Desktop main window only: on
  phone and tablet, and in pop-outs, the entry points aren't offered and a
  pane that no longer fits is hidden, never cleared.
- **One module per pane.** A navigation to the module the other pane shows
  is delivered to that pane (a session opened from Home appears in a side
  Agents pane), so a module is never open twice.
- **Chrome.** The side pane has no bar of its own: its Swap / Open in main
  pane / Close buttons sit at the end of the page's own top row (`PageHeader`,
  or the Agents `TabBar`), so both panes' toolbars line up. The divider is a
  1px `--separator` hairline with a 9px grab area, a keyboard-operable
  `role="separator"` (←/→, ⇧ for a big step, Home/End, Enter or double-click
  for 50/50), clamped to 25–75% and a 360px floor per pane.
- **Mechanics.** The side pane is the app in a same-origin iframe
  (`?embed=1`, `lib/sidePane.ts`): its own router and stores, so any module
  works there unchanged. Anything that must run once per window (the native
  menu bridge, native notifications, notice toasts, route restore, window-key
  GC, the service worker) is skipped in the embedded document. See
  [multi-window.md → Side by side](../../features/multi-window.md#side-by-side).

## 7. Floating command bar

Otto's front door, inspired by cnvs.dev: **one** shared component,
`lib/components/FloatingBar.svelte`, in two hosts.

```
            ┌──────────────────────────────────────────────────────────┐
            │  (results · Ask Otto row · or the space's thread)        │  ← grows UPWARD,
            ├──────────────────────────────────────────────────────────┤    same glass
            │ ✦  Type or speak…        ⌘K │ (◉ default ▾) │ 01 02 03 04 │ 🎙 │
            └──────────────────────────────────────────────────────────┘
                           (bottom-centre, 16 px above the status bar)
```

**Anatomy.** One glass surface (`--bg-sidebar` at 78% + blur, `--border`,
`--shadow`; a pill at rest, `--radius-l` + 6 px once it opens). The panel above
the pill is drawn in the SAME surface — never a second glass layer.

1. **Input** "Type or speak…" — a `combobox` driving a `listbox` with
   `aria-activedescendant`. Typing ranks the palette's own commands
   (`registry.all`, frecency — `lib/commandSearch.ts`), then an **Ask Otto**
   row, then workspace search hits. When the query reads like a command
   ("go to git") the command is the Enter default; free text defaults to Ask
   Otto. `⌘↵` always asks.
2. **Ask Otto** goes through `ask(text, space)` in `lib/ask.ts` — today the ⌘I
   orchestrator engine (`lib/orchestrate.ts`, shared with the palette). The
   answer lands in the space's thread, attributed ("✦ Ask Otto · planner"). A
   planned change stays a draft with **Run plan / Cancel**; a permanent delete
   asks first. The assistant-threads work swaps the one `ask` binding.
3. **Model chip** — the space's agent + model; opens the space settings
   (name, workspace, agent, `ModelPicker`) in the panel, not a popover.
4. **Spaces 01–04** — four pinned contexts (default Personal · Work ·
   Research · Home), each remembering a name, a workspace (or "follow Otto"),
   agent + model and its last 20 turns. **Home's spaces are the same
   four:** `lib/stores/spaces.svelte.ts` reads and switches the bar's active
   space, and Home's view names are the space names (renaming either renames
   both). `⌃1–⌃4` inside the bar only
   (`keyContext.barFocused` stops the global "jump to session N"); clicking
   the active number opens its settings. Stored per device
   (`otto_bar_spaces`), shared by both hosts. In the main window, switching to
   a space pinned to a workspace selects that workspace.
5. **Mic** — `aria-disabled` with an honest tooltip until voice ships. Never
   fake it.

**Hosts.**

- **In-app** (`shell/App.svelte`, desktop width, not pop-outs): **⌘K focuses
  the bar** — it is the desktop command surface. The `Palette` sheet remains
  for phone/tablet, a hidden bar, and ⌘I (the full plain-English sheet).
- **⌥Space panel** (`#/bar`, `modules/desktop/BarHost.svelte`): the page is
  transparent over native HUD glass; it calls `bar.resize(h)` so the window
  grows upward (cap 560 px), `bar.hide()` on Esc, focuses on
  `otto://bar-shown`, and "Open in Otto" → `openInOtto(route)`. Its commands
  come from the API (Go to, sessions, repos) since no shell registers any.

**It never sits on the user's work.** Presence (`barPresence` in
`lib/floatingBar.ts`, user setting in Settings → Appearance → Floating bar):

| State | When | Looks like |
|---|---|---|
| `full` | focused/open, Home, or "Always full" (while no field has focus) | the whole pill |
| `rest` | "Always full" while a terminal / editor / field has focus | a 260 px "✦ Type or speak… ⌘K" pill |
| `dock` | Auto on any page but Home, scrolling, a terminal / editor / field has focus, or "Docked" | a 20 px chip *inside the status bar* — covers no content, takes no keys |
| `away` | a Modal, sheet or the palette is up | hidden and `inert` |
| `off` | "Hidden" | not mounted; ⌘K opens the palette |

In Auto the pill floats only on Home; every other page (API responses, result
grids, diffs) gets the chip. While the bar can rest over content it sets `--fb-clearance` on the content
column; `PageBody` (and Home) pad their scroll end by it so the last row can
always scroll clear. The panel is clamped to the window (`panelBudget`) and
scrolls; e2e asserts it with `expectFullyInViewport`
(`ui/e2e/desktop-floating-bar.spec.ts`).

**Rules:**

- One bar per window. Don't build a second command system or a page-level
  floating bar; register ⌘K commands and they appear in it.
- At most 720 px wide, centred, 16 px above the status bar. Never on phone.
- Glass rules from [foundations.md §7](./foundations.md#7-translucency-vibrancy-and-the-ambient-backdrop):
  78% tint (it floats over content, not the banded backdrop), opaque
  `--surface` fallback without `backdrop-filter`, opaque under reduced
  transparency. Motion off under reduced motion.
- Keyboard: `Esc` clears the query, then closes (the panel hides the window);
  `Enter` runs the selection; `↑↓` wrap. The pill is never the only way to
  reach an action.

**Ambient backdrop.** Shipped: see
[foundations.md §7](./foundations.md#7-translucency-vibrancy-and-the-ambient-backdrop).
It shows through chrome on every page and fills only Home's desktop (and, when
a studio adopts it, the pasteboard). Content cards on top of it stay opaque
`--surface`. It is off under reduced transparency and when the user picks
**None**.
