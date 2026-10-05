# Accessibility

The target is **WCAG 2.1 AA** in all five theme and scheme combinations, in both
text directions, with the keyboard alone. Many Otto users run long agent
sessions all day, so legibility and keyboard reach matter every day, not only
in audits.

---

## 1. Contrast

| Content | Minimum |
|---|---|
| Body and UI text (anything under 18.66 px bold / 24 px regular) | **4.5:1** |
| Large text, icons that carry meaning, focus rings, input borders, status dots | **3:1** |

- Build from tokens and you mostly pass by construction. The measured table is
  in [foundations.md §1.4](./foundations.md#14-contrast-measured-values). The
  traps:
  - Text tokens are measured on every surface and on the accent tints
    (`tokenContrast.test.ts`). A tint you mix yourself is not: measure it,
    or use `--text` on it.
  - `--accent` as text fails in several themes: use `--accent-text`.
  - Anything on `--accent-solid` must be `--accent-contrast`, which is dark
    text in Warm dark.
  - Status indicator colours (`--status-*`) are for dots and bars, not text.
  - A custom accent is handled by `accentFill()` for fills. Your own
    accent-tinted text must use `--accent-text`, which is mixed toward
    `--text`; custom hues are measured against the active theme surfaces before
    choosing that mix.
- **Check** anything you tint, overlay or put on a translucent material, in
  light and in dark. `ui/e2e/desktop-dialogs-tokens.spec.ts` has a `contrastOf`
  helper that composites real computed styles. Copy it for new assertions.
- Placeholders use the full `--text-dim` (they used to fail at 2.9:1). Don't
  fade them further.

## 2. Focus

- There is a global `:focus-visible` ring in `app.css`: 2 px, opaque `--accent-text`,
  1 px offset. Inputs use a contrast-checked `--accent-text` border plus a
  soft 3 px accent halo instead.
- **Never remove an outline without a replacement.**
  `outline: none` / `outline: 0` is allowed only if the same rule set also
  styles `:focus-visible`: a ring, a border or a background. The audit found 25
  files that remove it with no replacement; don't add more.
- Focus must be **visible on every surface**, including translucent chrome and
  selected rows. Check that the ring shows over `--accent-soft`.
- Focus order follows the visual order. Don't use a positive `tabindex`.
- Modals move focus in and give it back (`Modal` does this). Custom popovers
  and drawers must do the same.

## 3. Keyboard

- **Every interactive element is a real control**: `<button>`, `<a href>`,
  `<input>`, `<select>`, `<textarea>`. `onclick` on a `div`, `span`, `li`, `tr`
  or `td` is a bug; the audit found 98. If a whole row is clickable, put a
  `<button>` in it or make the row a button.
- **Don't add `svelte-ignore a11y_*`** to make a warning go away. There are 172
  suppressions already, and the number should only go down. Fix the markup
  instead.
- **Composite widgets need their keys:**

  | Widget | Keys |
  |---|---|
  | `role="tablist"` | ←/→ (and Home/End) move between tabs; Tab moves into the panel |
  | `role="menu"` | ↑/↓ (and Home/End) move, Enter/Space activate, a letter jumps to an item, Esc closes. `ContextMenu` implements all of this; use `ctxMenu` rather than a hand-rolled menu. |
  | Lists and trees | ↑/↓ move, Enter opens, ←/→ collapse and expand |
  | Grids | Arrow keys between cells, Enter to edit, Esc to cancel |

- `Esc` closes the top-most layer only. Global shortcuts don't fire while an
  overlay is open. `ui.overlayOpen` covers `Modal`, `Drawer`, the palette and
  the new-session sheets, so register overlays properly (`ui.pushModal()`).
- **A right-click menu has a keyboard and a touch path.** Put `use:rowMenu`
  (`lib/rowMenu.ts`) on any element with `oncontextmenu`: the ContextMenu key,
  ⇧F10 and a touch long-press then open the same menu
  (`node scripts/codemods/row-menu.mjs` adds it). The chord works while focus
  is on the element or any non-editable control inside it (a header's sort
  button), so a non-focusable `<div>`/`<tr>` host is still reachable as long
  as it contains a focusable control; a host with none needs `tabindex="0"`. A double-click action is
  also in that menu, or has a key (canvas nodes: Enter / F2 edits).
- **Single-key shortcuts** (no modifier) act only while focus is inside their
  surface, and can be switched off (WCAG 2.1.4) — see the Cluster workspace.
- Everything reachable by pointer is reachable by keyboard, and every major
  verb is in ⌘K ([patterns.md §9](./patterns.md#9-keyboard-first)).

## 4. Names, roles and states

- **Icon-only buttons need both `aria-label` and `title`, with the same text.**
  `PageHeader` also reads them to label the rows of its overflow menu.

  ```svelte
  <button class="icon-btn" onclick={remove} aria-label="Delete pack" title="Delete pack">
    <Icon name="trash" size={14} />
  </button>
  ```
- **Shortcuts are not part of the label.** Put a button's chord in
  `aria-keyshortcuts` (`aria-keyshortcuts="Meta+Shift+C"`), the ⌘K
  command's `shortcut`, a menu row's `hint` and the `KEYMAP` table — never
  appended to `title` ("Collapse panel (⌘J)" breaks the label = tooltip rule).
- `Icon` renders `aria-hidden="true"`. The meaning must come from the control's
  label or visible text, never from the SVG.
- **Form fields have labels**: `<label for>`, a wrapping `<label>`, or
  `aria-label`. A placeholder is not a label. `ui-guards`' `unlabeled-control`
  rule fails any `<input>` / `<select>` / `<textarea>` without one (hidden
  file pickers are exempt).
- **Toggles** expose their state:
  - `aria-pressed` on toggle buttons and value-segmented controls
  - `aria-selected` on tabs
  - `aria-expanded` on disclosures and section headers (the Navigator's group
    headers do this)
  - `role="switch"` plus `aria-checked` for switches
- **Live regions:**
  - `Toasts` is `aria-live="polite"`.
  - Status that changes without user action (a run finishing, a session
    needing you) should be announced politely or reachable from the bell.
  - Loading regions get `aria-busy="true"` (as `Skeleton` sets) or
    `role="status"`.
- **Colour is never the only signal.** Status dots carry a `title` and sit next
  to a text label. Diffs have `+`/`−`. Chips carry words.
  Required/invalid fields have text, not only a red border.
- Give landmarks names: `<nav aria-label="Navigator">`, one `<h1>` per page
  (`PageHeader` renders it), then `<h2>`/`<h3>` in order.

## 5. Motion and transparency

- Respect `prefers-reduced-motion: reduce`. Pulses, shimmers and sheet
  animations stop or become instant. `app.css` has a global override that
  collapses every animation and transition, so a plain animation is covered.
  Add your own `@media` block only when the state needs a different treatment
  (a busy spinner becomes a static dotted ring; the shared `.spinner` already
  does this), see [foundations.md §8](./foundations.md#8-motion).
- Respect `prefers-reduced-transparency` on translucent chrome where the engine
  supports it. The opaque fallback must look right on its own
  ([foundations.md §7](./foundations.md#7-translucency-vibrancy-and-the-ambient-backdrop)).
- Nothing flashes more than 3 times a second.

## 6. Right-to-left

Otto supports RTL. `ui.direction` sets `<html dir>`, and `ui/e2e/rtl.spec.ts`
covers it. The layout mirrors itself **if you use logical properties**:

| Instead of | Use |
|---|---|
| `margin-left` / `padding-right` | `margin-inline-start` / `padding-inline-end` |
| `left: 0` / `right: 0` | `inset-inline-start: 0` / `inset-inline-end: 0` |
| `border-left` | `border-inline-start` |
| `text-align: left` | `text-align: start` |
| `float: left` | `float: inline-start` |

- **Asymmetric shapes** need an explicit RTL override. For example, the active
  nav bar's radius:
  ```css
  .nav-item.active::before { inset-inline-start: 0; border-radius: 0 2px 2px 0; }
  :global([dir='rtl']) .nav-item.active::before { border-radius: 2px 0 0 2px; }
  ```
- **Directional icons** (`chevronLeft`/`chevronRight`, `arrow`, `send`,
  `undo`, `logout`) flip in RTL — `Icon.svelte` does it for every use (its
  `DIRECTIONAL` set). Never add a per-module `[dir='rtl'] … scaleX(-1)` on an
  Icon (it would flip twice); pass `noflip` when the wrapper rotates the glyph
  (a disclosure chevron). Non-directional icons (play, refresh, clock) don't
  flip, and nothing flips inside a `dir="ltr"` island.
- **Code, paths, commands, terminals and diffs stay LTR** (`dir="ltr"` on the
  container) even in an RTL UI.
- **Every free-text field declares its direction.** A `<textarea>` or text
  `<input>` that holds human language gets `dir="auto"`, so a Hebrew message
  lays out right-to-left and an English one left-to-right whatever the UI
  direction. One that holds code, a path, a URL, a command or a key gets
  `dir="ltr"`. `ui-guards`' `bidi-dir` rule enforces it;
  `node scripts/codemods/bidi-dir.mjs` adds the attribute to new fields.
- About 247 physical `left`/`right` declarations remain in the tree. Convert
  them in any file you touch.

## 7. Zoom and text size

- The app zoom (⌘+ / ⌘− / ⌘0, 0.6×–2×) scales the whole page: native WKWebView
  zoom in Tauri, CSS `zoom` in the browser build. At 1.2× and above the
  viewport is effectively narrower, so a desktop layout can drop into the
  tablet breakpoint. Layouts must survive two zoom-in steps without clipped
  controls or horizontal page scroll.
- Don't lock text into fixed-height boxes. Let rows grow when a label wraps or
  gets translated. `PageHeader`'s fixed row is the deliberate exception: it
  truncates the subtitle with an ellipsis and keeps the full text in `title`.

## 8. Testing

| Tool | What it catches | Notes |
|---|---|---|
| `npm run check` | Unknown icon names (`IconName`), type errors, undefined CSS variables, native dialogs | Runs in CI. It does **not** check contrast or ARIA. |
| `expectAccessible(page)` (`ui/e2e/helpers.ts`) | axe `wcag2a`/`wcag2aa`/`wcag21a`/`wcag21aa` | **Fails only on `critical`.** It returns every violation, so assert on `serious` `color-contrast` yourself for new surfaces: `expect(v.filter(x => x.id === 'color-contrast')).toEqual([])` |
| `ui/e2e/theme.spec.ts` | Light-mode overflow plus axe on every page | Forces light with `localStorage.setItem('otto_scheme', 'light')` in an init script |
| `ui/e2e/rtl.spec.ts` | RTL layout | Add your page's key flows |
| `contrastOf()` (`desktop-dialogs-tokens.spec.ts`) | Real composited contrast of one element | Use it for tinted or translucent surfaces |
| Manual | Keyboard-only pass, VoiceOver spot check (⌘F5), reduced-motion toggle, both schemes, one custom accent | Do this before asking for review |
