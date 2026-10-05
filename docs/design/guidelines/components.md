# Components

When to use each shared building block, and its API. Shared components live in
`ui/src/lib/components/`, stores in `ui/src/lib/`, and global classes in
`ui/src/app.css`.

**Reuse first.** The code audit counted 1,040 custom-styled buttons, 358 local
chip styles, 81 local tab styles and 133 local empty-state styles, and each one
drifts on its own. If the shared piece is missing something, extend it in
`lib/` (one PR, used by everyone). Don't fork a local copy.

Planned primitives, not built yet, are listed in [§12](#12-planned-primitives-tbd).
Until they exist, use the classes and patterns below.

---

## 1. Buttons

Global classes, defined in `app.css`:

| Class | Look | Use |
|---|---|---|
| `.btn` | 26 px, `--surface` fill, hairline border | Default secondary action |
| `.btn.primary` | `--accent-solid` fill, `--accent-contrast` text | **The** primary action of a view. One per view. |
| `.btn.ghost` | No border, no fill, `--surface-2` on hover | Tertiary actions, toolbar text buttons, "Cancel" inside dense panes |
| `.btn.danger` | `--danger` text, `--danger-soft` on hover | Destructive actions in lists and menus (the *request* to delete) |
| `.btn.small` | 22 px, `--fs-s` | In-pane toolbars, `PageHeader` actions, table rows |
| `.icon-btn` | 24 × 24, transparent, `--text-dim` | Icon-only actions: close, add, overflow, row tools |

The filled red button (`--danger-solid`) appears in exactly one place: the
confirm button of `ConfirmDialog` when `danger` is true. Don't build filled red
buttons elsewhere.

**Hierarchy**

- **One primary per view.** A view is the page header, a modal footer, or an
  empty state. When the page is empty, the empty state's CTA is the primary
  and the header drops its own.
- The primary sits at the trailing end of its group, with secondary actions
  before it. In a modal footer the order is `Cancel` then the primary, both
  right-aligned (`Modal`'s footer is `justify-content: flex-end`).
- Destructive actions never sit right next to the primary.
- Two equally weighted buttons that do nearly the same thing means one of them
  should go.

**Anatomy**

```svelte
<!-- icon + label: icon first, 12px in .small, 13–14px in .btn -->
<button class="btn small" onclick={refresh}><Icon name="refresh" size={12} /> Refresh</button>

<!-- icon-only: aria-label AND title, same text -->
<button class="icon-btn" onclick={close} aria-label="Close panel" title="Close panel">
  <Icon name="x" size={14} />
</button>

<!-- busy: disable and say what is happening; don't swap in a spinner-only button -->
<button class="btn" onclick={download} disabled={busy}>
  <Icon name="download" size={13} /> {busy ? 'Preparing…' : 'Download support bundle'}
</button>
```

**Don't**

- Redefine `.btn` inside a component (`.btn { width: 30px }`). The global side
  padding still applies and squashes the icon. Use `.icon-btn` for icon-only
  buttons.
- Invent module button systems (`.p-btn`, `.tb-btn`…).
- Put `onclick` on a `div` or `span`. Use a `<button>`, which also gets you
  keyboard access and the focus ring.
- Write "OK", "Yes" or "Submit". Name the action ([content.md](./content.md#2-verbs)).

## 2. Forms

```svelte
<div class="field">
  <label for="task-name">Name</label>
  <input id="task-name" class="input" bind:value={name} placeholder="Weekly dependency report" />
  <span class="hint">Shown in notifications and the report title.</span>
</div>

<label class="checkbox-row"><input type="checkbox" bind:checked={enabled} /> Run on a schedule</label>

<select class="input" bind:value={cadence}>…</select>
<textarea class="input" rows="4" bind:value={prompt}></textarea>
```

| Class | Notes |
|---|---|
| `.field` | A column: label (`--fs-s`, 500, `--text-dim`), control, `.hint` (`--fs-xs`) |
| `.input` | 27 px, `--surface-2` fill. Focus is an `--accent` border plus a 3 px 22% ring. The placeholder is the full `--text-dim` (it was 2.9:1 before). |
| `.checkbox-row` | A checkbox or radio with its label on the trailing side |
| `.pill-toggle` / `.pill-toggle.on` | A compact on/off pill for options that are visible at a glance (orchestrator, filters) |

Rules:

- **Every control has a label**: a visible `<label for>`, or `aria-label` when
  the context makes it obvious (a search field with a search icon). A
  placeholder is an example value, never the label.
- **Validation is inline.** Put the message under the field in `--danger`,
  saying what is wrong and how to fix it. Show it on blur or on submit, not on
  every keystroke. Don't use a toast for a field error.
- **Explain disabled controls.** Give them a `title`, or a `.hint` saying why
  ("Connect a Git account to enable").
- Paths use `PathField` / `FolderPicker`. Model selection uses `ModelPicker`.
  Code and query input uses `CodeEditor`. Don't build new pickers for these.
- Secrets (tokens, passwords) are `type="password"` fields that are **written
  to the Keychain through the daemon** and never echoed back. After saving,
  show "Stored in Keychain" and a Replace action.
- **Advanced options** go behind a disclosure ("Advanced ▸"). A creation form
  shows the 3–5 fields most people need. The New Session sheet is the known
  offender.

## 3. Tabs and segmented controls

For 2–5 options that switch a view, use `.segmented`. Give it tab semantics when
it switches content:

```svelte
<div class="segmented" role="tablist" aria-label="Insights view">
  <button role="tab" aria-selected={tab === 'reports'} class:active={tab === 'reports'}
          onclick={() => router.go('insights')}>Reports</button>
  <button role="tab" aria-selected={tab === 'health'} class:active={tab === 'health'}
          onclick={() => router.go('insights/health')}>Health</button>
</div>
```

- Page-level tabs go in `PageHeader`'s `tabs` snippet (inline, or `below` for
  longer sets). Never put them above the title.
- When a tab is addressable, the tab is a **route** (`#/insights/health`), so
  back/forward and deep links work.
- A segmented control that picks a *value* (List | Graph, Desktop | Tablet |
  Phone, Unstaged | Staged) uses `aria-pressed` on each button instead of tab
  roles, inside `role="group"` + `aria-label`. `class:active` alone is
  invisible to VoiceOver — `ui-guards` fails it (`segmented-state`). A group of
  plain ACTIONS (Set all to Viewer | Editor | Admin) has no `active` and needs
  neither; keep a destructive action out of such a group.
- Tab counts go after the label in `--text-dim` (`Pending 3`), not in a
  coloured bubble, unless the count means "needs you" (then use `--warning`).
- Selection is shown by the surface lift of `.segmented > .active`. Don't use
  the accent colour, and **never green** (green means success).
- **Keyboard:** tablists support ←/→ (visual direction, so RTL flips) and
  Home/End between tabs, with a roving tabindex (`lib/tabKeys.ts`).

**In-pane tab strips: `Tabs`** (`lib/components/Tabs.svelte`). A strip of
views inside a pane or drawer (Messages / Send / Attributes, Overview / Logs /
Manifest) is `<Tabs>`, not a local `.tabs` style. It is the one underline
look: dim labels, the selected one in `--text` over a 2 px accent underline on
the strip's hairline, `role="tablist"` / `role="tab"` / `aria-selected`,
roving focus with arrow keys, disabled tabs skipped.

| Prop | Notes |
|---|---|
| `tabs` | `{ id, label, icon?, count?, disabled?, title? }[]`. `count` renders after the label in `--text-dim`; `title` says why a tab is disabled. |
| `value` + `onchange(id)` | `onchange` is the one place the view switches (pointer and keyboard). |
| `label` | The tablist's `aria-label`. |
| `idBase` | Tab ids become `{idBase}-tab-{id}` with `aria-controls="{idBase}-panel-{id}"`; give the panel `role="tabpanel"`, that id and `aria-labelledby`. |
| `size` | `'m'` (default) or `'s'` for dense drawers. |
| `activate` | `'focus'` (default), `'click'` when the switch is async or can be refused, `'manual'` (Enter/Space select). |
| `trailing` | A snippet for controls at the end of the strip (outside the tablist). |

```svelte
<Tabs label="Queue details" idBase="sqs" value={tab} onchange={(id) => (tab = id)}
      tabs={[{ id: 'messages', label: 'Messages' }, { id: 'send', label: 'Send', disabled: !canSend, title: 'Needs Edit on SQS' }]} />
<div role="tabpanel" id="sqs-panel-{tab}" aria-labelledby="sqs-tab-{tab}">…</div>
```

Document tabs with a close button (open clusters, open notes, query tabs) are
a different control and keep their own strip. `ui-guards` ratchets every other
`role="tablist"` outside `lib/components` (`local-tablist`; a `.segmented`
view switch is exempt only when it wires `lib/tabKeys` — the class alone is
not): migrate a strip when you touch it. A tablist owns only tabs — put other
controls in `trailing`.

## 4. Chips, badges and status

| Piece | Use |
|---|---|
| `.chip` | A neutral pill (`--fs-xs`, 20 px): tags, kinds, env, counts |
| `.chip.ok` / `.chip.bad` / `.chip.accent` | A tone pill: success / danger / selected-or-link |
| `StatusDot` | The live state of a session. Pass `state={sessionState(…)}` (lib/status.ts); the legacy `status` + `needsYou` props derive the same state. `role="img"` + `aria-label`; pulses stop under reduced motion. See [patterns.md §1](./patterns.md#1-agent-presence-and-live-status) |
| `StatusBadge` | Dot + sentence-case word for any status: `status={runStatus(raw)}` for runs/steps/jobs, or `tone` + `label`. `variant="pill"` (tinted) or `"text"` (dense rows) |
| `EnvBadge` | A connection / cluster / account environment: prod (danger), staging (warning), dev (neutral); `readOnly` shows "RO" on non-prod |
| `ProofStatusChip`, `ProofBadge(s)` | Proof-pack status and badges |
| `modules/mcp/McpPill.svelte` | One component maps a domain vocabulary to a tone (`ok` / `warn` / `bad` / `neutral` / `info`) in one place |

Rules:

- **Map domain values to tones in one pure function per domain** (`lib/status.ts`:
  `sessionState`, `runStatus`, `envTone`; McpPill for MCP). Don't pick colours
  at each call site.
- There is no `.chip.warn` or `.chip.info` yet. Write the variant from tokens:
  ```css
  .chip.warn {
    color: var(--warning);
    border-color: color-mix(in srgb, var(--warning) 35%, transparent);
    background: var(--warning-soft);
  }
  ```
- Chip text is a word, not only a colour. Pair colour with an icon or a dot
  where the chip is scanned in a list.
- Neutral is the default. Sources and kinds (Jira, GitHub, Slack, Kafka…) use a
  neutral chip plus the brand/kind icon. **No per-source hues.**
- Unmet *optional* checks are neutral grey, not red ✕. Red means something
  failed and needs action.

## 5. Cards and sections

- `.card`: `--surface`, `--border`, `--radius-m`, no shadow. Use it for
  dashboard boxes, settings groups and preview tiles, not for every list row.
- `.section-title`: the uppercase `--fs-xs` label above a group. Use one level
  of it. Sub-structure comes from spacing, not from a second smaller label.
- Put an interactive card on a `<button>` or `<a>` (or give it a real button
  inside). Hover is `--hover`; selected is `--accent-soft` plus
  `--border-strong`.

## 6. Tables, lists and data grids

| Need | Use |
|---|---|
| Long lists (hundreds of rows or more) | `VirtualList` (`items`, `estimateHeight`, a `row` snippet, optional `key`) |
| Query results / editable grids | The Database Explorer grid (`modules/database`); don't rebuild it |
| Diffs | `DiffView` |
| Terminal output | `Terminal` |
| Time series | `MetricChart` (with `lib/metric-format.ts` units) |
| Plain table | `<table>` with the rules below. **TBD:** a shared `DataTable` |

Rules for plain tables and dense lists:

- Header row: `--fs-xs` or `--fs-s`, weight 600, `--text-dim`, **sticky**, with
  a `--border` bottom edge.
- Rows 28–32 px, a hairline between rows, `--hover` on hover, `--accent-soft`
  when selected.
- **Right-align numbers** with `font-variant-numeric: tabular-nums`. Units go
  in the header ("Size (MB)") or come from the formatters.
- Truncate long cells with an ellipsis plus `title` (the full value). Show IDs
  shortened, with copy on click.
- Show nulls as a dim italic `NULL` or `—`, never as an empty cell that looks
  like a bug.
- Row actions appear on hover or focus at the trailing edge, **close to the
  row's content** (not 1,000 px away). More than two row actions go into a
  `⋯` `ctxMenu`.
- Wide tables scroll horizontally **inside** their container, never the page.

## 7. Dialogs: `Modal`

`ui/src/lib/components/Modal.svelte` is the only way to make a sheet.

| Prop | Type | Notes |
|---|---|---|
| `title` | `string` | Required. Header text and `aria-label`. |
| `width` | `number` | Default 460. Clamped to `calc(100vw - 24px)`. |
| `onclose` | `() => void` | Required. Esc, backdrop click and ✕ all call it. A backdrop click only counts when the press **started** on the backdrop (a text-selection drag that ends there does not close it). |
| `children` | snippet | Body (scrolls; the sheet caps at the window height). |
| `footer` | snippet | Action row, right-aligned. |
| `dismissable` | `boolean` | Default `true`. `false` blocks Esc, backdrop click and the ✕ while a form is busy (mid-submit); close it yourself when the work settles. |

```svelte
{#if open}
  <Modal title="New scheduled task" width={520} onclose={() => (open = false)}>
    <div class="field">…</div>
    {#snippet footer()}
      <button class="btn" onclick={() => (open = false)}>Cancel</button>
      <button class="btn primary" onclick={create} disabled={!valid}>Create task</button>
    {/snippet}
  </Modal>
{/if}
```

What you get for free, and lose if you hand-roll a dialog:

- Focus moves into the sheet on open and back to the trigger on close.
- Tab is trapped inside the sheet.
- Esc only closes the **top** sheet.
- `ui.pushModal()` registration. This hides the native browser webview (which
  paints above the HTML) and blocks global shortcuts while the sheet is open.
- Sizing that is safe in WKWebView. `100vh` there is the *screen* height, so
  tall hand-rolled modals clip their footer.

Rules:

- Use a Modal for short, blocking tasks: create, rename, confirm, pick. If the
  user will spend minutes in it, it should be a page, a pane or a window.
- The title names the task ("New scheduled task"), and the primary button
  names the action ("Create task").
- **Drawers:** use `shell/Drawer.svelte` (off-canvas slide-over: focus trap via
  `dialogFocus`, Esc, backdrop, `ui.pushModal()`, a 24 px ✕; `title` adds a
  header row, `head` a richer one, `onclose` reports a self-dismiss). Don't
  hand-roll one — the phone `BottomNav` "More" overflow is a titled Drawer too.
- **Docked detail columns** (a resource's details beside its table — AWS
  EC2/RDS, Kubernetes) use `lib/components/DockedDrawer.svelte`: an inline
  column with one header (title or `head` snippet + ✕, Esc closes) on desktop,
  and the shell Drawer as a full-width sheet on phone (or when `modal`). The
  caller renders only the body (tabs first); no local `pushModal` /
  `dialogFocus`.

## 8. Confirm, prompt and choose: `confirmer`

Native `confirm()`, `prompt()` and `alert()` **silently return false, null or
undefined in the Tauri WKWebView**, so a delete guarded by one never runs.
`npm run check` fails on them. Use the in-app dialog (`lib/confirm.svelte.ts`,
rendered once by `ConfirmDialog.svelte`):

```ts
import { confirmer } from '../../lib/confirm.svelte';

// Destructive (the default label is 'Delete'; `danger` is derived from the
// confirm verb: Delete / Remove / Discard / Drop … paint red, anything else doesn't)
if (!(await confirmer.ask(`Delete account "${a.label}"? Its token is removed from the Keychain.`,
                          { title: 'Delete account' }))) return;

// Non-destructive: pass a real confirmLabel (the default is "Delete"). A non-destructive verb is not painted red; pass danger:false to be explicit
const ok = await confirmer.ask(`Start ${label}?`, { title: 'Start instance', confirmLabel: 'Start', danger: false });

// One line of text: resolves the trimmed value, or null on cancel/empty
const name = await confirmer.promptText('View name', { title: 'New view', confirmLabel: 'Create', initial: 'View 2' });

// Several labelled outcomes, plus an optional "remember" checkbox
const { value, remember } = await confirmer.choose('This branch has local changes.', {
  title: 'Switch branch',
  options: [
    { label: 'Stash and switch', value: 'stash', kind: 'primary' },
    { label: 'Discard and switch', value: 'discard', kind: 'danger' },
  ],
  checkboxLabel: 'Remember for this repo',
});
```

- The message states the **consequence**, including the object's name ("Its
  token is removed from the Keychain"), not just "Are you sure?".
- Confirm only what is destructive, irreversible or outward-facing. Confirming
  harmless actions trains people to click through.
- For richer confirmation content (a preview of what will be posted, a diff),
  use a `Modal` with the same button rules.

## 9. Menus and popovers: `ctxMenu`

The global `ctxMenu` store (`lib/contextmenu.svelte.ts`) is rendered once by
`ContextMenu.svelte`. It is **already clamped into the viewport and capped in
height**. Use it for right-click menus, ⋯ overflow menus and "New ▾" menus
rather than hand-rolling a popup.

```ts
import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';

const items: MenuItem[] = [
  { label: 'Open', icon: 'external', action: open },
  { label: 'Copy ID', icon: 'copy', action: () => copyText(row.id) },
  { separator: true },
  { label: 'Delete…', icon: 'trash', danger: true, action: remove },
];
ctxMenu.show(event, items);                                      // MouseEvent or KeyboardEvent
ctxMenu.show(event, repoItems, { filter: true, maxVisible: 12 }); // long, data-driven lists
```

`MenuItem` fields:

| Field | Notes |
|---|---|
| `label` | The row text |
| `icon` | An `IconName` string |
| `danger` | Destructive styling |
| `disabled` | Shown but not selectable |
| `separator` | Renders a divider |
| `pinned` | Always shown, never filtered |
| `action` | Called when the row is chosen |

A keyboard-opened menu anchors to the focused element.

**The floating-UI rule** (from [AGENTS.md](../../../AGENTS.md)). Any menu,
dropdown, popover or typeahead that positions itself with coordinates must:

1. **Clamp into the viewport.** Never "flip" to the other side of the anchor
   without a floor: a popup taller than the window flips to a negative top and
   can't be reached.
2. **Cap its height** against the window (`max-height` plus `overflow-y: auto`)
   so every entry can be scrolled to when the list is data-driven (repos,
   branches, completions…).
3. Measure against `window.innerHeight` (or an `inset: 0` container), **not
   `100vh`**. In the overlay-titlebar WKWebView `100vh` is the screen height.
4. Be covered by an e2e check using `expectFullyInViewport`, with **enough
   seeded items to overflow the window** (see
   `ui/e2e/desktop-git-add-menu.spec.ts`).

A popover with real content (a form, a preview) that can't be a menu:

- sits on `--surface` with `--border`, `--radius-l` and `--glass-shadow`
- closes on Esc and on an outside click
- returns focus to its trigger
- follows the same clamp and cap rules

## 10. Toasts vs inline messages

`toasts` (`lib/toast.svelte.ts`) appear bottom-right. The API is
`toasts.success | info | warn | error(title, body?)`. The default lifetime is
4.5 s; errors stay 12 s.

| Situation | Use |
|---|---|
| The result of an action the user just took, especially if the effect is off-screen or async ("Support bundle downloaded", "PR draft opened") | **Toast** |
| A page, pane or card failed to load | **Inline error** in that region, with Retry |
| A form field is invalid | **Inline**, under the field |
| A background job finished while the user is elsewhere | **Notification** (the bell), plus a toast if they are on the page |
| Something needs a decision | **Dialog** (`confirmer` / `Modal`), never a toast |

Rules:

- **The title says what happened, in words** ("Couldn't delete the workflow").
  Raw exception text goes in the body, if anywhere. The audit found about 494
  calls passing `String(e)` straight through. Don't add more
  ([content.md → Errors](./content.md#4-error-messages)).
- Don't toast things the user can already see change on screen ("Saved" on a
  toggle that visibly flipped).
- A toast can carry **one action** (`action: { label, run }`, rendered as a
  small button; see `lib/toast.svelte.ts`) — use it for "Undo" or "Open" only
  when the action really works. Each toast has its own `role` (`alert` for
  errors, `status` otherwise), so the container is not a live region.
  **TBD:** `reportError()` with a details disclosure.

## 11. Empty, loading and error states

Every data region has four states: **loading, empty, error, loaded**. Design all
four.

**Empty: `EmptyState`**

| Prop | Notes |
|---|---|
| `title` | Required. States the situation ("No proof packs yet"). |
| `body` | One or two sentences on what this is for. |
| `icon` | `IconName`, default `box`. Use the module's icon. |
| `actionLabel` + `onaction` | The one CTA (a `.btn.primary`). |
| `actionIcon` | `IconName`. |
| `variant` | `'page'` (the whole page or main pane; pinned about 15vh from the top, not vertically centred) or `'panel'` (the default, inside a pane or card). |
| `tone` | `'neutral'` (default) or `'error'`: a `--danger-soft` icon tile and `role="alert"`, for a state that is a failure. A failed *load* should use `LoadState` (below) instead. |
| `children` | A quiet secondary link or hint only (`.btn.ghost` or dim text). Never a second primary. |

```svelte
<EmptyState
  variant="page"
  icon="check"
  title="No proof packs yet"
  body="Verified evidence — tests, diffs, CI, reviews, approvals — assembled for each piece of work."
  actionLabel="New proof pack"
  actionIcon="plus"
  onaction={newPack}
/>
```

- Distinguish **empty** ("No workflows yet" plus a CTA) from **filtered empty**
  ("No workflows match 'deploy'" plus "Clear filter") and from **not
  configured** ("Connect a Kafka cluster" shown only when there really are
  none).
- Never show "Connect a cluster" while a cluster is listed.

**Loading**

- Use `Skeleton` (`rows`, `height`, optional `label`) when the layout of the
  result is known (lists, cards). Standalone it is a `role="status"` region
  with a visually hidden "Loading {label}"; inside `LoadState` the wrapper owns
  that announcement.
- Use short, specific text for small regions: "Loading proof packs…", not
  "Loading…".
- A button's own busy state is its disabled label (see [§1](#1-buttons)).
- Don't block the whole page for one region. Don't flash a skeleton for fast
  loads: skip it under about 150 ms, or keep showing stale data while you
  refresh.
- For a small busy indicator use the shared `.spinner` class from `app.css`
  (`--spinner-size` sets the size; give the wrapper `role="status"` and a
  text label). Don't add another spinner `@keyframes`: `otto-spin` and
  `otto-pulse` are global. Under reduced motion `.spinner` becomes a static
  dotted ring.

**Error**

- Inline in the region that failed. It needs:
  - the `warning` icon on a `--danger` tile or accent, with the headline in
    `--text` and `role="alert"`
  - what failed, in words
  - **Retry**
  - raw detail in a dim second line
- A failed page load is an inline error, not a toast.

**The shared state components: use them, don't hand-roll an error block.**
A failed load inside an `{#if}` chain that already handles loading and empty
is still `LoadState`: pass `error` and `empty` (`<LoadState what="signals"
error={err} empty onretry={load} />`) and it renders only the error view —
`variant="compact"` in a narrow rail. `ui-guards` ratchets every
`…Retry</button>` outside `lib/components` (`inline-retry`); a Retry for a
failed *action* (a send, a save banner) is not a load error, keep it and let
the ratchet record it.

| Component | Use |
|---|---|
| `LoadState` (`lib/components/LoadState.svelte`) | The one wrapper for a loaded list/page: `what`, `loading`, `error`, `empty`, `onretry`, `variant` (`page` / `panel` / `compact`), an `emptyView` snippet, and children for the loaded state. Precedence: error with nothing to show, then a full inline "Couldn't load {what}" with Retry; error with stale data, then the data under a slim "Showing the last good load" bar; loading with nothing yet, then a skeleton (never the empty state); empty, then `emptyView`; else the children. A failed load can therefore never read as "No X yet". |
| `LazyMount` (`lib/components/LazyMount.svelte`) | Renders a `lazyComponent` with every state designed: skeleton while the chunk loads, the same inline error with Retry on failure, the component once loaded. `quiet` is for overlays (nothing while loading, a toast on failure). |
| `loadErrorText(e)` (`lib/loadError.ts`) | Turns a caught error into the human cause shown as the detail line (session expired, no access, daemon unreachable). Pass its result as `error`; never the raw exception text. |

```svelte
<LoadState what="scheduled tasks" loading={s.loading} error={s.error}
           empty={s.tasks.length === 0} onretry={() => s.load()}>
  {#snippet emptyView()}<EmptyState title="No scheduled tasks yet" … />{/snippet}
  …the list…
</LoadState>
```

## 12. Primitives: built and planned

**Built** — use these; their sections above are the reference:

| Primitive | Replaces |
|---|---|
| `Badge` (`lib/components/Badge.svelte`: `tone`, `variant`, `size`, `dot`) | `.chip` variants and local pill styles (`local-pill-class` ratchet) |
| `Switch` (`lib/components/Switch.svelte`: `checked`, `onchange`, `label`, `tone`) | `.pill-toggle` and local toggles; a binary setting is a Switch, not a checkbox |
| `Tabs` (`lib/components/Tabs.svelte`, [§3](#3-tabs-and-segmented-controls)) | local in-pane tab strips (`local-tablist` ratchet) |
| `LoadState` / `EmptyState tone="error"` ([§11](#11-empty-loading-and-error-states)) | hand-rolled "Couldn't load + Retry" blocks (`inline-retry` ratchet) |

**Planned** — proposed by the code audit and not built yet. **Don't reference
them until they exist.** Build them in `lib/components/` when the first feature
needs one, and update this doc:

| Primitive | Replaces |
|---|---|
| `Button` / `IconButton` (required `label` → `aria-label` plus tooltip) | `.btn` / `.icon-btn` classes (keep the classes as the implementation) |
| `Segmented` (radiogroup) | `.segmented` without ARIA |
| `Drawer` | `AwsDrawer`, `RefineDrawer`, `ResourceDrawer`, `RulesDrawer` |
| `Spinner` / `Loading` | local spinners (the global `.spinner` covers most) |
| `DataTable` | 51 locally styled `<table>` elements |
| `use:floating` action | The per-component clamp code behind ctxMenu, NamespacePicker and the git ref popover |
