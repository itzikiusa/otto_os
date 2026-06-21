# RTL, Responsive Shell, Theming & Per-Device Sessions

Otto's UI is a single Svelte 5 app served two ways — inside the Tauri desktop
window and over the network to a phone/tablet browser (see
[`./connections-ssh-sftp.md`](./connections-ssh-sftp.md) and the remote-access
runbook at `../remote-access-runbook.md`). One codebase therefore has to cover a
**desktop 3-pane shell**, a **tablet 2-pane shell**, and a **phone single-pane
shell with drawers + bottom nav**, in **light or dark**, in **three themes**,
**left-to-right or right-to-left**, with an opt-in **per-device session filter**.
This document is the authoritative reference for all four of those concerns.

The guiding rule throughout the code: **the desktop layout is never touched.**
Every mobile/tablet affordance is gated behind `viewport.isMobile`/`isPhone`/
`isTablet`, every RTL/theme change is an attribute flip on `<html>`, and CSS uses
**logical properties** (`inset-inline-*`, `margin-inline-*`, `border-inline-*`)
so the layout mirrors for RTL with zero per-component RTL code.

---

## 1. Overview & where it lives

| Concern | Owner | Key file(s) |
|---|---|---|
| UI state (theme, scheme, direction, accent, RTL-bidi, term font, isolation, zoom) | `ui` store (`UiStore`) | `ui/src/lib/stores/ui.svelte.ts` |
| Responsive breakpoint (`phone`/`tablet`/`desktop`) | `viewport` store | `ui/src/lib/stores/viewport.svelte.ts` |
| Shell layout switch (3-pane vs mobile) | App shell | `ui/src/shell/App.svelte` |
| Off-canvas drawers (Navigator / RightPanel on mobile) | `Drawer` | `ui/src/shell/Drawer.svelte` |
| Phone bottom nav + overflow sheet | `BottomNav` | `ui/src/shell/BottomNav.svelte` |
| Phone quick-action bar (⌘K/⌘T/⌘W/⌘F/⌘⇧B) | `MobileActionBar` | `ui/src/shell/MobileActionBar.svelte` |
| Theme / scheme / direction / accent / RTL toggle / isolation toggle UI | Appearance settings | `ui/src/modules/settings/Appearance.svelte` |
| Theme CSS variables (per theme × scheme) | tokens | `ui/src/lib/tokens.css` |
| Breakpoint tokens + base layout | global CSS | `ui/src/app.css` |
| Terminal renderer + RTL bidi reflow | `Terminal` | `ui/src/lib/components/Terminal.svelte` |
| Cousine Hebrew font registration | app entry | `ui/src/main.ts` (`@fontsource/cousine`) |
| Per-device id + session filter | `ui.clientId()` / `ws.refreshSessions` | `ui/src/lib/stores/ui.svelte.ts`, `ui/src/lib/stores/workspace.svelte.ts` |
| E2E (responsive + light/dark + RTL + per-device) | Playwright suite | `ui/e2e/*.spec.ts`, `ui/playwright.config.ts` |

Everything user-visible is reachable from **Settings → Appearance**
(`#/settings/appearance`). All preferences are **client-side and persist
per-device** in `localStorage`; nothing here is stored on the daemon or scoped to
the user account.

---

## 2. RTL support

### 2.1 How direction is toggled

**Settings → Appearance → Direction** is a two-button segmented control:
`Left-to-right` / `Right-to-left` (`Appearance.svelte`, the `directions` array).
It writes `ui.direction` (`'ltr' | 'rtl'`), persisted to `localStorage` key
**`otto_direction`**, and re-applies the theme.

`ui.applyTheme()` sets the document direction directly on the root element:

```ts
// ui/src/lib/stores/ui.svelte.ts — applyTheme()
const el = document.documentElement;
el.setAttribute('data-theme', this.theme);
el.setAttribute('data-scheme', resolved);
el.dir = this.direction;        // ← RTL support: flips <html dir>
```

`applyTheme()` is invoked once at boot from `App.svelte` (`ui.applyTheme()` at
module top), and again on every theme/scheme/direction/accent change. The hint
under the control reads: *"Right-to-left mirrors the layout for RTL languages
(Hebrew, Arabic)."*

