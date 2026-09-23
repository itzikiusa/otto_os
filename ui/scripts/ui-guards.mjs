#!/usr/bin/env node
// Static UI guards, run first by `npm run check` (so CI enforces them too).
// Pure Node, no dependencies. Each rule was born from a real bug:
//
// 1. No native dialogs. window.confirm()/prompt()/alert() silently return
//    false/null/undefined inside the Tauri WKWebView, so a "Delete?" guarded
//    by confirm() is a no-op in the desktop app. Use `confirmer.ask()` /
//    `confirmer.promptText()` / `confirmer.choose()` (lib/confirm.svelte.ts)
//    or a toast instead.
//
// Escape hatch for a deliberate exception: put `ui-guards: allow` in a
// comment on the same line.

import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const UI = fileURLToPath(new URL('..', import.meta.url));
const SRC = join(UI, 'src');

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

if (problems.length) {
  console.error(`ui-guards: ${problems.length} problem(s)\n`);
  for (const p of problems) console.error('  ' + p);
  console.error('\nSee the header of ui/scripts/ui-guards.mjs for the rules.');
  process.exit(1);
}
console.log(`ui-guards: ok (${files.length} files; no native dialogs)`);
