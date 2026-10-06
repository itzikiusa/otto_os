#!/usr/bin/env node
// Codemod: every element with an `oncontextmenu` menu also gets `use:rowMenu`
// (lib/rowMenu.ts) — the ContextMenu key / ⇧F10 and touch long-press paths
// into the same menu (a11y P2: right-click-only actions). Idempotent; re-run
// after merges:
//
//   node scripts/codemods/row-menu.mjs [--dry]
//
// Skipped: handlers that only SUPPRESS the native menu (`e.preventDefault();
// close()`), delegated handlers that read `e.target` off a container (a grid
// <tbody>), and the menu's own backdrop. Mark any other deliberate exception
// with `data-no-row-menu` on the element. Files that already declare their own
// `rowMenu` (a menu-builder function) import the action as `rowMenuKeys`.
import { readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { join, relative, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { startTags } from './markup.mjs';

const UI = fileURLToPath(new URL('../..', import.meta.url));
const SRC = join(UI, 'src');
const DRY = process.argv.includes('--dry');
const SKIP_FILES = new Set(['src/lib/components/ContextMenu.svelte', 'src/modules/agents/conversation/Markdown.svelte']);

function walk(dir, out = []) {
  for (const name of readdirSync(dir).sort()) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (name.endsWith('.svelte')) out.push(p);
  }
  return out;
}

export function wants(tag, attrs) {
  if (!/(?:^|\s)oncontextmenu\s*=/.test(attrs)) return false;
  if (/use:rowMenu(?:Keys)?\b|data-no-row-menu/.test(attrs)) return false;
  if (tag === 'tbody') return false; // delegated: the handler reads e.target
  const h = /oncontextmenu\s*=\s*\{([\s\S]*)/.exec(attrs)?.[1] ?? '';
  if (/^\s*\(\w*\)\s*=>\s*\{\s*\w+\.preventDefault\(\);\s*close\(\)/.test(h)) return false;
  return true;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  let n = 0;
  let files = 0;
  for (const p of walk(SRC)) {
    const rel = relative(UI, p);
    if (SKIP_FILES.has(rel)) continue;
    let text = readFileSync(p, 'utf8');
    if (!text.includes('oncontextmenu')) continue;
    const ownName = /\bfunction rowMenu\b|\b(?:const|let) rowMenu\b/.test(text);
    const name = ownName ? 'rowMenuKeys' : 'rowMenu';
    const tags = startTags(text, ['[a-zA-Z][\\w.:-]*']).filter((t) => /^[a-z]/.test(t.tag) && wants(t.tag, t.attrs));
    if (!tags.length) continue;
    for (const t of tags.sort((a, b) => b.nameEnd - a.nameEnd)) text = text.slice(0, t.nameEnd) + ` use:${name}` + text.slice(t.nameEnd);
    if (!new RegExp(`import \\{[^}]*\\b${name}\\b[^}]*\\} from '[./]*lib/rowMenu'`).test(text)) {
      let spec = relative(dirname(p), join(SRC, 'lib', 'rowMenu'));
      if (!spec.startsWith('.')) spec = './' + spec;
      const imp = ownName ? `  import { rowMenu as rowMenuKeys } from '${spec}';\n` : `  import { rowMenu } from '${spec}';\n`;
      const m = /<script(?![^>]*context=["']module["'])(?![^>]*\bmodule\b)[^>]*>\n/.exec(text);
      if (!m) throw new Error(`${rel}: no instance <script> to import into`);
      text = text.slice(0, m.index + m[0].length) + imp + text.slice(m.index + m[0].length);
    }
    n += tags.length;
    files++;
    if (DRY) console.log(`${rel}: ${tags.length}`);
    else writeFileSync(p, text);
  }
  console.log(`row-menu: ${DRY ? 'would add' : 'added'} use:rowMenu to ${n} element(s) in ${files} file(s)`);
}