### 2.2 How the layout mirrors

There is **no per-component RTL code**. The layout mirrors because the entire
shell is written with **CSS logical properties** instead of physical ones:
`inset-inline-start/-end`, `margin-inline-start/-end`, `border-inline-start/-end`,
`text-align: start`, `padding-inline-end`, etc. When `<html dir>` flips to `rtl`,
the browser remaps "start"→right and "end"→left automatically.

Examples in the shell that mirror for free:
- `App.svelte`: `.bell-anchor { inset-inline-end: 10px }` (notification bell),
  `.content.bell-gutter { padding-inline-end: 42px }`, `.msidebar
  { border-inline-end: … }`, `.imp-real { margin-inline-start: 4px }`.
- `Drawer.svelte`: `.drawer-close { inset-inline-end: 8px }`.
- `BottomNav.svelte`: `.more-sheet { inset-inline-start/-end: 0 }`,
  `.bn-badge { inset-inline-end: -8px }`.
- `Appearance.svelte`: `.theme-card { text-align: start }`,
  `.exp-tag { margin-inline-start: 6px }`.

This same logical-property discipline runs across the module pages (Git, Database,
Brokers, Product, API, Swarm, Vault, …) — `grep`-able as the long list of files
using `inset-inline`/`margin-inline`. The net effect: toggling RTL mirrors the
chrome **and** the content panes with no extra work.

### 2.3 Terminal RTL — the hard problem (deep-dive)

A terminal is the one place where `<html dir="rtl">` is **not** enough. xterm.js
renders text to a grid, and with the default **WebGL renderer** it paints cells in
raw *logical* order with no bidi awareness — so Hebrew would come out reversed and
left-aligned regardless of document direction. Otto therefore ships a **separate,
opt-in experimental toggle** specifically for terminal text:

**Settings → Appearance → "Right-to-left text in the terminal"** (labelled
*Experimental*), bound to `ui.rtlBidi`, persisted to `localStorage` key
**`otto_term_rtl_bidi`**. Its warning text spells out the trade-off verbatim:

> ⚠ Lays out Hebrew right-to-left with English embedded left-to-right, using the
> browser's bidi engine (switches the terminal off the GPU renderer). Because text
> is reflowed for reading, the monospace grid no longer lines up exactly — great
> for chat-style output, imperfect for TUI tables or box art. Toggling reloads
> open terminals.

**Mechanism (`Terminal.svelte`):**

1. **Renderer selection.** The terminal build effect reads `ui.rtlBidi` as a
   tracked dependency, so flipping it **re-runs the effect and rebuilds the
   terminal**. WebGL is used only when neither RTL nor phone applies:

   ```ts
   const rtl = ui.rtlBidi;
   …
   const useWebgl = !rtl && !viewport.isPhone;   // RTL forces the DOM renderer
   if (useWebgl) { /* loadAddon(new WebglAddon()) … */ }
   ```

   The DOM renderer emits one `<span>` per run inside `.xterm-rows > div`, which
   the browser's bidi engine can actually reorder; WebGL cannot.

2. **Bidi reflow via CSS.** With the DOM renderer active, the host div gets the
   `.rtl-bidi` class (`class:rtl-bidi={ui.rtlBidi}`), and these global rules turn
   each row into a single bidi paragraph:

   ```css
   .term-host.rtl-bidi :global(.xterm-rows > div)       { unicode-bidi: plaintext; }
   .term-host.rtl-bidi :global(.xterm-rows > div span)  {
     display: inline !important;     /* xterm uses inline-block (atomic to bidi) */
     unicode-bidi: normal !important;
     width: auto !important;
     letter-spacing: 0 !important;
   }
   ```

   `unicode-bidi: plaintext` gives each line a **per-line base direction** — RTL
   when the line starts with Hebrew, LTR otherwise — matching native bidi: Hebrew
   flows right-to-left with embedded English left-to-right. Forcing the spans back
   to `display: inline` is the crux: xterm normally lays each run as a fixed-width
   `inline-block`, which the bidi algorithm treats as one atomic box (so words
   would *not* reorder); inline flow makes the whole row reorderable.

