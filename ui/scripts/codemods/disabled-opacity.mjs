#!/usr/bin/env node
// Codemod: a disabled control fades to ONE opacity — var(--disabled-opacity)
// (0.45, lib/tokens.css; what app.css .btn / .icon-btn / .segmented use).
// Rewrites `opacity: 0.2…0.7` inside any component rule set whose selector is
// a disabled state (`:disabled`, `[disabled]`, `[aria-disabled…]`,
// `.disabled`). Idempotent; re-run after a merge, `ui-guards`
// (`disabled-opacity`) keeps the tree there.
//
//   node scripts/codemods/disabled-opacity.mjs [--dry]
import { readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const UI = fileURLToPath(new URL('../..', import.meta.url));
const SRC = join(UI, 'src');
const DRY = process.argv.includes('--dry');
const DISABLED_SEL = /:disabled\b|\[disabled\]|\[aria-disabled|\.disabled(?![\w-])/;
const LITERAL = /(^|[;{\s])opacity\s*:\s*(0?\.[2-6]\d*|0?\.7)\s*(?=;|\}|$|\s*!important)/gm;

function walk(dir, out = []) {
  for (const name of readdirSync(dir).sort()) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (name.endsWith('.svelte')) out.push(p);
  }
  return out;
}

let changed = 0;
for (const path of walk(SRC)) {
  const text = readFileSync(path, 'utf8');
  const next = text.replace(/(<style\b[^>]*>)([\s\S]*?)(<\/style>)/gi, (_, open, css, close) => {
    // Leaf rule sets only (no nested braces): `selector { decls }`.
    const out = css.replace(/(?<=^|[{};])([^{};]*)\{([^{}]*)\}/g, (whole, sel, body) => {
      if (!DISABLED_SEL.test(sel.replace(/\/\*[\s\S]*?\*\//g, ''))) return whole;
      const nb = body.replace(LITERAL, (m, lead, v) => {
        changed++;
        if (DRY) console.log(`${relative(UI, path)}: ${sel.trim()} opacity ${v}`);
        return `${lead}opacity: var(--disabled-opacity)`;
      });
      return `${sel}{${nb}}`;
    });
    return open + out + close;
  });
  if (next !== text && !DRY) writeFileSync(path, next);
}
console.log(`disabled-opacity: ${changed} literal(s) ${DRY ? 'would change' : 'now var(--disabled-opacity)'}`);
