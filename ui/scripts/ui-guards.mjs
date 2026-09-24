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
// RATCHETED rules — the tree predates them, so today's debt is recorded per
// file in scripts/ui-guards-baseline.json and only an INCREASE fails (a file
// gaining an occurrence, or a new file having any). They scan the <style>
// blocks of .svelte files and the .css files under src/ (not lib/tokens.css,
// which is where literal values belong), plus button markup for the last:
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
//   accent-text       `color: var(--accent)` → var(--accent-text) (the fill
//                     colour fails contrast as text).
//   icon-button-label a <button> whose only content is <Icon …/> without
//                     BOTH aria-label and title.
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
  'accent-text': 'color: var(--accent) as text — use var(--accent-text)',
  'icon-button-label': 'icon-only <button> needs both aria-label and title',
};

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
const GLOBAL_CLASS = /\.(btn|chip|row|card|input|icon-btn)(?![\w-])/g;
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
      }
      if (prop === 'z-index') {
        const v = value.trim().replace(/\s*!important$/, '');
        const n = /^-?\d+$/.test(v) ? Number(v) : NaN;
        const ok = (Number.isFinite(n) && n >= -1 && n <= 10) || /var\(\s*--z-/.test(v) || /^(auto|inherit|initial|unset)$/.test(v);
        if (!ok) hit('z-index-literal', f, at, `z-index: ${v}`);
      }
      if (PHYSICAL.test(prop)) hit('physical-prop', f, at, `${prop}:${value.trimEnd()}`);
      if (prop === 'text-align' && /^\s*(left|right)\b/.test(value)) hit('physical-prop', f, at, `text-align:${value.trimEnd()}`);
      if (prop === 'color' && /^\s*var\(\s*--accent\s*\)/.test(value)) hit('accent-text', f, at, 'color: var(--accent)');
    }
    if (f.path.endsWith('.svelte')) {
      for (const p of css.matchAll(PRELUDE)) {
        const sel = p[2];
        // Drop :global(…) groups (balanced one level deep) and the `:global`
        // prefix form, then look for a bare global class.
        const local = sel.replace(/:global\((?:[^()]|\([^()]*\))*\)/g, (m) => blank(m));
        if (/(^|\s|,):global\s/.test(local)) continue;
        for (const m of local.matchAll(GLOBAL_CLASS)) {
          hit('global-class', f, offset + p.index + p[1].length + m.index, `selector "${sel.trim()}" styles .${m[1]}`);
        }
      }
    }
  }
}

// icon-only buttons: scan markup (script/style/comments blanked).
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
    hit('icon-button-label', f, m.index, `icon-only <button> missing ${missing}`);
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