3. **Cost.** Because runs are reflowed for reading, the fixed monospace column
   grid no longer aligns exactly — fine for prose/agent chat output, imperfect for
   TUI tables and box-drawing art. This is the documented, accepted trade-off.

### 2.4 Hebrew glyphs — the Cousine font

Even with bidi off, Hebrew needs glyphs in a monospace face. Otto bundles
**Cousine** (a Croscore monospace with full Hebrew coverage) via
`@fontsource/cousine` (`^5.2.7`), imported at app entry:

```ts
// ui/src/main.ts
import '@fontsource/cousine/latin.css';
import '@fontsource/cousine/hebrew.css';
```

The **Terminal font** picker (Appearance → Terminal font: `System` / `Cousine` /
`Menlo`) maps to `TERM_FONT_STACKS` in `ui.svelte.ts`. In every stack Cousine sits
**after** the Latin fonts (e.g. `'SF Mono', …, 'Cousine', monospace`), so the
browser only falls through to it for codepoints the primary font lacks (Hebrew) —
Latin/code is unchanged. The picker hint: *"Hebrew & other right-to-left text
renders crisply via the bundled Cousine font in every option."* The terminal is
created with `rescaleOverlappingGlyphs: true` so fallback Hebrew glyphs stay inside
their grid cell. Changing the font family applies to open terminals live (no
rebuild); changing the *direction* toggle reloads them.

### 2.5 Scripts & gotchas

- **Languages:** the UI explicitly names **Hebrew and Arabic** for layout RTL; the
  terminal bidi path is described/tested against **Hebrew + embedded English**.
- **Document RTL (`otto_direction`)** and **terminal bidi (`otto_term_rtl_bidi`)**
  are **independent toggles.** You can mirror the layout without touching the
  terminal, or reflow the terminal while keeping an LTR layout.
- Enabling terminal RTL **disables the GPU renderer** for that terminal (DOM
  renderer only) — see Troubleshooting for the performance/alignment implications.
- It is **per-device** (localStorage), so RTL on your phone doesn't change your
  desktop, and vice-versa.

---

## 3. Responsive / mobile shell

### 3.1 Breakpoints

The `viewport` store (`viewport.svelte.ts`) is a `matchMedia`-backed reactive
`mode` with three buckets, kept in sync with `app.css` tokens
(`--bp-phone-max: 640px`, `--bp-tablet-max: 1024px`):

| Mode | Width | Probe |
|---|---|---|
| `phone` | ≤ **640px** | `(max-width: 640px)` |
| `tablet` | **641–1024px** | `(max-width: 1024px)` & not phone |
| `desktop` | ≥ **1025px** | neither matches |

It exposes `isPhone`, `isTablet`, `isDesktop`, and `isMobile` (= phone OR tablet).
It is SSR-safe (defaults to `desktop` when `window`/`matchMedia` is absent) and
recomputes on `change` events from both media queries — so **rotating an iPad or
phone re-buckets live** (an iPhone in landscape is 932px wide → treated as
*tablet*, not phone; this matters for terminal renderer selection, see §3.5).

### 3.2 The three shells (`App.svelte`)

`App.svelte` branches on `viewport.isDesktop`. The center column content is
factored into a `centerContent()` snippet so **desktop and mobile render
byte-for-byte identical content** — only the surrounding chrome differs.

- **Desktop (≥1025px)** — the original, unchanged 3-pane shell:
  `.sidebar` (Navigator when expanded, else the icon `Rail`) · `.center` (banners,
  notification bell, TabBar on Agents, module router) · `RightPanel`
  (Git/Files/Notes/Activity/Info/Browser/API for the focused agent session) ·
  a `StatusBar` row.
- **Tablet (641–1024px)** — single content pane with a **persistent narrow
  Navigator column** (`.msidebar`, 220px) on the left; the RightPanel moves into a
  **right drawer**. Compact top bar (`.mtopbar`), no bottom nav.
- **Phone (≤640px)** — single content pane; the Navigator moves **off-canvas into
  a left drawer** (reusing `ui.railExpanded` as its open-state, opened by the
  hamburger in `.mtopbar`); the RightPanel into a **right drawer**
  (`ui.rightOpen`); a phone **quick-action bar** (`MobileActionBar`) sits below the
  top bar; a **`BottomNav`** sits at the bottom.

