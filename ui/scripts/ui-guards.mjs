#!/usr/bin/env node
// Static UI guards, run first by `npm run check` (so CI enforces them too).
// Pure Node, no dependencies, well under a second. Two kinds of rule:
//
// HARD rules — any occurrence fails:
//
// 1. No native dialogs. window.confirm()/prompt()/alert() silently return
//    false/null/undefined inside the Tauri WKWebView, so a "Delete?" guarded
//    by confirm() is a no-op in the desktop app. Use `confirmer.ask()` /
//    `confirmer.promptText()` / `confirmer.choose()` (lib/confirm.svelte.ts)
//    or a toast instead.
//
// 2. No undefined CSS custom properties. Every `var(--x)` must name a
//    property that is DEFINED somewhere under src/ — a global token in
//    lib/tokens.css / app.css, a component-local `--x: …` declaration, a
//    `style:--x` / `--x=` prop, or a JS `setProperty('--x', …)`. This applies
//    even when the var() carries a fallback: a fallback on a token that never
//    exists is what always renders, and those fallbacks were hard-coded for
//    one scheme (dark hex in light mode and vice versa). If a component wants
//    an optional knob, declare its default on the component's root element
//    (`--knob: 8px;`) — that counts as a definition. Properties owned by
//    third-party libraries are listed in EXTERNAL_PREFIXES.
//
// 3. Icon-only buttons are labelled. A <button> whose only content is
//    <Icon …/> needs BOTH aria-label (screen readers) and title (hover
//    tooltip) — name the action, e.g. "Delete credential", not "Delete".
//    (Ratcheted until the tree hit zero on 2026-10-03, then promoted.)
//
// RATCHETED rules — the tree predates them, so today's debt is recorded per
// file in scripts/ui-guards-baseline.json and only an INCREASE fails (a file
// gaining an occurrence, or a new file having any). They scan the <style>
// blocks of .svelte files and the .css files under src/ (not lib/tokens.css,
// which is where literal values belong):
//
//   color-literal     #hex / rgb() / hsl() / named white|black in a component
//                     style → a token (lib/tokens.css). rgba(0,0,0,x) —
//                     shadows and scrims — is exempt.
//   font-size-small   a px font-size below 11px → the --fs-* scale (11px is
//                     the floor for anything a user reads).
//   z-index-literal   a z-index outside -1…10 (in-pane stacking) that is not
//                     var(--z-*) → a layer token (tokens.css, foundations §6).
//   physical-prop     margin/padding/border-left|right, left:/right:,
//                     text-align: left|right → logical properties
//                     (margin-inline-start, inset-inline-end, text-align:
//                     start…) so RTL mirrors.
//   global-class      a component <style> selector on a global app.css class
//                     (.btn .chip .row .card .input .icon-btn) outside
//                     :global() → rename the local class (prefix it), or style
//                     the shared one via :global() on purpose.
//   local-button-class  a component <style> that (re)defines the shared button
//                     system: a `.tb-btn` selector, or a bare `.btn` / `.icon-btn`
//                     rule (`.btn`, `.btn:hover`, `.icon-btn:disabled`; a compound
//                     like `.btn.accent` or `.icon-btn.spin` is a variant, not a
//                     redefinition) → use the global `.btn` (+ `small` / `primary`
//                     / `ghost`) and `.icon-btn`; keep a page-specific class for
//                     layout-only tweaks (components.md §1).
//   accent-text       `color: var(--accent)` → var(--accent-text) (the fill
//                     colour fails contrast as text).
//   accent-fill       a rule set with `background(-color): var(--accent)` AND
//                     a `color:` declaration — i.e. text on the raw accent,
//                     which is ~3.6:1 under white (fails AA) and ignores the
//                     Warm-dark `--accent-contrast`. Fill with
//                     var(--accent-solid) + color: var(--accent-contrast)
//                     (as `.btn.primary` does). Text-less dots/bars (no
//                     `color:` in the rule set) stay on --accent.
//   outline-removed   `outline: none|0` in a rule set that (a) has no :focus
//                     in its selector (:focus/-visible/-within), (b) sets no
//                     border-color / box-shadow itself, and (c) whose OWN
//                     selector (its last class, else its element) has no
//                     replacement ring in the file: no `:focus` / `:focus-visible`
//                     rule on that class setting border-color / box-shadow /
//                     outline, and no `:focus-within` ring on an ancestor
//                     (`.box:focus-within` for `.box textarea`). Judged per
//                     selector — one ring elsewhere in the file no longer
//                     excuses every other bare control. That is the "bare
//                     input, nothing lights up" case → use `.input` or
//                     `.input-group` (app.css), or add a ring for that control.
//
//   hover-only-reveal a rule set hiding a control (`opacity: 0` /
//                     `visibility: hidden`) that the file reveals on `:hover`
//                     (`.row:hover .x { opacity: 1 }`) with no keyboard or
//                     touch path: no `:focus-visible` / `:focus-within`
//                     reveal of the same class and no `(hover: none)` query
//                     in the file. Keyboard users tab onto an invisible
//                     button and touch screens never see it → use the
//                     global `.reveal-on-hover` (app.css), which covers all
//                     three.
//   heavy-weight      font-weight ≥ 650 / bold (or the same inside a `font:`
//                     shorthand) — chrome uses 400/500/600.
//   local-pill-class  a component rule set that DRAWS a pill — it sets a
//                     background, border-radius or padding — on a local
//                     `.pill` / `.chip` / `.badge` / `.tag` class (compounds
//                     like `.chip.status-open` / `.row .tag` count; `:global(…)`
//                     and differently named classes don't). Layout-only tweaks
//                     (margin, flex, font-size) don't count either. → <Badge
//                     tone label> (lib/components/Badge.svelte), the one
//                     tinted pill.
//   font-size-literal a px / em / rem font-size of 11px or more that is not a
//                     var(--fs-*) step → the scale (foundations §2.1). A 16px
//                     input that stops iOS zooming carries `ui-guards: allow`.
//   media-width       an @media min-/max-width other than the two breakpoints
//                     (640/641 phone, 1024/1025 tablet; layout.md §5) — a
//                     container query or a documented allow instead.
//   data-bar-transition  a `transition` on width / inline-size / flex-basis /
//                     stroke-dasharray: data bars and meters show the value,
//                     they don't animate to it (foundations §8). A resize the
//                     user drags carries `ui-guards: allow`.
//   disabled-opacity  an `opacity` literal (0.2–0.7) in a disabled-state rule set
//                     (`:disabled`, `[disabled]`, `[aria-disabled…]`,
//                     `.disabled`) → var(--disabled-opacity) (0.45, tokens.css);
//                     `node scripts/codemods/disabled-opacity.mjs` rewrites them.
//   local-spinner     a component rule set spinning its own ring (`animation:
//                     … spin …`, incl. otto-spin) → the global `.spinner`
//                     (`--spinner-size` for the diameter).
//
// Markup / script rules:
//   text-loader       a <p>/<div>/<span> whose whole text is "Loading …" — a
//                     named plain-text loader still reads as an empty state
//                     → LoadState / Skeleton (a busy BUTTON label is fine).
//   danger-menu-ellipsis  a menu row `{ label: '…', danger: true }` whose label
//                     doesn't end in "…": a destructive row that opens a
//                     confirm says so (patterns §6). An instant action with
//                     no confirm carries `ui-guards: allow`.
//
// Ratcheted rules that scan MARKUP:
//
//   a11y-ignore       a `svelte-ignore a11y_…` comment — each one silences a
//                     real accessibility check (a click on a div, a missing
//                     label…). Fix the markup (a real <button>, a label)
//                     instead of muting the compiler.
//   bidi-dir          a <textarea> or text-like <input> (no type, text,
//                     search, url, email, tel) with no `dir` → dir="auto" for
//                     human language, dir="ltr" for code / paths / URLs
//                     (accessibility.md §6). scripts/codemods/bidi-dir.mjs
//                     adds them; re-run it after a merge.
//   unlabeled-control an <input> / <select> / <textarea> with no accessible
//                     name: no aria-label / aria-labelledby / title, no
//                     <label for> naming its id, not inside a <label>. A
//                     placeholder is not a label (it vanishes on typing and
//                     many screen readers skip it) → aria-label, or a <label>.
//
// More ratcheted MARKUP rules (script/style/comments blanked):
//
//   icon-size         an `<Icon size={…}>` literal off the icon scale
//                     (foundations §9): 12, 13–14, 16, 24–26 — and 20 only in
//                     the phone touch chrome (ICON_TOUCH_CHROME below). Numeric
//                     literals inside an expression count (`compact ? 16 : 26`).
//                     `node scripts/codemods/icon-sizes.mjs` snaps them.
//   inline-retry      a hand-rolled Retry button (`…Retry</button>`) outside
//                     lib/components → LoadState (`error` + `onretry`), which
//                     owns the one "Couldn’t load X / detail / Retry" look
//                     (components.md §11).
//   local-tablist     a `role="tablist"` outside lib/components (a `.segmented`
//                     class no longer exempts it — any look can be a <Tabs>
//                     with `variant`) → <Tabs> (lib/components/Tabs.svelte:
//                     one style, arrow keys, roving focus, named panels).
//   segmented-state   a button inside a `.segmented` group that marks its
//                     selection with `active` but carries no `aria-pressed`,
//                     `aria-selected`/`aria-checked` or `role="tab|radio"` —
//                     VoiceOver reads identical buttons with no state. Plain
//                     action groups (no `active`) are fine.
//
// Two more ratcheted rules scan SCRIPT code (.ts/.js files and the non-style
// part of .svelte files, comments blanked) — perf patterns (GAPS §0 G):
//
//   raw-set-interval  `setInterval(` outside lib/poll.ts and the clock/UI
//                     allowlist (INTERVAL_ALLOW below: 1 s clocks, fps
//                     meters, a touch keep-alive) → pollWhileVisible /
//                     liveQuery (lib/poll.ts, lib/live.ts), which pause while
//                     hidden, never overlap, and follow the events socket.
//   smooth-scroll     a literal `behavior: 'smooth'` — JS scrolling ignores the
//                     CSS reduced-motion override → `behavior: scrollBehavior()`
//                     (lib/motion.ts); a Svelte transition takes motionMs(ms).
//   body-style        `document.body.style.cursor|userSelect = …` or
//                     `documentElement.style.setProperty(…)` → a
//                     full-document style recalc per write (SF-02/SF-03);
//                     use lib/dragCursor.ts (overlay) / a scoped custom prop.
//
// One ratcheted rule scans the E2E SPECS (ui/e2e/*.ts, top level):
//
//   e2e-wait-timeout  `page.waitForTimeout(…)` — a fixed sleep is either too
//                     short on a slow runner (flake) or wasted time on a fast
//                     one → wait for the condition: expect(…).toBeVisible(),
//                     expect.poll(…), page.waitForResponse(…). A sleep that
//                     proves ABSENCE ("nothing else fires within 1 s") is
//                     legitimate — mark it `ui-guards: allow`.
//
// Updating the baseline: `node scripts/ui-guards.mjs --update-baseline`
// rewrites scripts/ui-guards-baseline.json from the current tree (sorted keys,
// deterministic). Do it when you PAY DOWN debt (so the lower count sticks) —
// never to wave through a new violation; fix that instead.
//
// Escape hatch for a deliberate exception (any rule): put `ui-guards: allow`
// in a comment on the same line.

