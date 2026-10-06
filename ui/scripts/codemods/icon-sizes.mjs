#!/usr/bin/env node
// Codemod: snap every `<Icon … size={N}>` under src/ onto the icon scale
// (docs/design/guidelines/foundations.md §9). Idempotent — re-run it after a
// merge brings in new off-scale sizes, then let `ui-guards` (`icon-size`)
// confirm the tree is clean.
//
//   node scripts/codemods/icon-sizes.mjs          # rewrite in place
//   node scripts/codemods/icon-sizes.mjs --dry    # list the changes only
//
// The scale: 12 (small buttons, chips, carets), 13–14 (buttons, toolbars),
// 16 (nav rows, standalone), 20 (phone touch chrome only — TOUCH_CHROME
// below), 24–26 (empty-state / placeholder tiles). Mapping:
//   ≤ 11 → 12   15 → 14   17–18 → 16   19–23 → 24 (20 in touch chrome)   ≥ 27 → 26
// Only numeric literals inside the size={…} expression are rewritten, so
// `size={compact ? 18 : 26}` becomes `size={compact ? 16 : 26}`.
import { readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const UI = fileURLToPath(new URL('../..', import.meta.url));
const SRC = join(UI, 'src');
const DRY = process.argv.includes('--dry');

/** Phone touch chrome: 20 px icons are the documented exception here. */
const TOUCH_CHROME = new Set(['src/shell/BottomNav.svelte', 'src/shell/MobileActionBar.svelte', 'src/shell/App.svelte']);

function snap(n, rel) {
  const touch = TOUCH_CHROME.has(rel);
  if (touch && n >= 17 && n <= 23) return 20;
  if (n <= 11) return 12;
  if (n === 15) return 14;
  if (n === 17 || n === 18) return 16;
  if (n >= 19 && n <= 23) return 24;
  if (n >= 27) return 26;
  return n;
}

function walk(dir, out = []) {
  for (const name of readdirSync(dir).sort()) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (name.endsWith('.svelte')) out.push(p);
  }
  return out;
}

/** Index of the `>` closing the start tag that begins at `i`, skipping strings and {…}. */
function tagEnd(text, i) {
  let depth = 0;
  let q = '';
  for (; i < text.length; i++) {
    const c = text[i];
    if (q) {
      if (c === q) q = '';
    } else if (depth > 0 && (c === '"' || c === "'" || c === '`')) q = c;
    else if (c === '{') depth++;
    else if (c === '}') depth = Math.max(0, depth - 1);
    else if (c === '>' && depth === 0) return i;
  }
  return -1;
}

let changed = 0;
let files = 0;
for (const path of walk(SRC)) {
  const rel = relative(UI, path);
  if (rel === 'src/lib/components/Icon.svelte') continue;
  const text = readFileSync(path, 'utf8');
  let out = '';
  let last = 0;
  for (const m of text.matchAll(/<Icon(?=[\s/>])/g)) {
    const end = tagEnd(text, m.index + 5);
    if (end === -1) continue;
    const tag = text.slice(m.index, end);
    const next = tag.replace(/(\ssize=\{)([^{}]*)\}/, (whole, open, expr) =>
      `${open}${expr.replace(/(?<![\w.'"-])(\d+)(?![\w.'"-])/g, (d) => {
        const s = snap(Number(d), rel);
        if (s !== Number(d)) {
          changed++;
          if (DRY) console.log(`${rel}: size ${d} → ${s}`);
        }
        return String(s);
      })}}`,
    );
    if (next !== tag) {
      out += text.slice(last, m.index) + next;
      last = end;
    }
  }
  if (last) {
    out += text.slice(last);
    files++;
    if (!DRY) writeFileSync(path, out);
  }
}
console.log(`icon-sizes: ${changed} size(s) in ${files} file(s) ${DRY ? 'would change' : 'snapped to the scale'}`);