The top bar also hosts `NavButtons` (Back/Forward, placed left of the title so a
thumb can reach them) and the current module/session title. A
`--mobile-topbar-h: 44px` / `--mobile-bottomnav-h: 56px` token pair keeps the
touch chrome sized consistently. The shell uses `height: 100%` (not `100vh`)
deliberately — in the transparent overlay-titlebar WKWebView, `100vh` resolves to
full *screen* height and clips the bottom row.

### 3.3 Collapsible, independently-scrollable sections

The headline mobile-layout fix is that **every content pane keeps a real height
and owns its own scroll** — no pane collapses to ~0, and no content runs off the
right edge. Two mechanisms:

- **The flex height chain.** `.mbody`/`.mcenter`/`.center` all carry
  `min-height: 0` so nested panes (terminal output, DB results grid, diff viewers)
  can shrink below their intrinsic content height instead of collapsing. The
  comment in `App.svelte` documents exactly this.
- **Per-module collapsible accordions** on mobile/tablet widths. Module pages turn
  their side panels into collapsible sections that the user taps to expand/collapse,
  while the primary content area (diff, results, graph) becomes its **own
  independent scroll container** (`overflow-y: auto`). For example, in Git
  (`ui/src/modules/git/*`): the graph view is a collapsible accordion
  (`.graph-panel.mob-collapsed`), tapping a commit auto-collapses the commit list
  and opens the diff, and the Changes view's file list collapses
  (`.changes-side.mob-files-collapsed`). The diff area is its own scroll container
  (`overflow-y: auto`) so long diffs are always reachable. This pattern repeats
  across Database, Brokers, Product, Connections, Swarm, Vault, etc.

The **document itself never scrolls horizontally** — panes use
`overflow: hidden` / `overflow-x: auto` internally, which is why the E2E suite
both checks document-level overflow *and* drills into specific panes (§7).

### 3.4 Phone navigation affordances

- **Bottom nav (`BottomNav.svelte`)** — the first `PRIMARY_COUNT = 4` modules the
  user can view (filtered through `auth.can(feature, 'view')`, mirroring
  `Rail.svelte`) as large tap targets, plus a **"More"** button that opens a
  bottom-sheet grid of the overflow modules + Settings. The Agents tab shows a
  working-count badge. Respects `env(safe-area-inset-bottom)` for the iOS home
  indicator.
- **Quick-action bar (`MobileActionBar.svelte`)** — exposes the desktop chords that
  a touch user can't produce: **Palette (⌘K), New (⌘T), Close (⌘W), Find (⌘F),
  Broadcast (⌘⇧B)**. Each button calls the *exact same* function the keyboard map
  calls (passed from `App.svelte`), so behaviour is identical; Broadcast only shows
  when sessions exist. Tap targets are `min-width: 44px` (WCAG 2.5.5).
- **Drawers (`Drawer.svelte`)** — reusable off-canvas slide-over for the Navigator
  (left) and RightPanel (right). Backdrop tap or **Esc** dismisses; an explicit ✕
  close button is pinned to the panel corner (a thin backdrop sliver is hard to tap
  on a phone). Width defaults to `min(86vw, 320px)`; animations honour
  `prefers-reduced-motion`. Desktop never mounts a drawer.

### 3.5 The touch terminal (`Terminal.svelte`)

The phone terminal had a class of "big black void, can't see or type" bugs that
are now fixed and regression-guarded:

- **DOM renderer on phone.** On phone the terminal uses xterm's **DOM renderer, not
  WebGL** (`useWebgl = !rtl && !viewport.isPhone`). Mobile WKWebView/Safari often
  fails to create a GL context or silently loses it after first paint, leaving a
  permanently black canvas; the DOM renderer has no GPU dependency, so output is
  always visible. (Desktop keeps WebGL, with `onContextLoss → dispose` so it falls
  back to the DOM renderer if a laptop GPU resets.)
- **Readability font floor.** `PHONE_MIN_FONT = 15` — phone terminals never render
  below 15px even if `ui.termFontSize` is lower, so the grid isn't cramped on a
  high-DPI handset.
