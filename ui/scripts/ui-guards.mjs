#!/usr/bin/env node
// Static UI guards, run first by `npm run check` (so CI enforces them too).
// Pure Node, no dependencies. Two rules, both born from real bugs:
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
// Escape hatch for a deliberate exception: put `ui-guards: allow` in a
// comment on the same line.

import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const UI = fileURLToPath(new URL('..', import.meta.url));
const SRC = join(UI, 'src');

/** Custom properties set by libraries we style against, not by our code. */
const EXTERNAL_PREFIXES = ['--xy-'];

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
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
function stripComments(text) {
  const blank = (m) => m.replace(/[^\n]/g, ' ');
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

if (problems.length) {
  console.error(`ui-guards: ${problems.length} problem(s)\n`);
  for (const p of problems) console.error('  ' + p);
  console.error('\nSee the header of ui/scripts/ui-guards.mjs for the rules.');
  process.exit(1);
}
console.log(`ui-guards: ok (${files.length} files; no native dialogs, no undefined CSS vars)`);