import { readFileSync, readdirSync, statSync, writeFileSync, existsSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { markupOf, startTags, attrValue } from './codemods/markup.mjs';
import { needsDir } from './codemods/bidi-dir.mjs';

const UI = fileURLToPath(new URL('..', import.meta.url));
const SRC = join(UI, 'src');
const BASELINE = join(UI, 'scripts', 'ui-guards-baseline.json');
const UPDATE = process.argv.includes('--update-baseline');

/** Custom properties set by libraries we style against, not by our code. */
const EXTERNAL_PREFIXES = ['--xy-'];

function walk(dir, out = []) {
  for (const name of readdirSync(dir).sort()) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (/\.(svelte|ts|js|css|html)$/.test(name)) out.push(p);
  }
  return out;
}

const files = walk(SRC).map((p) => ({ path: p, rel: relative(UI, p), text: readFileSync(p, 'utf8') }));

function lineOf(text, index) {
  let n = 1;
  for (let i = 0; i < index; i++) if (text.charCodeAt(i) === 10) n++;
  return n;
}
function lineText(text, index) {
  const s = text.lastIndexOf('\n', index - 1) + 1;
  const e = text.indexOf('\n', index);
  return text.slice(s, e === -1 ? undefined : e);
}
const allowed = (text, index) => lineText(text, index).includes('ui-guards: allow');