- **Tap-to-focus + key accessory bar.** Tapping the terminal focuses the xterm
  textarea (raising the soft keyboard via `onTouchPointerDown → term.focus()`). A
  floating ⌨ button toggles a **key accessory bar** (`.keys-bar`) exposing keys a
  soft keyboard can't produce — Esc / Ctrl / arrows / Ctrl-C — each ≥44px tall,
  sharing the same WS input path as a physical keyboard.
- The desktop **terminal toolbar** (font zoom + copy-on-select) renders only when
  `!viewport.isPhone && ui.termToolbar`.

### 3.6 Using it on a phone / iPad

- **Phone:** tap the hamburger (top-left) for the Navigator drawer; use the bottom
  nav to switch modules ("More" for the rest + Settings); use the quick-action bar
  for palette/new/close/find/broadcast; tap a terminal to type, ⌨ for special keys.
- **iPad:** the Navigator is a persistent left column; the right Activity panel is a
  drawer toggled from the top bar's panel button. Landscape (>1024px) gets the full
  desktop 3-pane shell with an inline collapsible sidebar.

App-level zoom: in **Tauri** it uses the native WKWebView page-zoom
(`applyNativeZoom`) so the terminal stays crisp; in a **browser** Otto applies *no*
CSS `zoom` to the shell (CSS zoom stretches the WebGL canvas and skews
hit-testing) — users scale with the browser's own ⌘+/− instead.

---

## 4. Light / dark theming

### 4.1 Toggle (Settings → Appearance)

Three independent controls:

- **Theme** — three cards: **Native** (macOS vibrancy, system accent),
  **Pro Dark** (always-dark, violet accent), **Warm** (paper tones, green accent).
  `ui.theme: 'native' | 'pro-dark' | 'warm'`.
- **Scheme** — segmented `Auto` / `Light` / `Dark`. `ui.scheme: 'auto' | 'light'
  | 'dark'`; *Auto follows the system light/dark preference.*
- **Accent color** — a color picker overriding `--accent`; Reset clears it.

There are also palette commands (`⌘K`): *Theme: Native / Pro Dark / Warm*.

### 4.2 How themes are applied — CSS variables on `<html>`

`ui.applyTheme()` resolves the scheme (for `auto`, it reads
`window.matchMedia('(prefers-color-scheme: dark)')` and subscribes to its `change`
so the app re-themes when the OS flips), then sets attributes on
`document.documentElement`:

```ts
el.setAttribute('data-theme', this.theme);     // native | pro-dark | warm
el.setAttribute('data-scheme', resolved);      // light | dark
el.dir = this.direction;                        // (RTL, §2)
if (this.accent) el.style.setProperty('--accent', this.accent);
else el.style.removeProperty('--accent');
```

`ui/src/lib/tokens.css` defines the **CSS custom properties** keyed on those two
attributes — one block per `theme × scheme`, e.g.:

```css
html[data-theme='native'][data-scheme='dark']  { --bg:#1e1e23; --bg-sidebar:#232328; --surface:#2a2a30; … }
html[data-theme='native'][data-scheme='light'] { --bg:#f5f5f7; --surface:#ffffff; … }
html[data-theme='pro-dark'][data-scheme='light'],
html[data-theme='pro-dark'][data-scheme='dark'] { --bg:#16161c; --accent:#6c5ce7; … }  /* always-dark */
html[data-theme='warm'][data-scheme='light']    { --bg:#faf9f7; --accent:#0f9d58; … }
html[data-theme='warm'][data-scheme='dark']      { --bg:#211f1b; --accent:#2bb673; … }
```

Every component reads tokens (`var(--bg)`, `var(--surface)`, `var(--text)`,
`var(--accent)`, `var(--border)`, …) and `color-mix(in srgb, …)` derivations —
so a single attribute flip restyles the whole app instantly with no re-render.
`pro-dark` is intentionally identical in both light and dark (always-dark).
The terminal carries its own xterm theme derived from `(ui.theme, scheme)` via
`terminalTheme(...)`. Some syntax-highlight rules are scheme-specific
(`html[data-scheme='dark'] .hljs-*`).

### 4.3 Persistence

