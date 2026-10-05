# Foundations

The design tokens and low-level rules: colour, type, spacing, radius, elevation,
layers, translucency, motion and icons.

All tokens are defined in `ui/src/lib/tokens.css`; global primitives are in
`ui/src/app.css`. **Use only names defined there.** `npm run check` runs
`ui/scripts/ui-guards.mjs`, which fails the build on any `var(--x)` that isn't
defined anywhere under `ui/src/`, fallback or not.

---

## 1. Colour

### 1.1 Themes and schemes

`<html>` carries two attributes, set by `ui.applyTheme()` in
`lib/stores/ui.svelte.ts`:

- `data-theme`: `native` (default), `pro-dark` or `warm`
- `data-scheme`: `light` or `dark`. When the user picks "auto", it is resolved
  from `prefers-color-scheme`. **Pro Dark is always dark.**

That gives five combinations: Native light, Native dark, Pro Dark, Warm light,
Warm dark. The user may also pick a **custom accent**, which is written inline
on `<html>` as `--accent` together with a contrast-checked
`--accent-solid`/`--accent-contrast` pair from `lib/accent.ts` (`accentFill()`).

Rules:

- Never branch on the theme in component CSS. The tokens already change per
  theme and scheme. `html[data-scheme='dark'] …` overrides belong in
  `tokens.css` (and, for now, the highlight.js colours in `app.css`), not in
  modules.
- Never hard-code the accent (`#0a84ff`). It is blue only in Native: violet in
  Pro Dark, green in Warm, and anything at all with a custom accent.
- Test every new surface in at least Native light and Native dark. Check Warm
  dark if you use accent fills, because its accent is bright enough that
  `--accent-contrast` flips to dark text.

### 1.2 Token reference

**Surfaces**, lowest to highest. This is also the elevation ladder:

| Token | Use for |
|---|---|
| `--bg` | Window and page background, content behind cards (and the base under the toolbar glass) |
| `--bg-sidebar` | Sidebar tint (see [§7 Translucency](#7-translucency-vibrancy-and-the-ambient-backdrop)) |
| `--surface` | Cards, sheets (Modal), popovers, menus, toasts, list panes |
| `--surface-2` | Inputs, wells, code blocks, hovered buttons, segmented-control track, empty-state icon tile |
| `--surface-3` | Pressed or nested wells, a selected cell inside a `--surface-2` well. Use sparingly. |
| `--term-bg` | Terminal and log panes only |

**Lines and interaction**

| Token | Use for |
|---|---|
| `--border` | Hairline borders: cards, inputs, dividers inside content |
| `--separator` | The quieter hairline between chrome and content: the `PageHeader` bottom edge, the sidebar's inline-end edge, the status bar's top edge, a Modal footer, stat dividers in Home widgets |
| `--border-strong` | Emphasised borders: focused/selected cards, drag targets, the agent-content rule (see patterns.md) |
| `--hover` | Hover wash on rows and ghost controls (7% of `--text`, works on any surface) |
| `--scrim` | The dimmed backdrop behind a `Modal` or `Drawer` (incl. the phone "More" drawer) (per scheme) |
| `--scrim-soft` | A lighter backdrop for the command palette, which should keep the page legible |
| `--scrim-media` | The veil behind a caption or control sitting on an image, video or 3D viewport (same in both schemes; text on it is white) |

**Text**

| Token | Use for |
|---|---|
| `--text` | Primary text, titles, values |
| `--text-dim` | Secondary text: subtitles, metadata, labels, placeholders, dim icons |

**Accent.** Each variant has one job:

| Token | Use for | Never for |
|---|---|---|
| `--accent` | Indicators: the 3 px active-nav bar, active icon tint, progress fills | Text, or a fill under text |
| `--accent-text` | Links, accent-coloured labels, `.chip.accent`, breadcrumb hover, focus outlines/input borders | Large fills |
| `--accent-solid` + `--accent-contrast` | A filled primary button and its label (`.btn.primary`) | Anything that isn't the one primary action |
| `--accent-soft` | Selected rows, the active nav item, "on" toggles (14% tint) | Large backgrounds, whole cards |

**Semantic tones** (text-safe). The bare name is ≥ 4.5:1 on `--bg`, `--surface`,
`--surface-2` and on its own `-soft` tint in every theme:

| Tone | Text/icon | Tinted background | Filled | Meaning |
|---|---|---|---|---|
| danger | `--danger` | `--danger-soft` | `--danger-solid` (white text; the confirm button only) | failed, destructive, error, prod |
| warning | `--warning` | `--warning-soft` | — | needs attention, pending, degraded, dirty |
| success | `--success` | `--success-soft` | — | passed, healthy, done, connected |
| info | `--info` | `--info-soft` | — | informational, neutral-positive, in progress (non-live) |

**Status indicators.** Dots, bars and rails only; they only need 3:1 as
graphics. They are what `StatusDot` renders:

| Token | State |
|---|---|
| `--status-working` | live and actively working (pulses) |
| `--status-idle` | idle |
| `--status-exited` | exited or failed |
| `--status-warn` / `--status-warn-soft` | attention: needs you, ahead/behind, waiting (a suspended session is a hollow idle ring, not amber) |

`--status-warn` is an alias of `--warning` and `--status-idle` an alias of
`--text-dim` in every theme and scheme, so a dot and the text beside it are
always the same amber or grey. `--status-working` and `--status-exited` alias
`--success` / `--danger` in the light scheme; in dark they stay brighter for
the 3:1 graphic contrast on glass.

Use a tone token for text and a status token for dots. Don't write a status
label in `--status-working`; use `--success`.

### 1.3 Semantic usage: do and don't

```css
/* Do: tone tokens, soft tints, borders mixed from the same tone */
.row.failed { background: var(--danger-soft); }
.err-msg    { color: var(--danger); }
.pill.warn  {
  color: var(--warning);
  background: var(--warning-soft);
  border: 1px solid color-mix(in srgb, var(--warning) 35%, transparent);
}
.row.selected { background: var(--accent-soft); }

/* Don't: literal hex (scheme-specific, ignores themes) */
.err-msg  { color: #ff5f57; }
/* Don't: fallback on a token that doesn't exist (always renders the fallback) */
.title    { color: var(--fg, #e6edf3); }
/* Don't: accent as text (fails contrast in 3 of 5 themes) */
.link     { color: var(--accent); }
/* Don't: status indicator colour as text */
.ok-label { color: var(--status-working); }
```

- **Colour never carries meaning alone.** Always pair it with a word, an icon
  or a shape. `StatusDot` has a `title`; chips carry text; diffs carry `+`/`−`.
- **No categorical rainbows.** Label sources and kinds (Jira, GitHub, Slack…)
  with neutral chips and their icon, not eight hues. (Run with Otto's source
  chips are the known offender.)
- **Studio badges** (Design Hall) are the one categorical exception:
  `--studio-frames`, `--studio-graphics`, `--studio-site`, `--studio-3d`,
  `--studio-whiteboard`, `--studio-brand` and `--studio-spatial` fill a
  studio's small badge tile behind a `--studio-glyph` icon, always next to the
  studio's name. They are never text, status or a large area.
- **Git worktree identity**: `--worktree` (pip fill, tints, borders) and
  `--worktree-text` (text-safe) mark a branch checked out in another
  worktree, so it never reads as the local-branch accent. Defined per scheme.
- **Chart series** use the categorical palette `--cat-1…6` (per scheme,
  graphics only: bars, segments, legend swatches — never text or status).
  Always pair a series with its legend label. Other kinds (sources, features)
  are told apart by icon and label on a neutral chip.
- **Agent identity has no colour token.** An agent is told apart by its
  provider icon and name (`AgentChip`, `AgentByline`) on neutral chrome — never
  by the accent or an extra hue — so there is no `--agent` token. See
  [patterns.md → Agent-authored content](./patterns.md#2-agent-authored-content).

### 1.4 Contrast: measured values

WCAG ratios computed from the current `tokens.css` values (tints composited
over `--surface`). AA needs **4.5:1** for text and **3:1** for large text and
UI graphics.

| Pair | Native light | Native dark | Pro Dark | Warm light | Warm dark |
|---|---|---|---|---|---|
| `--text` on `--bg` | 15.5 | 14.9 | 14.8 | 10.8 | 13.0 |
| `--text-dim` on `--surface-2` | 6.42 | 6.52 | 6.38 | 6.34 | 7.17 |
| `--text-dim` on `--surface-3` | 5.87 | 5.78 | 5.71 | 5.79 | 6.44 |
| `--text-dim` on `--accent-soft-strong` over `--surface-3` | 4.62 | 4.63 | 4.65 | 4.64 | 4.63 |
| `--accent-text` on `--bg` | 6.74 | 8.51 | 7.67 | 7.02 | 9.03 |
| `--accent-text` on `--accent-soft-strong` over `--surface-3` | 4.65 | 4.64 | 4.66 | 4.63 | 4.61 |
| `--accent-contrast` on `--accent-solid` | 4.94 | 4.94 | 6.38 | 4.76 | 6.83 |
| `--danger` on `--surface-2` | 5.75 | 5.58 | 6.57 | 5.60 | 5.70 |
| `--warning` on `--surface-2` | 5.67 | 6.54 | 7.70 | 5.52 | 6.68 |
| `--success` on `--surface-2` | 5.44 | 6.45 | 7.59 | 5.30 | 6.59 |
| `--info` on `--surface-2` | 5.96 | 5.73 | 6.75 | 5.80 | 5.85 |
| tone on its own `-soft` tint | 5.0–5.5 | 4.9–5.5 | 5.6–6.3 | 5.0–5.5 | 4.9–5.5 |

Rules that follow from the table:

- **Every text token clears AA on every ground and every accent tint.**
  `--text`, `--text-dim` and `--accent-text` are ≥ 4.5:1 on `--bg`,
  `--bg-sidebar`, `--surface…-3` and on `--accent-soft` / `--accent-soft-strong`
  over each — a selected (and hovered) row on `--surface-3` was the worst case
  at 3.5–4.0:1, so `--text-dim` moved toward `--text` (Native light `#56565b`,
  Native dark `#b9b9bf`, Pro Dark `#a8a8b5`, Warm light `#59554f`, Warm dark
  `#c5c0b5`) and each theme sets its own `--accent-text` mix.
  `ui/unit/tokenContrast.test.ts` measures the whole matrix; a custom accent's
  `--accent-text` is chosen against the same tints (`lib/accent.ts`).
- **`--text-dim` clears AA on `--surface-3` in every theme.** Native dark
  (`#9f9fa6` → `#acacb3`), Warm light (`#6b6760` → `#605c56`) and Warm dark
  (`#a09a8e` → `#ada79b`) were nudged for it; it stays far dimmer than `--text`
  (about 10:1 or more on the same surface), so the hierarchy still reads.
  (Native light's `--text-dim` moved from `#69696e` to `#636368` in the ambient
  pass so dim text clears AA on the sidebar glass.)
- **Code colours are tokens.** Syntax tokens (`--code-keyword`, `--code-string`,
  `--code-number`, `--code-builtin`) and the find-in-page highlights
  (`--code-find`, `--code-find-current`, `--code-find-text`) are each ≥ 4.5:1 on
  `--surface`, `--surface-2` and `--surface-3` of their scheme. Never write a
  hex in a highlight rule.
- Tones are safe on `--bg`, `--surface`, `--surface-2`, `--surface-3` and their
  own soft tint. Don't put a tone on another tone's tint (for example
  `--warning` text on `--danger-soft`).
- Nothing on a `--accent-solid` fill except `--accent-contrast`. Don't assume
  white: Warm dark uses near-black.
- Text over an image or a translucent material must be checked against the
  **worst** backdrop it can sit on. See [§7](#7-translucency-vibrancy-and-the-ambient-backdrop)
  for the measured glass table.

### 1.5 Where literal colours are still allowed

Literal colours in module styles aren't checked automatically; reviewers check
them (see [review-checklist.md](./review-checklist.md)). The only acceptable
places:

- brand marks (`ProviderIcon`'s Claude/Codex/Antigravity art)
- the terminal palette (`lib/termtheme.ts`)
- user content previews (iframes, device frames, a designed site on a canvas)
- the find-in-page highlights and highlight.js colours in `app.css`. These are
  existing exceptions; move them into tokens when you touch them.
- black-alpha shadows (`rgba(0,0,0,.x)`)

Everything else uses a token. When migrating an old module, map:

| Legacy | Use |
|---|---|
| `--fg` | `--text` |
| `--fg-muted`, `--muted`, `--text-muted`, `--text-2/3` | `--text-dim` |
| `--mono` | `--font-mono` |
| `--radius` | `--radius-m` |
| `--bg-2`, `--bg-secondary`, `--surface-raised` | `--surface-2` |
| `--border-1` | `--border` |
| `--ok`, `--warn`, `--red` | `--success`, `--warning`, `--danger` |
| `#7ee787` / `#febc2e` / `#ff5f57` etc. | the matching tone |

---

## 2. Typography

Two families only:

- `--font-ui` (the system font: SF Pro)
- `--font-mono` (`ui-monospace`, SF Mono, Menlo)

### 2.1 Scale

| Token | px | Use |
|---|---|---|
| `--fs-xs` | 11 | **The floor.** Metadata, chip text, hints, section titles (uppercase), table meta columns |
| `--fs-s` | 12 | Secondary body, the `PageHeader` subtitle, labels, small buttons (`.btn.small`), segmented controls, code and mono (`code`, `pre`, `.mono`) |
| `--fs-m` | 13 | **Body default** (`body`), buttons, inputs, list rows |
| `--fs-l` | 15 | The page title (`PageHeader` h1), page-level empty-state title |
| `--fs-xl` | 18 | Hero numbers in KPI tiles, rare in-content headings |
| `--fs-2xl` | 22 | Dashboard hero figures; the top heading of long-form **content** (a walkthrough step, an article in the browser's reader view); a number card's empty dash — never chrome |
| `--fs-hero` | 28 | The wordmark on the boot, sign-in and onboarding screens; the single big number of a data-viz number card (`database/Chart`) — never body or page titles |

Rules:

- **11 px floor.** Anything a user has to read is at least `--fs-xs`. Sizes
  below that are allowed only for decorative counters (a badge count on an
  icon) and never for words.
- **No new px font sizes.** Use the tokens. When you touch a legacy file, map
  its values to the nearest step:

  | Legacy value | Token |
  |---|---|
  | 8–10.5 px | `--fs-xs` |
  | 11.5 px | `--fs-xs` or `--fs-s` |
  | 12.5 px | `--fs-s` or `--fs-m` |
  | 14 px | `--fs-m` or `--fs-l` |
  | 16–18 px | `--fs-l` or `--fs-xl` |
  | `0.85–0.95em` (inline code in prose) | `--fs-s` |

  `npm run check` ratchets every px/em/rem font-size of 11 px or more that
  isn't a `--fs-*` step (`font-size-literal`). The one exception: a text
  input set to `16px` so iOS Safari doesn't zoom on focus, marked with
  `ui-guards: allow` on the line.
- **One title size.** Every page title is `--fs-l`/600 in `PageHeader`. Don't
  give a page a bigger h1. (Legacy `.page-header h1` is aligned to `--fs-l`.)
- **Content exception.** Rendered long-form content — a walkthrough step
  (`help/Walkthroughs`), an article in the reader view (`browser/ReaderView`),
  markdown bodies — is a document, not chrome: its own top heading may use
  `--fs-2xl`/600 (and `--fs-xl` below it). The page title above it in
  `PageHeader` still stays `--fs-l`.
- **Tracking.** Two letter-spacing values exist: `.06em` on uppercase
  micro-labels, and `-0.01em` on titles at `--fs-l` and above (page titles,
  hero figures, content headings) — large sans text reads loose at 0. Body,
  controls and mono stay at the default (0). `npm run check` ratchets any
  other value (`letter-spacing-literal`).
- **Weights:** 400 (body), 500 (buttons, labels, chips), 600 (titles, active
  nav item, table headers). No 700/800 in chrome.
- **Uppercase micro-labels** exist in one form only: `.section-title`
  (`--fs-xs`, 600, `letter-spacing: .06em`, `--text-dim`). Don't invent
  9–10 px letter-spaced labels.
- **Mono** for identifiers, paths, hashes, commands, code, and numbers in
  columns. Add `font-variant-numeric: tabular-nums` to columns of numbers,
  sans or mono, so digits line up.
- **Line height:** body 1.45 (global), markdown 1.6 (`.md-body`), single-line
  controls set explicit heights instead.

Sidebar section labels, the Workspaces label and palette group text all use
`--fs-xs` (the 11 px floor), like every other uppercase micro-label.

---

## 3. Spacing and sizing

The scale is **2 px steps up to 24 px, 4 px steps above** (28, 32, 40, 48…).
The named rungs are tokens in `tokens.css`:

| Token | `--sp-1` | `--sp-2` | `--sp-3` | `--sp-4` | `--sp-5` | `--sp-6` | `--sp-7` | `--sp-8` | `--sp-9` |
|---|---|---|---|---|---|---|---|---|---|
| px | 2 | 4 | 6 | 8 | 12 | 16 | 20 | 24 | 32 |

Prefer the tokens in new shared primitives; a literal px value on the scale is
fine in module styles. The half-steps 10, 14, 18 and 22 px are allowed for
control internals (button side padding, row padding) and must not be used for
layout gaps between groups. Odd values (3, 5, 7, 9, 11, 13 px) are off the
scale: `ui-guards` ratchets them (`off-grid-spacing`), and the remaining
baseline is virtualized rows whose JS assumes a pixel height.

Fixed dimensions to match, as used by the shared primitives:

| Element | Size |
|---|---|
| `PageHeader` row | 46 px (`--ph-h`); horizontal padding 20/16 px (14/10 px on phone) |
| `PageBody` padding | 18 px top, 20 px sides, 40 px bottom (12/14/32 px on phone) |
| Readable column | `--page-readable: 1200px` (`PageBody width="readable"`) |
| `.btn` | 26 px high, 10 px side padding, 6 px icon gap |
| `.btn.small` | 22 px high, 8 px side padding |
| `.icon-btn` | 24 × 24 px (`PageHeader`'s ⋯ is 28 px) |
| `.input` | 27 px high |
| `.chip` | 20 px high, pill |
| `.segmented` button | 22 px high inside a 2 px track |
| Status bar | 24 px |
| Mobile top bar / bottom nav | `--mobile-topbar-h` 44 px / `--mobile-bottomnav-h` 56 px |
| List row | 28–32 px for dense lists, 36–44 px for two-line rows |

Gaps: 6 px between toolbar controls, 8 px between related controls, 12 px
between groups, 16–24 px between sections. Line things up to a shared left
edge instead of adding more space.

---

## 4. Radius

| Token | px | Use |
|---|---|---|
| `--radius-s` | 5 | Controls: buttons, inputs, icon buttons, nav rows, the focus ring |
| `--radius-m` | 8 | Cards, list items, code blocks, skeleton rows |
| `--radius-l` | 12 | Sheets (Modal), popovers, the notification panel, Home widgets and glance cards, the empty-state icon tile, the floating bar |
| `999px` | pill | `.chip`, `.pill-toggle`, status pills |

Don't use 3 px, 4 px, 6 px or `99px`. (4 px inside the `.segmented` track is
the one existing exception.)

---

## 5. Elevation and shadow

Elevation comes from **surface steps and hairlines first, shadow second**.
There are three levels, each with one token:

| Level | What | Token |
|---|---|---|
| 0: the page | `--bg` (and, on Home, the ambient backdrop) | none |
| 1: content cards | `.card`, Home widgets and glance cards, the empty-space panel on Home: `--surface` + 1 px `--border` | `--shadow-card` (a whisper: 1–2 px, low alpha, tuned per scheme) |
| 2: floating layers | Modal sheets, menus, popovers, the palette, the floating bar | `--glass-shadow`, with a `--glass-border` hairline |

- `.card` carries `--shadow-card` globally, so a card rests just above the page
  and above the ambient backdrop on Home. Don't add a stronger shadow to "lift"
  a card; change its surface step or its border (`--border-strong`) instead.
- Floating layers share one family: `--glass-border` + `--glass-shadow`, whether
  the surface is glass (menus, palette) or opaque (Modal). The legacy
  `--shadow` alias is retired; toasts and popovers use `--glass-shadow` too.
- A selected segment in `.segmented` has a 1 px micro-shadow. That is the only
  other in-flow shadow.

Don't stack shadows, add coloured glows, or invent a fourth level.

---

## 6. Layers (z-index)

The layers are `--z-*` tokens in `lib/tokens.css`. Put new UI in one of them
(`z-index: var(--z-modal)`); don't invent a number. `npm run check` ratchets
z-index literals outside the in-pane range (−1…10).

| Layer | Token (value) | Who |
|---|---|---|
| In-pane stacking | literal 1–10, `--z-sticky` (10) | sticky headers, resize handles, the right-panel edge (5); in the agents grid: pane chrome ≤ 5, split/tile dividers 8, corner grip and pane close 9, the drag-drop veil 10 |
| Floating bar | `--z-floating-bar` (40) | `FloatingBar` over the content column (below every sheet and menu) |
| Mobile chrome | `--z-mobile-nav` (60), `--z-drawer` (90, +1) | `BottomNav`, `Drawer` (90–91; also the phone "More" overflow and `DockedDrawer` sheets) |
| Command surfaces | `--z-command` (150) | `Palette`, `ShortcutsOverlay` |
| Sheets | `--z-modal` (200) | `Modal` (and so `ConfirmDialog`) |
| Toasts | `--z-toast` (300) | `Toasts` |
| Find bar | `--z-find` (9000) | `FindInPage` |
| Menus | `--z-popover-backdrop` (9998), `--z-popover` (9999) | `ContextMenu` backdrop and menu, `NotificationBell` popover |

Rules:

- **A new dialog is a `Modal`.** Don't hand-roll a fixed overlay with its own
  z-index. Hand-rolled overlays at z-index 50 have ended up *under* the mobile
  bottom nav, and they don't register with `ui.pushModal()`, so the native
  browser webview paints over them.
- A menu opened from inside a Modal still goes through `ctxMenu`, which sits
  above sheets.
- Nothing goes above `--z-overlay-max` (9999).
- **Contain a component's own stack** with `isolation: isolate` (or `contain:
  paint`, as the terminal does) when its internals use the in-pane range and
  it hosts no fixed overlay. Don't isolate a container that renders a
  non-portalled `Modal` or lightbox — the overlay would be trapped under its
  later siblings and the app chrome.

---

## 7. Translucency, vibrancy and the ambient backdrop

Otto's chrome is glass over a calm backdrop; its content is opaque. Three
pieces make that work, all wired through `ui/src/lib/tokens.css`:

1. **The ambient backdrop** (`--ambient-image`): a soft, full-bleed image
   behind the window. Settings → Appearance → Backdrop: **None**, **Subtle** (an
   accent wash, the default) or **Wallpaper** (a generated wallpaper, or the
   user's own photo). `lib/ambient.ts` generates it from the accent in effect
   and the resolved scheme; there are no network images. A photo is processed
   on the device by `lib/wallpaper.ts` (downscaled, blurred, clamped into a
   light and a dark variant) and stored in `localStorage` only.
2. **Glass materials**: the `--glass-*` tokens and three classes, for chrome.
3. **Native vibrancy** in the desktop app. The Tauri window is created with
   `"transparent": true` and `window_vibrancy::apply_vibrancy(…,
   NSVisualEffectMaterial::Sidebar)` (`apps/desktop/src-tauri/src/main.rs`,
   `windows.rs`). The shell clears the document background there
   (`html.otto-vibrant`), so the sidebar glass shows the real desktop. A Subtle
   backdrop is translucent and tints the native material; a Wallpaper paints
   its own opaque base.

### 7.1 Tokens and classes

| Token | Value | Used by |
|---|---|---|
| `--ambient-image` | the generated art (`var(--ambient-art)`, set on `<html>` by `ui.applyTheme()`), `none` when off | the shell, `.chrome-material`, Home's desktop |
| `--glass-tint` | `--bg-sidebar` at 72% | `.sidebar-material` over the ambient (browser, remote, tablet) |
| `--glass-tint-native` | `--bg-sidebar` at 84% | the sidebar and the menu-bar popover over **native** vibrancy (the real desktop is not luminance-banded) |
| `--glass-tint-bar` | `--bg` at 72% | `.chrome-material`: the `PageHeader` row and the status bar |
| `--glass-tint-raised` | `--surface` at 90% | `.glass-raised`: context menus, the ⌘K palette, the notification panel |
| `--glass-blur`, `--glass-blur-raised` | `blur(24px) saturate(1.5)`, `blur(20px) saturate(1.4)` | the backdrop-filter that goes with them |
| `--glass-border` | `--text` at 10% | the hairline edge of any floating layer, glass or not |
| `--glass-shadow` | two layers, per scheme | elevation 2 (see [§5](#5-elevation-and-shadow)) |

| Class | What it paints | Where |
|---|---|---|
| `.sidebar-material` | `--glass-tint` + `--glass-blur` over whatever is behind it (the ambient, or native vibrancy) | Navigator, Rail, the pop-out title strip |
| `.chrome-material` | `--bg`, then the ambient **fixed to the viewport** (`background-attachment: fixed`), then `--glass-tint-bar`. There is no backdrop-filter, so it never becomes the containing block of a fixed-position popup inside it. | `PageHeader`, `StatusBar` |
| `.glass-raised` | `--glass-tint-raised` + `--glass-blur-raised` + `--glass-border` + `--glass-shadow` | `ContextMenu`, `Palette`, the `NotificationBell` panel |

Because `.chrome-material` fixes the ambient to the viewport, the toolbar strip,
the status bar, the sidebar and Home all show one continuous backdrop.

### 7.2 The rule: glass is for chrome, never for content

| Glass allowed (chrome) | Always opaque (content) |
|---|---|
| Sidebar / Rail | Tables, grids, result sets |
| The toolbar strip (`PageHeader`) and the status bar | Editors, terminals, diffs, logs |
| Menus, popovers, the palette, the menu-bar popover (`#/tray`) | Forms and settings bodies |
| The floating command bar | Modal sheets: they hold forms, so they are opaque `--surface` with the glass edge and shadow |
| The ambient backdrop on Home, around the widgets | Anything the user reads for more than a glance |

The content column (`.center` in the shell) is always `--bg`. Only **Home**
paints the ambient behind its content, and only around opaque cards.

### 7.3 The contrast contract

Every ambient pixel stays inside a **scheme luminance band** (`AMBIENT_LUM` in
`lib/ambient.ts`): relative luminance ≥ 0.68 in light schemes and ≤ 0.07 in dark
ones. The generator aims inside it, and a photo is clamped into it per pixel
(mixed toward white or black in linear light). That bound is what lets the
chrome tint drop to 72% and still keep every text token AA over *any* backdrop
the user can pick. `unit/ambient.test.ts` enforces it against the real token
values: it composites each glass tint over every generated sample (every theme
accent plus extreme custom accents, their blends, and clamped extreme photo
pixels) and fails below 4.5:1.

Worst case measured (WCAG ratio; 4.5 is AA):

| Pair (worst backdrop) | Native light | Native dark | Pro Dark | Warm light | Warm dark |
|---|---|---|---|---|---|
| `--text` on sidebar glass | 13.34 | 12.70 | 12.42 | 9.25 | 11.08 |
| `--text-dim` on sidebar glass | 4.74 | 5.40 | 5.46 | 4.59 | 5.03 |
| `--text` on toolbar glass | 14.13 | 13.33 | 13.11 | 9.74 | 11.63 |
| `--text-dim` on toolbar glass | 5.02 | 5.66 | 5.76 | 4.84 | 5.28 |
| `--text` straight on the backdrop (Home greeting) | 12.63 | 9.44 | 8.64 | 8.49 | 8.31 |
| `--text-dim` on raised glass (over app surfaces) | 5.85 | 5.30 | 5.79 | 5.49 | 5.03 |
| `--text` on raised glass (over pure black or white) | 13.42 | 9.32 | 9.91 | 9.03 | 8.25 |

Rules that follow:

1. **Only `--text` sits straight on the backdrop.** `--text-dim` is not AA on
   the raw ambient; put secondary text on a card or on glass.
2. **Never change a glass tint without running `npm run test:unit`.** A tint
   below 70% over the banded backdrop, or below 78% over native vibrancy, fails
   the test by design.
3. **Raised glass assumes app content behind it.** It is measured over the
   scheme's own surfaces for `--text-dim` and over black and white for
   `--text`. Keep dim text in menus short (shortcut hints, meta).
4. **Opaque fallback.** In e2e runs, the phone/remote UI and any browser without
   `backdrop-filter`, the same tints composite over the ambient without blur and
   still pass: the numbers above are blur-free.
5. **Reduced transparency.** `prefers-reduced-transparency: reduce`, or
   Settings → Appearance → *Reduce transparency* (`data-transparency="reduced"`
   on `<html>`), turns every `--glass-*` tint into its opaque token, sets the
   blurs to `none` and sets `--ambient-image: none`. Components need no code of
   their own for it; use the tokens.
6. **One level of glass.** Never put a translucent surface on top of another
   translucent surface. (A menu opened from a menu is fine: each one sits over
   content.)

---

## 8. Motion

Motion confirms an action or shows live state. It is never decoration.

| Use | Duration / easing | Examples |
|---|---|---|
| Hover and press feedback | 120–140 ms `ease-out` | `.btn` (130 ms), `.icon-btn`, `.segmented`, `.pill-toggle` |
| Enter/leave of a floating layer | 140–160 ms `ease-out`, fade plus ≤ 8 px translate, scale ≥ 0.985 | `Modal` backdrop fade and sheet-in |
| Live state (continuous) | 1.4–1.6 s ease-in-out loop | `StatusDot` working pulse (`otto-pulse`), `Skeleton` shimmer |
| Busy ring | 0.8 s linear spin | the shared `.spinner` (`otto-spin`) |

Rules:

- Keep UI feedback at 200 ms or less. No bounces, springs or parallax.
- Continuous animation is **only for live state**: something is working right
  now, or loading. A finished or failed item stops animating.
- Don't animate data changes: rows arriving, numbers ticking. Update them in
  place. **Data bars and meters** (usage bars, progress fills, the done-contract
  ring) show their value — no `transition` on `width`, `inline-size`,
  `flex-basis` or `stroke-dasharray` (`data-bar-transition` ratchet). Chrome
  that changes size because the user dragged it, or a bar morphing between
  its own states (the floating command bar), is marked `ui-guards: allow`.
- **Reduced motion.** `app.css` has one global
  `@media (prefers-reduced-motion: reduce)` override that collapses every
  animation and transition to an instant change. A component that needs a
  different reduced-motion treatment (hide a pulse, keep a static state) still
  ships its own override:

  ```css
  @media (prefers-reduced-motion: reduce) {
    .dot.working { animation: none; }
  }
  ```

  Examples in the tree: `shell/Drawer.svelte`,
  `run-with-otto/RunStageRail.svelte`.
- **Shared primitives.** `app.css` owns `@keyframes otto-spin` and
  `otto-pulse` plus a `.spinner` utility (size via `--spinner-size`); a
  component never spins its own ring (`local-spinner` ratchet). Use them
  instead of a new `@keyframes`. `otto-pulse` ends on its fully-lit frame, so
  the global reduced-motion override leaves a live dot "on". Under reduced
  motion `.spinner` becomes a static dotted ring, so "busy" is still shown
  without rotation.
- **Entrances and timing.** `--dur-fast` (130 ms) is the hover/press duration,
  `--dur-enter` (160 ms) the entrance of a floating layer, `--ease-out` the
  easing. `app.css` ships `otto-fade-in` (opacity) and `otto-pop-in` (opacity +
  ≤ 8 px rise) on those tokens; use them rather than a private keyframe.
  `scripts/ui-guards.mjs` (`private-keyframes`) ratchets new `@keyframes` in
  module styles; keep a local one only when the motion is genuinely different
  (a blinking cursor, a sweep bar).

---

## 9. Icons

`ui/src/lib/components/Icon.svelte` is the only icon set: inline SVG paths on a
16-unit viewBox, 1.4 stroke, `currentColor`, no dependencies.

```svelte
<script lang="ts">
  // No path alias in this app: import relatively (from ui/src/modules/<m>/).
  import Icon, { asIcon } from '../../lib/components/Icon.svelte';
</script>

<Icon name="trash" size={14} />
<!-- from untyped data (plugin manifest, API payload): narrow it, don't cast -->
<Icon name={asIcon(plugin.icon, 'box')} />
```

- **`name` is typed** (`IconName = keyof typeof paths`). `svelte-check` rejects
  an unknown name. Before the type existed, 10 misspelled names silently
  rendered as a dot.
- Use `asIcon(value, fallback)` / `isIconName()` for strings from data. Don't
  write `as IconName`.
- **Sizes:**

  | px | Where |
  |---|---|
  | 12 | inside `.btn.small` and chips |
  | 13–14 | inside `.btn`, `.icon-btn` and toolbars (14 is the toolbar minimum) |
  | 16 | nav rows, the `PageHeader` icon, standalone |
  | 24–26 | empty-state tiles only |

  Don't use other sizes.
- **Adding an icon:** add a path to `paths` in `Icon.svelte`'s module script. It
  should be drawn for 16×16 as strokes in the same visual weight, with a
  one-line comment if the metaphor isn't obvious. Check it at 12 px and at
  16 px.
- **One icon per module.** Sidebar icons must be unique across
  `SIDEBAR_MODULES`, because the collapsed Rail shows icons only. The registry
  uses `book` (Vault), `compass` (Browser), `server` (MCP) and `calendar`
  (Scheduled Tasks) to remove the old `globe`/`plug`/`clock` collisions. Don't
  reuse a module's icon for a different module.
- **One icon per concept.** Don't give two adjacent buttons the same icon (as
  "Ask AI" and "Ask in English" once did).
- **No glyph characters as icons** in new code: `✕ × ⋯ → ▸`. Use `x`,
  `chevronRight` and so on; for "more" use `more` (`PageHeader`'s overflow
  button and Home's widget menu do). Existing exceptions: the Modal and toast
  close `✕`.
- **Provider marks** (Claude, Codex, Antigravity, custom providers) come from
  `ProviderIcon`, never from `Icon`. Every provider gets a mark; custom ones
  get a deterministic monogram tile.
- Icons inherit colour. Dim icons use `--text-dim`, the active nav icon uses
  `--accent`, and destructive icons take `--danger` only inside a destructive
  control.