/** Blank out comments (keeping offsets/newlines) so prose never trips a rule. */
const blank = (m) => m.replace(/[^\n]/g, ' ');
function stripComments(text) {
  return text
    .replace(/\/\*[\s\S]*?\*\//g, blank)
    .replace(/<!--[\s\S]*?-->/g, blank)
    .replace(/(^|[\s;{}(,])\/\/[^\n]*/g, (m, lead) => lead + blank(m.slice(lead.length)));
}

const problems = [];

// ---------- rule 1: native dialogs ----------
const DIALOG = /(?<![\w$.])(?:(?:window|globalThis|self)\.)?(confirm|prompt|alert)\(/g;
for (const f of files) {
  if (!/\.(svelte|ts|js)$/.test(f.path)) continue;
  const code = stripComments(f.text);
  for (const m of code.matchAll(DIALOG)) {
    const before = code.slice(Math.max(0, m.index - 20), m.index);
    if (/function\s+$/.test(before)) continue; // a declaration, not a call
    // A method definition (`confirm(x: T): R {`) — also not a call.
    if (/^\s*(async\s+)?(confirm|prompt|alert)\([^)]*\)\s*(:[^{=]*)?\{\s*$/.test(lineText(code, m.index))) continue;
    if (allowed(f.text, m.index)) continue;
    const use = { confirm: 'confirmer.ask()', prompt: 'confirmer.promptText()', alert: 'toasts.*()' }[m[1]];
    problems.push(
      `${f.rel}:${lineOf(f.text, m.index)}  native ${m[1]}() — a silent no-op in the Tauri webview; use ${use} (lib/confirm.svelte.ts, lib/toast.svelte.ts)`,
    );
  }
}

// ---------- rule 2: undefined CSS custom properties ----------
const DEFS = [
  /(--[\w-]+)\s*:/g, // CSS declaration / inline style="--x: …"
  /['"`](--[\w-]+)['"`]\s*[:,)]/g, // setProperty('--x', …) / { '--x': … }
  /style:(--[\w-]+)/g, // Svelte style directive
  /\s(--[\w-]+)=/g, // Svelte component custom-property prop
];
const defsOf = (text) => new Set([...DEFS].flatMap((re) => [...text.matchAll(re)].map((m) => m[1])));
// .html files under src/ are standalone documents (export/design templates):
// they neither see the app's tokens nor define any for it.
const isDoc = (f) => f.path.endsWith('.html');
const appDefined = new Set(files.filter((f) => !isDoc(f)).flatMap((f) => [...defsOf(f.text)]));

const USE = /var\(\s*(--[\w-]+)/g;
for (const f of files) {
  const code = stripComments(f.text);
  const defined = isDoc(f) ? defsOf(f.text) : appDefined;
  for (const m of code.matchAll(USE)) {
    const name = m[1];
    if (defined.has(name) || EXTERNAL_PREFIXES.some((p) => name.startsWith(p))) continue;
    if (allowed(f.text, m.index)) continue;
    problems.push(
      `${f.rel}:${lineOf(f.text, m.index)}  var(${name}) — defined nowhere under src/; use a token from lib/tokens.css (or declare it locally)`,
    );
  }
}

// ---------- ratcheted rules ----------
/** Stylesheets that are not app chrome: the token source itself, and the Site
 *  Studio stylesheet (byte-identical to crates/otto-design's copy; it styles
 *  exported sites from their own brand slots, not from our tokens). */
const STYLE_EXCLUDE = new Set(['src/lib/tokens.css', 'src/modules/design-hall/site/engine/site.css']);

const RULES = {
  'color-literal': 'literal colour — use a token from lib/tokens.css',
  'font-size-small': 'px font-size below 11px — use the --fs-* scale (11px floor for readable text)',
  'z-index-literal': 'z-index literal outside -1…10 — use a layer token var(--z-*) (tokens.css)',
  'physical-prop': 'physical left/right property — use the logical one (…-inline-start/-end, text-align: start/end)',
  'global-class': 'local style on a global app.css class — prefix the class name, or wrap it in :global() on purpose',
  'local-button-class': 'local style redefining the shared button system (.tb-btn, bare .btn/.icon-btn) — use the global .btn / .icon-btn; a page-specific class for layout tweaks',
  'accent-text': 'color: var(--accent) as text — use var(--accent-text)',
  'accent-fill': 'text on background: var(--accent) — fill with var(--accent-solid) and set color: var(--accent-contrast)',
  'outline-removed': 'outline: none on a control with no ring of its own — add a :focus / :focus-visible ring on that class (or a :focus-within ring on its wrapper): border-color + box-shadow as app.css .input / .input-group',
  'raw-set-interval': 'raw setInterval — use pollWhileVisible / liveQuery (lib/poll.ts, lib/live.ts); clocks go on INTERVAL_ALLOW',
  'status-as-text': 'color: var(--status-*) — status tokens are for dots/bars; text uses --success/--danger/--warning/--text-dim',
  'token-fallback': 'var(--token, fallback) on a token defined in lib/tokens.css — drop the fallback',
  'radius-literal': 'off-scale border-radius literal — use var(--radius-s|m|l) (0, 1-2px hairlines, 50%, 999px allowed)',
  'heavy-weight': 'font-weight ≥ 650 (or bold in a font: shorthand) — chrome uses 400/500/600',
  'local-pill-class': 'local pill look (background / radius / padding on a local .pill/.chip/.badge/.tag) — use <Badge> (lib/components/Badge.svelte)',
  'hover-only-reveal': 'control hidden until :hover with no :focus-visible/:focus-within reveal and no (hover: none) fallback — use .reveal-on-hover (app.css)',
  'a11y-ignore': 'svelte-ignore a11y_… — fix the markup (real <button>, label) instead of silencing the check',
  'bidi-dir': 'free-text field with no dir — dir="auto" for prose, dir="ltr" for code/paths/URLs (node scripts/codemods/bidi-dir.mjs)',
  'unlabeled-control': 'form control with no accessible name (placeholder is not a label) — add aria-label, or a <label for> / wrapping <label>',
  'focus-accent': 'outline in var(--accent) — focus rings use var(--accent-text)',
  'physical-shorthand': '4-value padding/margin/inset with different left/right — use -block / -inline',
  'private-keyframes': 'private @keyframes — spinners use .spinner / otto-spin, live-dot pulses otto-pulse, entrances otto-fade-in / otto-pop-in (app.css); keep a local one only when the motion is genuinely different',
  'transition-literal': 'transition with a literal 80–220 ms duration — use var(--dur-fast) / var(--dur-enter)',
  'straight-couldnt': "straight apostrophe in “Couldn't” — write Couldn’t (content.md §3)",
  'toast-failed-title': 'toasts.error titled “… failed” / “Could not …” — use toastError(\'Couldn’t <verb> …\', e)',
  'off-grid-spacing': 'padding/margin/gap px value off the spacing scale (2 px steps to 24, 4 px above; foundations.md §3)',
  'letter-spacing-literal': 'letter-spacing other than .06em (uppercase micro-labels) or -0.01em (titles)',
  'raw-toast-body': 'toast body is the raw exception (e.message / String(e)) — use toastError(title, e)',
  'hand-plural': "hand-rolled plural (`? 's' : ''`) — use plural(n, word) from lib/plural.ts",
  'uk-spelling': 'UK spelling in user copy (Cancelled, colour, analyse, behaviour…) — US English (content.md)',
  'straight-contraction': "straight apostrophe in a contraction (don't, it's, can't…) in user copy — use ’",
  'accent-mix-literal': 'hand-typed color-mix(var(--accent) N%, transparent) — use the accent ladder: --accent-faint / -soft / -soft-strong / -line / -line-strong (tokens.css)',
  'bare-loading': 'bare “Loading…” — name what loads (“Loading branches…”) or use LoadState',
  'text-loader': 'plain-text loader (<p>/<div>/<span>Loading …</…>) — use LoadState / Skeleton',
  'font-size-literal': 'font-size literal ≥ 11px (px/em/rem) — use the --fs-* scale (16px iOS input: ui-guards: allow)',
  'media-width': '@media width other than 640/641 or 1024/1025 — use the two breakpoints (layout.md §5)',
  'data-bar-transition': 'transition on width / inline-size / flex-basis / stroke-dasharray — data bars show the value, they don’t animate to it',
  'local-spinner': 'local spinning ring (animation: …spin…) — use the global .spinner (app.css)',
  'danger-menu-ellipsis': 'danger menu row whose label doesn’t end in “…” — a destructive row that opens a confirm ends in “…”',
  'e2e-wait-timeout': 'waitForTimeout in an E2E spec — wait for the condition (expect…toBeVisible / expect.poll / waitForResponse); an absence-proving sleep carries `ui-guards: allow`',
  'icon-size': 'off-scale <Icon size> — 12, 13–14, 16, 24–26 (20 in phone touch chrome only); run scripts/codemods/icon-sizes.mjs (foundations §9)',
  'inline-retry': 'hand-rolled Retry button — use LoadState (error + onretry) or EmptyState tone="error" (components.md §11)',
  'local-tablist': 'local role="tablist" — use <Tabs> (lib/components/Tabs.svelte; components.md §3)',
  'segmented-state': '.segmented value picker whose selected button (class:active) has no aria-pressed / role="tab|radio" — the state is invisible to a screen reader (components.md §3)',
  'smooth-scroll': "literal behavior: 'smooth' — use scrollBehavior() from lib/motion.ts (reduced motion)",
  'disabled-opacity': 'opacity literal on a disabled state — use var(--disabled-opacity) (scripts/codemods/disabled-opacity.mjs)',
  'body-style': 'document-level style write (body cursor/userSelect, documentElement setProperty) — use lib/dragCursor.ts or a scoped custom property',
};

/** Clocks, UI animation and explicitly bounded protocol/media lifecycles, not
 *  HTTP data polls (GAPS §0 G1). Protocol clocks must keep running when hidden. */
const INTERVAL_ALLOW = new Set([
  'src/lib/poll.ts',
  'src/lib/api/mock.ts',
  'src/lib/stores/now.svelte.ts',
  'src/App.svelte', // offline boot retry
  'src/shell/StatusBar.svelte',
  'src/shell/NotificationBell.svelte',
  'src/modules/workflows/RunSteps.svelte',
  'src/modules/workflows/RunAgents.svelte',
  'src/modules/product/design/DesignArena.svelte',
  'src/modules/git/CreatePr.svelte',
  'src/modules/git/WipPanel.svelte',
  'src/modules/vault/KnowledgeMetadata.svelte',
  'src/modules/agents/conversation/ConversationView.svelte', // touch keep-alive
  'src/modules/share/SharePage.svelte', // 60 s token refresh
  'src/modules/canvas/PresentMode.svelte',
  'src/modules/browser/live/RemoteLiveView.svelte', // fps meter
  'src/modules/database/ResultsGrid.svelte', // running-query elapsed clock
  'src/modules/rooms/RoomAnnotations.svelte', // local expiry clock; effect cleanup clears it
  'src/modules/rooms/room-client.ts', // WebSocket heartbeat; detach/close clears it
  'src/modules/rooms/room-media.ts', // audio acknowledgement + capture geometry; leave/stop/dispose clear timers
  'src/modules/rooms/recap-capture.ts', // consent-epoch media clock; bounded sample/upload queues, reset/finish stop it
]);

/** [{ css, offset }] — the CSS to scan and where it starts in the file. */
function styleBlocks(f) {
  if (f.path.endsWith('.css')) return STYLE_EXCLUDE.has(f.rel) ? [] : [{ css: f.text, offset: 0 }];
  if (!f.path.endsWith('.svelte')) return [];
  const out = [];
  for (const m of f.text.matchAll(/<style\b[^>]*>([\s\S]*?)<\/style[^>]*>/gi)) {
    out.push({ css: m[1], offset: m.index + m[0].indexOf('>') + 1 });
  }
  return out;
}

const HEX = /#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3,4})(?![\w-])/g;
const COLOR_FN = /(?<![\w-])(rgba?|hsla?)\(([^)]*)\)/g;
const NAMED = /(?<![\w-])(white|black)(?![\w-])/g;
const SCRIM = /^\s*0\s*[,\s]\s*0\s*[,\s]\s*0(?:\s*[,/]|\s*$)/; // rgba(0,0,0,x) / rgb(0 0 0 / x)
const DECL = /(^|[{;])(\s*)((?:--)?[a-zA-Z][\w-]*)\s*:([^;{}]*)/g;
const PRELUDE = /(^|[{};])([^{};@]*[^{};@\s][^{};@]*)\{/g;
// A selector that IS the shared button (`.btn`, `.icon-btn:disabled`), or any `.tb-btn`.
// `.row .btn` (a context tweak) and `.btn.accent` (a variant) are not redefinitions.
const LOCAL_BUTTON = /^\.(?:btn|icon-btn)(?::[\w-]+(?:\([^()]*\))*)*$|\.tb-btn(?![\w-])/;
const GLOBAL_CLASS = /\.(btn|chip|row|card|input|icon-btn)(?![\w-])/g;
/** Custom properties declared in lib/tokens.css (a fallback on one is dead code). */
const TOKEN_NAMES = new Set(
  [...(files.find((f) => f.rel === 'src/lib/tokens.css')?.text ?? '').matchAll(/(?:^|[\s;{])(--[a-z0-9-]+)\s*:/g)].map((m) => m[1]),
);
/** Split a CSS value on top-level whitespace (parentheses kept together). */
function splitTop(v) {
  const out = [];
  let depth = 0;
  let cur = '';
  for (const ch of v) {
    if (ch === '(') depth++;
    if (ch === ')') depth--;
    if (/\s/.test(ch) && depth === 0) {
      if (cur) out.push(cur);
      cur = '';
    } else cur += ch;
  }
  if (cur) out.push(cur);
  return out;
}
const PHYSICAL = /^(?:(?:margin|padding)-(?:left|right)|border-(?:left|right)(?:-[a-z]+)?|left|right)$/;

/** @type {Record<string, Record<string, {count: number, hits: string[]}>>} */
const found = Object.fromEntries(Object.keys(RULES).map((r) => [r, {}]));
function hit(rule, f, index, detail) {
  if (allowed(f.text, index)) return;
  const entry = (found[rule][f.rel] ??= { count: 0, hits: [] });
  entry.count++;
  entry.hits.push(`${f.rel}:${lineOf(f.text, index)}  ${detail}`);
}

for (const f of files) {
  for (const { css: raw, offset } of styleBlocks(f)) {
    const css = raw.replace(/\/\*[\s\S]*?\*\//g, blank);
    if (f.rel !== 'src/app.css') {
      for (const k of css.matchAll(/@keyframes\s+([\w-]+)/g)) hit('private-keyframes', f, offset + k.index, `@keyframes ${k[1]}`);
    }
    for (const q of css.matchAll(/@media[^{]*/g)) {
      for (const w of q[0].matchAll(/(?:min|max)-width\s*:\s*(\d+(?:\.\d+)?)px/g)) {
        if (!['640', '641', '1024', '1025'].includes(w[1])) hit('media-width', f, offset + q.index + w.index, `@media …${w[0]}`);
      }
    }
    for (const d of css.matchAll(DECL)) {
      const prop = d[3].toLowerCase();
      const value = d[4];
      const at = offset + d.index + d[1].length + d[2].length;
      const vAt = offset + d.index + d[0].length - value.length;
      for (const m of value.matchAll(HEX)) hit('color-literal', f, vAt + m.index, `${prop}: …${m[0]}`);
      for (const m of value.matchAll(COLOR_FN)) {
        if (/^rgba?$/.test(m[1]) && SCRIM.test(m[2])) continue;
        hit('color-literal', f, vAt + m.index, `${prop}: …${m[0]}`);
      }
      for (const m of value.matchAll(NAMED)) hit('color-literal', f, vAt + m.index, `${prop}: …${m[0]}`);
      if (prop === 'font-size') {
        const px = /^\s*(\d+(?:\.\d+)?)px\b/.exec(value);
        if (px && Number(px[1]) < 11) hit('font-size-small', f, at, `font-size: ${px[1]}px`);
        else if (px || /^\s*\d*\.?\d+r?em\b/.test(value)) hit('font-size-literal', f, at, `font-size:${value.trimEnd()}`);
      }
      if ((prop === 'transition' || prop === 'transition-property') && /(?:^|[\s,])(?:width|inline-size|flex-basis|stroke-dasharray)(?=[\s,]|$)/.test(value)) hit('data-bar-transition', f, at, `${prop}:${value.trimEnd()}`);
      if (f.rel !== 'src/app.css' && (prop === 'animation' || prop === 'animation-name') && /(?:^|[\s,])(?:otto-)?spin\b/.test(value)) hit('local-spinner', f, at, `${prop}:${value.trimEnd()}`);
      if (prop === 'z-index') {
        const v = value.trim().replace(/\s*!important$/, '');
        const n = /^-?\d+$/.test(v) ? Number(v) : NaN;
        const ok = (Number.isFinite(n) && n >= -1 && n <= 10) || /var\(\s*--z-/.test(v) || /^(auto|inherit|initial|unset)$/.test(v);
        if (!ok) hit('z-index-literal', f, at, `z-index: ${v}`);
      }
      if (PHYSICAL.test(prop)) hit('physical-prop', f, at, `${prop}:${value.trimEnd()}`);
      if (prop === 'text-align' && /^\s*(left|right)\b/.test(value)) hit('physical-prop', f, at, `text-align:${value.trimEnd()}`);
      if (prop === 'color' && /^\s*var\(\s*--accent\s*\)/.test(value)) hit('accent-text', f, at, 'color: var(--accent)');
      if (prop === 'transition' && /\b(?:(?:8|9)\d|1\d\d|2[01]\d|220)ms\b|\b0?\.(?:1|2)\d?s\b/.test(value)) hit('transition-literal', f, at, `transition:${value.trimEnd()}`);
      if (prop === 'color' && /var\(\s*--status-/.test(value)) hit('status-as-text', f, at, `color:${value.trimEnd()}`);
      for (const m of value.matchAll(/var\(\s*(--[\w-]+)\s*,/g)) if (TOKEN_NAMES.has(m[1])) hit('token-fallback', f, vAt + m.index, m[0]);
      if (prop === 'border-radius' && /^\s*([3-9]|1[0-9]|2[0-9])px\s*$/.test(value)) hit('radius-literal', f, at, `border-radius:${value.trimEnd()}`);
      if (prop === 'font-weight' && /^\s*(6[5-9]\d|[7-9]\d\d|bold|bolder)\b/.test(value)) hit('heavy-weight', f, at, `font-weight:${value.trimEnd()}`);
      if (prop === 'font' && /(?:^|\s)(6[5-9]\d|[7-9]\d\d|bold|bolder)(?=\s)/.test(value)) hit('heavy-weight', f, at, `font:${value.trimEnd()}`);
      if ((prop === 'outline' || prop === 'outline-color') && /var\(\s*--accent\s*\)/.test(value)) hit('focus-accent', f, at, `${prop}:${value.trimEnd()}`);
      if (/^(?:padding|margin|gap|row-gap|column-gap)(?:-[a-z-]+)?$/.test(prop)) {
        for (const m of value.matchAll(/(?<![\w.])-?(\d+)px\b/g)) {
          const n = Number(m[1]);
          if (n >= 2 && (n <= 24 ? n % 2 : n % 4)) hit('off-grid-spacing', f, vAt + m.index, `${prop}: …${m[0]}`);
        }
      }
      for (const m of value.matchAll(/color-mix\(in srgb,\s*var\(--accent\)\s*\d+%,\s*transparent\)/g)) hit('accent-mix-literal', f, vAt + m.index, m[0]);
      if (prop === 'letter-spacing' && !/^\s*(?:\.06em|0\.06em|-0?\.01em|0|normal|inherit)\s*(?:!important\s*)?$/.test(value)) hit('letter-spacing-literal', f, at, `letter-spacing:${value.trimEnd()}`);
      if (prop === 'padding' || prop === 'margin' || prop === 'inset') {
        const parts = splitTop(value.replace(/!important/, '').trim());
        if (parts.length === 4 && parts[1] !== parts[3]) hit('physical-shorthand', f, at, `${prop}:${value.trimEnd()}`);
      }
    }
    if (f.path.endsWith('.svelte')) {
      for (const p of css.matchAll(PRELUDE)) {
        const sel = p[2];
        // Drop :global(…) groups (balanced one level deep) and the `:global`
        // prefix form, then look for a bare global class.
        const local = sel.replace(/:global\((?:[^()]|\([^()]*\))*\)/g, (m) => blank(m));
        if (/(^|\s|,):global\s/.test(local)) continue;
        for (const one of local.split(',')) {
          if (LOCAL_BUTTON.test(one.trim())) hit('local-button-class', f, offset + p.index + p[1].length, `selector "${one.trim()}" redefines the shared button`);
        }
        for (const m of local.matchAll(GLOBAL_CLASS)) {
          hit('global-class', f, offset + p.index + p[1].length + m.index, `selector "${sel.trim()}" styles .${m[1]}`);
        }
      }
    }
  }
}

// rule-set rules (accent-fill, outline-removed): scan LEAF rule sets
// (`selector { decls }` with no nested braces — @media/@container bodies
// are reached through their leaf rules). Simple on purpose: no cascade, no
// cross-file knowledge.
const LEAF = /(?<=^|[{};])([^{};]*)\{([^{}]*)\}/g;
const isPillClass = (c) => /^(?:pill|chip|badge|tag)$/.test(c);
// `at` is the property name itself (not the whitespace before it), so a
// same-line `ui-guards: allow` comment is found.
const declsOf = (body) => [...body.matchAll(/(^|;)\s*([a-zA-Z-]+)\s*:([^;]*)/g)].map((d) => ({ prop: d[2].toLowerCase(), value: d[3].trim(), at: d.index + d[0].indexOf(d[2]) }));
for (const f of files) {
  const blocks = styleBlocks(f).map(({ css, offset }) => ({ css: css.replace(/\/\*[\s\S]*?\*\//g, blank), offset }));
  const sets = [];
  for (const { css, offset } of blocks) {
    for (const m of css.matchAll(LEAF)) {
      const sel = m[1].trim();
      if (!sel || sel.startsWith('@')) continue;
      const bodyAt = offset + m.index + m[0].indexOf('{') + 1;
      sets.push({ sel, decls: declsOf(m[2]).map((d) => ({ ...d, at: bodyAt + d.at })) });
    }
  }
  // A declaration "sets" a ring unless its WHOLE value is none/0 — `box-shadow:
  // 0 0 0 3px …` (the .input ring) starts with 0 and still counts.
  const sets2 = (s, props) => s.decls.some((d) => props.includes(d.prop) && !/^(?:none|0)\s*(?:!important)?$/.test(d.value));
  const RING = ['border-color', 'border', 'box-shadow', 'outline', 'outline-color', 'background', 'background-color'];
  // The control a selector part styles: its last class, else its last element name.
  const subjectOf = (part) => {
    const last = part.trim().replace(/:global\(([^()]*)\)/g, '$1').split(/[\s>+~]+/).pop() ?? '';
    return /\.([\w-]+)/.exec(last.replace(/:{1,2}[\w-]+(\([^)]*\))?/g, ''))?.[1] ?? /^([a-z][\w-]*)/i.exec(last)?.[1];
  };
  /** A replacement ring for `subject`: a :focus/:focus-visible rule on it, or a
   *  :focus-within rule on any compound that the bare selector descends from. */
  const hasRingFor = (part) => {
    const subject = subjectOf(part);
    if (!subject) return false;
    const re = new RegExp(`(?:\\.|^|[\\s>+~])${subject.replace(/[-]/g, '\\-')}(?![\\w-])[^\\s,]*:focus(?:-visible)?\\b`);
    const ancestors = part.trim().split(/[\s>+~]+/).slice(0, -1).map((a) => a.replace(/:{1,2}[\w-]+(\([^)]*\))?/g, ''));
    return sets.some((r) => r.sel.split(',').some((rp) =>
      (re.test(rp.trim()) && sets2(r, RING)) ||
      (/:focus-within/.test(rp) && sets2(r, RING) && (ancestors.length === 0 || ancestors.some((a) => a && rp.includes(a))) ),
    ));
  };
  // hover-only-reveal: the hidden control is keyed by the LAST class of each
  // selector part (`.row:hover .x-close` → `x-close`).
  const lastClass = (sel) => /\.([\w-]+)(?:[^.\s>+~]*)$/.exec(sel.trim())?.[1];
  const hides = (s) => s.decls.some((d) => (d.prop === 'opacity' && /^0(?:\.0+)?\s*(?:!important)?$/.test(d.value)) || (d.prop === 'visibility' && /^hidden\b/.test(d.value)));
  const shows = (s) => s.decls.some((d) => (d.prop === 'opacity' && !/^0(?:\.0+)?\s*(?:!important)?$/.test(d.value)) || (d.prop === 'visibility' && /^visible\b/.test(d.value)));
  const touchFallback = blocks.some(({ css }) => /\((?:any-)?hover\s*:\s*none\)/.test(css));
  if (!touchFallback) {
    const revealedBy = (cls, re) => sets.some((r) => shows(r) && r.sel.split(',').some((part) => re.test(part) && lastClass(part) === cls));
    const flagged = new Set();
    for (const s of sets) {
      if (!hides(s)) continue;
      for (const part of s.sel.split(',')) {
        const cls = lastClass(part);
        if (!cls || flagged.has(cls) || /:(?:hover|focus)/.test(part)) continue;
        if (revealedBy(cls, /:hover/) && !revealedBy(cls, /:focus-visible|:focus-within|:focus\b/)) {
          flagged.add(cls);
          hit('hover-only-reveal', f, s.decls[0]?.at ?? 0, `"${part.trim()}" is revealed on :hover only`);
        }
      }
    }
  }
  // local-pill-class: the shared Badge is the one place a pill is drawn.
  if (f.path.endsWith('.svelte') && f.rel !== 'src/lib/components/Badge.svelte') {
    for (const s of sets) {
      if (!s.decls.some((d) => /^(?:background(?:-color)?|border-radius|padding(?:-[a-z-]+)?)$/.test(d.prop))) continue;
      const local = s.sel.replace(/:global\((?:[^()]|\([^()]*\))*\)/g, ' ');
      const cls = [...local.matchAll(/\.([\w-]+)/g)].map((m) => m[1]).find(isPillClass);
      if (cls) hit('local-pill-class', f, s.decls[0].at, `"${s.sel}" draws a local .${cls}`);
    }
  }
  const DISABLED_SEL = /:disabled\b|\[disabled\]|\[aria-disabled|\.disabled(?![\w-])/;
  for (const s of sets) {
    if (!DISABLED_SEL.test(s.sel)) continue;
    for (const d of s.decls) {
      if (d.prop === 'opacity' && /^0?\.(?:[2-6]\d*|7)\s*(?:!important)?$/.test(d.value)) hit('disabled-opacity', f, d.at, `"${s.sel}" opacity: ${d.value}`);
    }
  }
  for (const s of sets) {
    const bg = s.decls.find((d) => (d.prop === 'background' || d.prop === 'background-color') && /^var\(\s*--accent\s*\)/.test(d.value));
    if (bg && s.decls.some((d) => d.prop === 'color')) hit('accent-fill', f, bg.at, `"${s.sel}" — ${bg.prop}: var(--accent) under text`);
    const ol = s.decls.find((d) => d.prop === 'outline' && /^(none|0)\b/.test(d.value));
    if (ol && !/:focus/.test(s.sel) && !sets2(s, ['border-color', 'box-shadow'])) {
      const bare = s.sel.split(',').filter((part) => !hasRingFor(part));
      if (bare.length) hit('outline-removed', f, ol.at, `"${bare.map((b) => b.trim()).join(', ')}" — outline: ${ol.value} with no focus replacement for that control`);
    }
  }
}

// ---------- copy ratchets (script + markup text) ----------
for (const f of files) {
  if (f.path.endsWith('.svelte')) {
    for (const m of f.text.matchAll(/svelte-ignore[^\n]*?\ba11y_[\w]+/g)) hit('a11y-ignore', f, m.index, m[0]);
  }
  if (!/\.(svelte|ts)$/.test(f.path)) continue;
  for (const m of f.text.matchAll(/Couldn't/g)) hit('straight-couldnt', f, m.index, "Couldn't");
  const copy = stripComments(f.text).replace(/<style\b[\s\S]*?<\/style[^>]*>/gi, blank);
  for (const m of copy.matchAll(/(?:toasts\.(?:error|warning)|toastError)\((['"`])(?!Couldn’t)(?:[^'"`\n]*?(?<![{(,]\s?)\bfailed\b(?!\s*[,)}])|Could not |Cannot )/gi)) hit('toast-failed-title', f, m.index, m[0]);
  // …and a ternary title: toasts.error(ok ? 'Delete failed' : 'Archive failed', …).
  for (const m of copy.matchAll(/(?:toasts\.(?:error|warning)|toastError)\([^'"`;\n]*\?\s*(['"`])(?!Couldn’t)[^'"`\n]*?\b(?:failed|Could not|Cannot)\b/g)) hit('toast-failed-title', f, m.index, m[0].slice(0, 80));
  for (const m of copy.matchAll(/toasts\.(?:error|warning)\([^;]*?,\s*(?:\w+ instanceof Error \? \w+\.message|String\((?:e|err|error|ex)\)|\((?:e|err|error|ex) as Error\)\.message|(?:e|err|error|ex)\.message)/g)) hit('raw-toast-body', f, m.index, m[0].slice(0, 60));
  // …or the raw exception as the TITLE (toasts.error(e.message)).
  for (const m of copy.matchAll(/toasts\.(?:error|warning)\(\s*(?:\w+ instanceof Error\b|String\((?:e|err|error|ex)\)|\((?:e|err|error|ex) as Error\)\.message|(?:e|err|error|ex)\.message)/g)) hit('raw-toast-body', f, m.index, m[0].slice(0, 60));
  for (const m of copy.matchAll(/\?\s*'s'\s*:\s*''|\?\s*''\s*:\s*'s'/g)) hit('hand-plural', f, m.index, m[0]);
  for (const m of copy.matchAll(/\b(?:Cancell(?:ed|ing)|colours?|Colours?|analys(?:e|ed|ing)|Analys(?:e|ed|ing)|behaviour|organis(?:e|ation)|favourite|(?<=[A-Za-z] )cancelled)\b/g)) hit('uk-spelling', f, m.index, m[0]);
  {
    let at = 0;
    for (const l of copy.split('\n')) {
      if (!/\.match\(|\.includes\(|\.test\(|\.replace\(|RegExp|startsWith|endsWith|===|!==|\.split\(|^\s*import |querySelector|https?:\/\//.test(l)) {
        for (const m of l.matchAll(/(?<=[A-Za-z])'(?:t|s|re|ll|ve|m|d)\b/g)) hit('straight-contraction', f, at + m.index, l.slice(Math.max(0, m.index - 12), m.index + 4));
      }
      at += l.length + 1;
    }
  }
  for (const m of copy.matchAll(/(?:>\s*|['"`])Loading(?:…|\.\.\.)\s*(?=<|['"`])/g)) hit('bare-loading', f, m.index, m[0].trim());
  if (f.path.endsWith('.svelte')) {
    // `{…}` interpolations count (`Loading {label.toLowerCase()}…`); LoadState
    // itself renders the one sanctioned (visually hidden) loading line.
    if (f.rel !== 'src/lib/components/LoadState.svelte') {
      for (const m of copy.matchAll(/<(p|div|span)\b[^>]*>\s*Loading (?:[^<{]|\{[^{}]*\})*?(?:…|\.\.\.)\s*<\/\1>/g)) hit('text-loader', f, m.index, m[0].slice(0, 80));
    }
  }
  // `{ label: 'Delete', …, danger: true }` (either order; one flat object).
  for (const m of copy.matchAll(/\{[^{}]*\}/g)) {
    if (!/\bdanger\s*:\s*true\b/.test(m[0])) continue;
    const l = /\blabel\s*:\s*(['"`])((?:(?!\1).)*)\1/.exec(m[0]);
    if (l && !/…$/.test(l[2]) && !/\.\.\.$/.test(l[2])) hit('danger-menu-ellipsis', f, m.index + l.index, `label: ${l[1]}${l[2]}${l[1]}`);
  }
}

// ---------- rule 3 (HARD): icon-only buttons — scan markup (script/style/comments blanked) ----------
function tagEnd(text, i) {
  // From just after `<button`, find the closing `>` of the start tag,
  // skipping quoted strings and {…} expressions (which may contain `>`).
  let depth = 0;
  let q = '';
  for (; i < text.length; i++) {
    const c = text[i];
    if (q) {
      if (c === q) q = '';
    } else if (c === '"' || c === "'" || (c === '`' && depth > 0)) q = c;
    else if (c === '{') depth++;
    else if (c === '}') depth = Math.max(0, depth - 1);
    else if (c === '>' && depth === 0) return i;
  }
  return -1;
}
for (const f of files) {
  if (!f.path.endsWith('.svelte')) continue;
  const markup = f.text
    .replace(/<script\b[\s\S]*?<\/script[^>]*>/gi, blank)
    .replace(/<style\b[\s\S]*?<\/style[^>]*>/gi, blank)
    .replace(/<!--[\s\S]*?-->/g, blank);
  for (const m of markup.matchAll(/<button(?=[\s>])/g)) {
    const end = tagEnd(markup, m.index + 7);
    if (end === -1) continue;
    const attrs = markup.slice(m.index + 7, end);
    if (attrs.trimEnd().endsWith('/')) continue;
    const close = markup.indexOf('</button>', end);
    if (close === -1) continue;
    const body = markup.slice(end + 1, close).trim();
    if (!body.startsWith('<Icon') || !body.endsWith('/>') || body.split('<').length !== 2) continue;
    if (/\{\s*\.\.\./.test(attrs)) continue; // spread props may carry the labels
    const hasAria = /(^|\s)aria-label(ledby)?\s*=|\{aria-label\}/.test(attrs);
    const hasTitle = /(^|\s)title\s*=|\{title\}/.test(attrs);
    if (hasAria && hasTitle) continue;
    const missing = [!hasAria && 'aria-label', !hasTitle && 'title'].filter(Boolean).join(' + ');
    if (allowed(f.text, m.index)) continue;
    problems.push(`${f.rel}:${lineOf(f.text, m.index)}  icon-only <button> missing ${missing} — add both (name the action)`);
  }
}

// ---------- markup a11y ratchets: bidi-dir, unlabeled-control ----------
// A file input is opened by a labelled button and never focused itself.
const NO_NAME_TYPES = /^(hidden|submit|button|reset|image|file)$/;
for (const f of files) {
  if (!f.path.endsWith('.svelte')) continue;
  const markup = markupOf(f.text);
  // <label …>…</label> spans: a control inside one is named by it.
  const spans = [];
  for (const m of markup.matchAll(/<label(?=[\s>])/g)) {
    const close = markup.slice(m.index).search(/<\/label\s*>/);
    if (close !== -1) spans.push([m.index, m.index + close]);
  }
  const forIds = new Set([...markup.matchAll(/\sfor\s*=\s*(?:"([^"]*)"|'([^']*)'|\{([^}]*)\})/g)].map((m) => (m[1] ?? m[2] ?? m[3]).trim()));
  for (const t of startTags(f.text, ['input', 'select', 'textarea'], markup)) {
    if (needsDir(t.tag, t.attrs)) hit('bidi-dir', f, t.start, `<${t.tag}> without dir`);
    if (/\{\s*\.\.\./.test(t.attrs)) continue; // a spread may carry the label
    const type = attrValue(t.attrs, 'type');
    if (t.tag === 'input' && type && NO_NAME_TYPES.test(type)) continue;
    // Out of the accessibility tree (a hidden file picker opened by a button).
    if (/(?:^|\s)hidden(?:\s|$|=)|aria-hidden\s*=\s*["{]?\s*["']?true/.test(t.attrs)) continue;
    if (/(?:^|\s)(?:aria-label|aria-labelledby|title)\s*=|\{(?:aria-label|title)\}/.test(t.attrs)) continue;
    const id = attrValue(t.attrs, 'id')?.trim();
    if (id && forIds.has(id)) continue;
    // A static id named again elsewhere (`{@render jsonLabel('np-body', …)}`
    // emits the <label for> from a snippet), or a snippet's own id parameter
    // (`{#snippet picker(id: string)}` — the caller pairs it with a label).
    if (id && /^[\w-]+$/.test(id) && markup.split(new RegExp(`['"]${id}['"]`)).length > 2) continue;
    if (id && /^\w+$/.test(id) && new RegExp(`\\{#snippet\\s+\\w+\\([^)]*\\b${id}\\b`).test(markup)) continue;
    if (spans.some(([a, b]) => t.start > a && t.start < b)) continue;
    hit('unlabeled-control', f, t.start, `<${t.tag}> with no accessible name`);
  }
}

// ---------- ratcheted markup rules: icon-size / inline-retry / local-tablist ----------
/** Phone touch chrome, where a 20 px icon is the documented exception (foundations §9). */
const ICON_TOUCH_CHROME = new Set(['src/shell/BottomNav.svelte', 'src/shell/MobileActionBar.svelte', 'src/shell/App.svelte']);
const ICON_SIZES = new Set([12, 13, 14, 16, 24, 25, 26]);
for (const f of files) {
  if (!f.path.endsWith('.svelte')) continue;
  const markup = f.text
    .replace(/<script\b[\s\S]*?<\/script[^>]*>/gi, blank)
    .replace(/<style\b[\s\S]*?<\/style[^>]*>/gi, blank)
    .replace(/<!--[\s\S]*?-->/g, blank);
  if (f.rel !== 'src/lib/components/Icon.svelte') {
    for (const m of markup.matchAll(/<Icon(?=[\s/>])/g)) {
      const end = tagEnd(markup, m.index + 5);
      if (end === -1) continue;
      const size = /\ssize=\{([^{}]*)\}/.exec(markup.slice(m.index, end));
      if (!size) continue;
      for (const n of size[1].matchAll(/(?<![\w.'"-])(\d+)(?![\w.'"-])/g)) {
        const v = Number(n[1]);
        if (ICON_SIZES.has(v) || (v === 20 && ICON_TOUCH_CHROME.has(f.rel))) continue;
        hit('icon-size', f, m.index, `<Icon size={${size[1].trim()}}>`);
      }
    }
  }
  if (!f.rel.startsWith('src/lib/components/')) {
    for (const m of markup.matchAll(/\b(?:Retry|Try again)\b[^<>]*<\/button>/g)) hit('inline-retry', f, m.index, m[0].slice(0, 60));
    for (const m of markup.matchAll(/<[a-z][\w-]*\b[^>]*\brole="tablist"[^>]*>/g)) hit('local-tablist', f, m.index, m[0].slice(0, 80));
  }
  // A `.segmented` group (up to its first closing </div>; segments don't nest
  // divs) whose `active`-marked buttons expose no state.
  for (const g of markup.matchAll(/<div\b[^>]*\bclass="[^"]*\bsegmented\b[^"]*"[^>]*>([\s\S]*?)<\/div>/g)) {
    const body = g[1];
    for (const b of body.matchAll(/<button\b/g)) {
      const end = tagEnd(body, b.index + 7);
      if (end === -1) continue;
      const tag = body.slice(b.index, end);
      if (!/\bclass:active\b|\bclass="[^"]*\bactive\b|\{[^}]*'active'/.test(tag)) continue;
      if (/\baria-(?:pressed|selected|checked)\b|\brole="(?:tab|radio|menuitemradio)"/.test(tag)) continue;
      hit('segmented-state', f, g.index + g[0].indexOf(body) + b.index, tag.slice(0, 80));
    }
  }
}

// script-code rules: setInterval / document-level style writes / smooth scroll.
const INTERVAL = /(?<![\w$.])(?:(?:window|globalThis|self)\.)?setInterval\s*\(/g;
const BODY_STYLE =
  /\bdocument\.(?:body|documentElement)\.style\.(?:(?:cursor|userSelect|webkitUserSelect)\s*=(?!=)|setProperty\s*\()/g;
for (const f of files) {
  if (!/\.(svelte|ts|js)$/.test(f.path)) continue;
  let code = stripComments(f.text);
  if (f.path.endsWith('.svelte')) code = code.replace(/<style\b[\s\S]*?<\/style[^>]*>/gi, blank);
  if (!INTERVAL_ALLOW.has(f.rel)) {
    for (const m of code.matchAll(INTERVAL)) hit('raw-set-interval', f, m.index, 'setInterval(');
  }
  if (f.rel !== 'src/lib/motion.ts') {
    for (const m of code.matchAll(/\bbehavior\s*:\s*(['"])smooth\1/g)) hit('smooth-scroll', f, m.index, m[0]);
  }
  if (f.rel !== 'src/lib/dragCursor.ts') {
    for (const m of code.matchAll(BODY_STYLE)) hit('body-style', f, m.index, m[0].replace(/\s+/g, ' '));
  }
}

// E2E specs: fixed sleeps (top-level ui/e2e/*.ts only — never the report dirs).
const E2E = join(UI, 'e2e');
if (existsSync(E2E)) {
  for (const name of readdirSync(E2E).sort()) {
    if (!/\.ts$/.test(name)) continue;
    const p = join(E2E, name);
    const f = { path: p, rel: relative(UI, p), text: readFileSync(p, 'utf8') };
    for (const m of stripComments(f.text).matchAll(/\bwaitForTimeout\s*\(/g)) hit('e2e-wait-timeout', f, m.index, 'waitForTimeout(');
  }
}

// ---------- baseline compare / update ----------
const current = {};
for (const rule of Object.keys(RULES).sort()) {
  const byFile = {};
  for (const rel of Object.keys(found[rule]).sort()) byFile[rel] = found[rule][rel].count;
  current[rule] = byFile;
}

if (UPDATE) {
  writeFileSync(BASELINE, JSON.stringify(current, null, 2) + '\n');
  const total = Object.values(current).reduce((s, r) => s + Object.values(r).reduce((a, b) => a + b, 0), 0);
  console.log(`ui-guards: baseline written (${relative(UI, BASELINE)}; ${total} recorded occurrence(s))`);
  if (problems.length) {
    console.error(`\nui-guards: ${problems.length} HARD problem(s) (not baselinable)\n`);
    for (const p of problems) console.error('  ' + p);
    process.exit(1);
  }
  process.exit(0);
}

const baseline = existsSync(BASELINE) ? JSON.parse(readFileSync(BASELINE, 'utf8')) : {};
const ratchet = [];
let paidDown = 0;
for (const rule of Object.keys(RULES)) {
  const was = baseline[rule] ?? {};
  for (const [rel, { count, hits }] of Object.entries(found[rule])) {
    const allowedN = was[rel] ?? 0;
    if (count > allowedN) {
      ratchet.push(`[${rule}] ${rel}: ${count} (baseline ${allowedN}) — ${RULES[rule]}`);
      for (const h of hits) ratchet.push('      ' + h);
    }
  }
  for (const [rel, n] of Object.entries(was)) if ((found[rule][rel]?.count ?? 0) < n) paidDown++;
}

if (problems.length || ratchet.length) {
  if (problems.length) {
    console.error(`ui-guards: ${problems.length} problem(s)\n`);
    for (const p of problems) console.error('  ' + p);
  }
  if (ratchet.length) {
    console.error(
      `\nui-guards: ratcheted rules got WORSE — fix the new occurrence(s) (all listed per file; the new one is among them):\n`,
    );
    for (const p of ratchet) console.error('  ' + p);
  }
  console.error('\nSee the header of ui/scripts/ui-guards.mjs for the rules.');
  process.exit(1);
}
const debt = paidDown
  ? `; ${paidDown} file/rule count(s) now below baseline — lock that in with \`node scripts/ui-guards.mjs --update-baseline\``
  : '';
console.log(`ui-guards: ok (${files.length} files; no native dialogs, no undefined CSS vars, no ratchet regressions${debt})`);