All four persist per-device in `localStorage`: `otto_theme`, `otto_scheme`,
`otto_direction`, `otto_accent`. The Appearance header states it plainly:
*"Themes apply instantly and persist per device."* Boot applies them before first
paint (`ui.applyTheme()` at the top of `App.svelte`).

---

## 5. Per-device session view

### 5.1 What it does

**Settings → Appearance → "Sessions on this device" → "Isolate sessions to this
device"** (`ui.sessionIsolation`, key **`otto_session_isolation`**, default
**off**). When ON, the sessions list / tabs / Navigator show **only the sessions
this device started**. The hint: *"Only show sessions started on this device.
Other devices' sessions stay hidden here (they still run on the daemon)."*

### 5.2 How a device is identified

`ui.clientId()` returns a **stable per-device UUID** generated once with
`crypto.randomUUID()` and persisted in `localStorage` key **`otto_client_id`**
(falling back to `dev-<ts>-<rand>` if `crypto` is unavailable). Because it lives in
`localStorage`, it is unique per browser/profile/device — which is exactly the
granularity of "this device".

### 5.3 How the filter works (client-side only)

This is a **pure client-side view filter**; the daemon is not involved in
enforcement:

- **On create** (`ws.createSession`) the device id is stamped into the session's
  free-form meta, preserving any caller meta:
  ```ts
  meta: { ...(req.meta ?? {}), client_id: clientId() }
  ```
- **On list** (`ws.refreshSessions`) the filter runs *after* fetching all sessions:
  ```ts
  if (ui.sessionIsolation) {
    const me = clientId();
    kept = kept.filter((s) => (s.meta as { client_id?: string } | null)?.client_id === me);
  }
  ```
- Because tabs / Navigator / agents list all derive from `ws.sessions`, the filter
  applies everywhere consistently. `ui.setSessionIsolation()` calls
  `ws.refreshSessions()` so flipping the toggle **applies live** without a reload.

The daemon stores session `meta` as an opaque `serde_json::Value`
(`crates/otto-core/src/domain.rs` — `pub meta: Value`) and round-trips it verbatim;
**no Rust crate reads `client_id` from session meta**, so the daemon neither
enforces nor is aware of device isolation.

### 5.4 Interaction with multi-user / RBAC

Device isolation is **orthogonal to** multi-user RBAC and per-session ownership
(see [`./MULTI-USER-RBAC.md`](../MULTI-USER-RBAC.md), if present, and the
multi-user docs):

- **RBAC / ownership** is **server-enforced** — who may *access* a session, scoped
  to the user account, across all their devices. The daemon's `GET
  /workspaces/{id}/sessions` already filters by ownership: a non-admin caller is
  returned only their own sessions, while workspace Admin / root see all
  (`crates/otto-sessions/src/http.rs`, `list_sessions`; `session_owner_or_admin`
  in `crates/otto-core/src/auth.rs`). Per-session ownership migrations underpin
  this (the multi-user work).
- **Device isolation** is a **client-side convenience view** layered *on top* of
  that server-filtered list — which of the sessions you already have access to you
  want *shown* on this particular device. Hidden sessions still run on the daemon
  and remain visible on other devices or with the toggle off.

So a session you cannot see due to RBAC is genuinely inaccessible; a session hidden
by device isolation is merely filtered out of this device's view.

---

## 6. How it's verified (E2E suite)

Playwright suite under `ui/e2e/`, configured by `ui/playwright.config.ts`. It
spins an **isolated throwaway daemon** (temp data dir + temp port, slot-keyed via
`OTTO_E2E_SLOT`/`OTTO_E2E_PORT`/`OTTO_E2E_PW_PORT`) so tests never touch real
sessions/DBs, serves the live UI via Vite, and runs across these **device
profiles** (`projects`):

| Project | Device |
|---|---|
| `iphone-portrait` | iPhone 14 Pro Max |
| `iphone-landscape` | iPhone 14 Pro Max (landscape → ~932px → tablet bucket) |
| `ipad-portrait` | iPad Pro 11 |
| `ipad-landscape` | iPad Pro 11 (landscape → >1024px → desktop bucket) |
| `iphone-se` | iPhone SE (smallest) |
| `desktop-browser` | Desktop Chrome 1280×800 (runs only `desktop-*.spec.ts`) |

Run with `cd ui && npm run test:e2e`.

**What's asserted (`helpers.ts` + specs):**

- **`pages.spec.ts`** — for *every* top-level page (`agents, api, brokers,
  connections, database, git, help, insights, plugins, product, settings,
  skills-eval, swarm, usage, vault, workflows`) on *every* device profile:
  (1) shell renders + content pane (`.center`) has real height
  (`expectContentHasHeight`, catches collapsed panes / blank terminal); (2) **no
  horizontal overflow** (`expectNoHorizontalOverflow`, ≤2px slack — catches
  clipped/off-screen content); (3) **no `critical` axe-core a11y violations**.
- **`theme.spec.ts`** — forces **light** scheme (`otto_scheme=light`) on
  iphone-portrait + ipad-landscape and re-runs the overflow + a11y checks (catches
  light-only contrast/visibility regressions; baseline is dark).
- **`rtl.spec.ts`** — forces **`otto_direction=rtl`** on iphone-portrait +
  ipad-landscape, asserts `document.documentElement.dir === 'rtl'` actually applied,
  then re-runs overflow + a11y on every page (proves the logical-property mirroring
  holds).
- **`terminal-mobile.spec.ts`** / **`deep.spec.ts`** — seed a real shell session
  with output: terminal host has real height (>120px); on a **live ≤640px**
  viewport the terminal uses the **DOM renderer (no WebGL canvas, `.xterm-rows`
  present)** with font ≥15px; the **key accessory bar** mounts with Esc/Ctrl/↑
  buttons ≥44px tall; tap-to-focus is asserted on pointer profiles (soft-keyboard
  focus is verified on a real device, not under WebKit touch emulation).
- **`desktop-shell.spec.ts`** — desktop-browser only: with `otto_zoom=2` the shell
  applies **no inline CSS zoom**, no overflow, and the sidebar collapse toggle
  changes width without hit-test skew.
- **`git-mobile.spec.ts`** (and `db-/brokers-/product-/swarm-/vault-/
  connections-mobile`) — real flows: drill into a commit → multi-file diff that is
  its **own independent scroll container** (`overflow-y: auto`, actually scrolls);
  sections collapse as an accordion (`.graph-panel.mob-collapsed`,
  `.changes-side.mob-files-collapsed`); content fits the viewport.
- **`session-isolation.spec.ts`** — seeds two sessions via the API with explicit
  `meta.client_id` (one = this device, one = another device), pins
  `otto_client_id`: isolation **OFF** shows both; **ON** shows only this device's;
  flipping the **Settings toggle live** makes the other-device session vanish with
  no reload. Confirms in-test that filtering is **client-side**.

The accessibility scan (`expectAccessible`) fails the build on any `critical` axe
violation across the WCAG 2.0/2.1 A/AA tag set, on every page, on every profile.

---

## 7. Capabilities & limitations

**Capabilities**
- Full layout mirroring (LTR↔RTL) via a single `<html dir>` flip, no per-component
  RTL code.
- Optional bidi-correct terminal output for Hebrew + embedded English.
- Bundled Cousine Hebrew monospace; live terminal-font switching.
- Three responsive shells (desktop 3-pane, tablet 2-pane, phone single-pane +
  drawers + bottom nav + action bar), live-rebucketing on rotation.
- Robust touch terminal (DOM renderer, readable font floor, key accessory bar).
- Three themes × auto/light/dark × custom accent, instant + per-device-persistent.
- Opt-in per-device session view that never affects what runs on the daemon.
- Verified by an extensive Playwright matrix incl. light, RTL, and a11y gates.

**Limitations**
- **Terminal RTL is experimental.** The DOM-renderer bidi reflow breaks exact
  monospace column alignment — good for prose/chat output, imperfect for TUI tables
  and box-drawing. It also disables the GPU renderer for that terminal.
- RTL/theme/direction/isolation are **per-device only** — not synced to the
  account or across devices.
- The two RTL toggles are independent on purpose; **layout RTL does not reflow the
  terminal**, and the terminal bidi toggle does not mirror the layout.
- Per-device isolation is a **view filter, not an access control** — it hides, it
  does not protect (RBAC does that).
- Soft-keyboard focus on a real phone is verified manually, not in CI (WebKit touch
  emulation can't synthesize the iOS focus path).
- `iphone-landscape` (~932px) is treated as **tablet** (desktop chrome + WebGL), so
  phone-only behaviour does not apply there — by design.

---

## 8. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| **Terminal is a black void on a phone** | WebGL context failed/lost. On phone Otto already forces the DOM renderer (`useWebgl = !rtl && !viewport.isPhone`); if you see this on desktop, a GPU reset should auto-fall-back via `onContextLoss → dispose`. Reload the session to rebuild the terminal. |
| **Hebrew in the terminal is reversed / left-aligned** | Terminal bidi is off. Enable **Settings → Appearance → "Right-to-left text in the terminal"** (`otto_term_rtl_bidi`); this forces the DOM renderer + per-line bidi. Toggling reloads open terminals. |
| **Terminal RTL works but TUI tables/box art are misaligned** | Expected: the bidi reflow drops exact monospace columns. Turn the toggle off for TUI-heavy sessions; it's intended for prose/agent chat. |
| **Hebrew glyphs look wrong / non-mono** | Pick **Cousine** (or any option — Cousine is the Hebrew fallback in all stacks) in Terminal font; ensure `@fontsource/cousine` loaded (it's imported in `main.ts`). |
| **Layout didn't mirror after switching to RTL** | The flip is `el.dir` on `<html>` via `applyTheme()`. Confirm `otto_direction=rtl` in localStorage and that the page booted (it's applied at boot + on every change). A component using *physical* CSS (`left`/`right`) instead of logical properties won't mirror — that's a bug to fix in that component. |
| **Content runs off the right edge / clipped (horizontal overflow)** | The E2E `expectNoHorizontalOverflow` guards this. If reproduced, the offending pane is using a fixed width or not wrapping — panes should use `overflow-x: auto`/`hidden` and the flex chain needs `min-width:0`. |
| **A pane collapsed to ~0 height (blank terminal / empty results)** | The flex height chain lost `min-height: 0` somewhere between `.mbody`/`.mcenter`/`.center` and the pane. Restore `min-height: 0` on the flex ancestors. |
| **Theme/accent didn't change** | `data-theme`/`data-scheme`/`--accent` live on `<html>`; confirm `applyTheme()` ran. `pro-dark` is always-dark by design — switching scheme won't lighten it. |
| **Auto scheme didn't follow the OS** | Auto relies on `matchMedia('(prefers-color-scheme: dark)')`; the listener re-themes on OS change. If stuck, set Light/Dark explicitly. |
| **A session from another device is missing** | "Isolate sessions to this device" is ON (`otto_session_isolation=1`). Turn it off in Appearance, or use the device that started the session. It's still running on the daemon. |
| **Phone can't open the Navigator / Activity panel** | Navigator = hamburger (top-left) → left drawer; Activity = the panel button in the top bar (agent sessions only) → right drawer. Tap the backdrop or ✕ or Esc to dismiss. |
| **Can't run ⌘-key actions on a phone** | Use the **MobileActionBar** (Palette/New/Close/Find/Broadcast) — same functions as the chords. |

---

## 9. Related docs

- [`./mobile-usage.md`](./mobile-usage.md) — the task-oriented "use Otto on a
  phone/iPad" guide that builds on this responsive-shell reference.
- [`./connections-ssh-sftp.md`](./connections-ssh-sftp.md) — SSH/SFTP/DB
  connection terminals that also run in the responsive Agents view.
- [`./agent-swarm.md`](./agent-swarm.md) — multi-agent sessions surfaced in the
  same responsive shell.
- [`./daemon-http-api.md`](./daemon-http-api.md) — the HTTP/WS API the UI talks to
  (session create carries the per-device `meta.client_id`).
- [`../MULTI-USER-RBAC.md`](../MULTI-USER-RBAC.md) — server-enforced access control
  that per-device isolation is layered on top of (a view filter, not a gate).
- [`../remote-access-runbook.md`](../remote-access-runbook.md) — exposing the UI to
  a phone/tablet over the network (where the responsive shell matters most).
