#!/usr/bin/env node
// Bundle-byte budget (perf F1), run in CI right after `npm run build`.
// Pure Node, no dependencies. Reads the Vite manifest (dist/.vite/manifest.json,
// `build.manifest: true` in vite.config.ts) and measures GZIP bytes of what
// each load actually fetches — a chunk plus its STATIC imports (and their
// CSS), never its dynamic imports:
//
//   entry          index.html's entry chunk + static closure (every document)
//   shell          the shell chunk (src/shell/App.svelte) + closure, minus entry
//   page:<key>     each shell/pages.svelte.ts LOADERS target + closure, minus
//                  entry ∪ shell — the bytes a first visit to that page adds
//   rightPanel     the right activity panel (src/shell/RightPanel.svelte, which
//                  the Agents page loads with itself) + closure, minus entry ∪
//                  shell ∪ page:agents — what it adds to the default landing
//   panel:<key>    each RightPanel.svelte PANEL_LOADERS tab + closure, minus
//                  all of the above — the bytes the first open of that tab adds
//
// The numbers are ratcheted like scripts/ui-guards-baseline.json: they live in
// scripts/bundle-budget.json and a target more than 3 % over its budget (or a
// target with no budget yet) fails. After making a chunk smaller, or after a
// deliberate growth, record the new numbers:
//
//   node scripts/bundle-budget.mjs --update
//
// `--json` prints the measurements as JSON instead of the table.
//
// Two structural rules run alongside the byte budgets (they fail on their own,
// whatever the sizes):
//
//   lazy-only    a heavy component (xterm's Terminal, CodeMirror's CodeEditor)
//                must not be in the STATIC import set of the pages listed in
//                LAZY_ONLY — they show it only inside an opened section, so it
//                must stay behind a dynamic import (LazyTerminal / LazyMount).
//                Byte budgets have 3 % slack and a page can shrink elsewhere;
//                this pins the specific regression.
//                LAZY_ONLY_PKG does the same for a whole npm package (`three`).
//   duplicates   no npm package may be bundled from two node_modules locations
//                (e.g. a nested mermaid under @excalidraw/mermaid-to-excalidraw
//                next to the top-level one) — pin it with package.json
//                `overrides` instead of shipping both copies.

import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';

const UI = join(dirname(fileURLToPath(import.meta.url)), '..');
const DIST = join(UI, 'dist');
const MANIFEST = join(DIST, '.vite', 'manifest.json');
const BUDGET = join(UI, 'scripts', 'bundle-budget.json');
const PAGES = join(UI, 'src', 'shell', 'pages.svelte.ts');
const RIGHT_PANEL = join(UI, 'src', 'shell', 'RightPanel.svelte');
/** Allowed growth over the recorded budget before CI fails. */
const TOLERANCE = 0.03;

const args = new Set(process.argv.slice(2));

if (!existsSync(MANIFEST)) {
  console.error(`bundle-budget: ${MANIFEST} not found — run \`npm run build\` first.`);
  process.exit(2);
}
/** @type {Record<string, {file: string, css?: string[], imports?: string[], isEntry?: boolean, src?: string}>} */
const manifest = JSON.parse(readFileSync(MANIFEST, 'utf8'));

const gzCache = new Map();
function gz(file) {
  let n = gzCache.get(file);
  if (n === undefined) {
    n = gzipSync(readFileSync(join(DIST, file)), { level: 9 }).length;
    gzCache.set(file, n);
  }
  return n;
}

/** Output files (js + css) a manifest key loads eagerly. */
function closure(key, out = new Set(), seen = new Set()) {
  if (seen.has(key)) return out;
  seen.add(key);
  const c = manifest[key];
  if (!c) throw new Error(`bundle-budget: no manifest entry "${key}"`);
  if (c.file.endsWith('.js')) out.add(c.file);
  for (const css of c.css ?? []) out.add(css);
  for (const imp of c.imports ?? []) closure(imp, out, seen);
  return out;
}

const sum = (files) => [...files].reduce((n, f) => n + gz(f), 0);
const minus = (a, b) => new Set([...a].filter((f) => !b.has(f)));

const entryKey = Object.keys(manifest).find((k) => manifest[k].isEntry && k.endsWith('index.html'));
if (!entryKey) throw new Error('bundle-budget: no index.html entry in the manifest');
const entry = closure(entryKey);
const shell = minus(closure('src/shell/App.svelte'), entry);
const base = new Set([...entry, ...shell]);

/** @type {Record<string, number>} */
const measured = { entry: sum(entry), shell: sum(shell) };

/** `key → manifest key` of a `{ key: () => import('./x') }` loader map in a
 *  src/shell file, read from source so a new entry is budgeted the moment it
 *  is added. A key's target is the first `import('…')` after it. */
function loaderMap(file, marker) {
  const src = readFileSync(file, 'utf8');
  const at = src.indexOf(marker);
  if (at < 0) throw new Error(`bundle-budget: "${marker}" not found in ${file}`);
  const end = Math.min(...['\n};', '\n  };'].map((t) => src.indexOf(t, at)).filter((i) => i >= 0));
  /** @type {Map<string, string>} */
  const out = new Map();
  let current = null;
  for (const line of src.slice(at, end).split('\n')) {
    const key = line.match(/^\s*'?([a-zA-Z][\w-]*)'?:\s*(?:\(\)|async)/);
    if (key) current = key[1];
    const imp = line.match(/import\('(\.\.?\/[^']+)'\)/);
    if (imp && current) {
      out.set(current, join('src/shell', imp[1]).replaceAll('\\', '/'));
      current = null;
    }
  }
  return out;
}

// Page targets straight from the lazy page map.
const pages = loaderMap(PAGES, 'const LOADERS');
for (const [key, src] of pages) {
  if (manifest[src]) measured[`page:${key}`] = sum(minus(closure(src), base));
}

// The right activity panel and its per-tab chunks (perf G1).
const RP_KEY = 'src/shell/RightPanel.svelte';
if (manifest[RP_KEY]) {
  const agents = pages.get('agents');
  const landing = new Set([...base, ...(agents && manifest[agents] ? closure(agents) : [])]);
  const rp = minus(closure(RP_KEY), landing);
  measured.rightPanel = sum(rp);
  const withPanel = new Set([...landing, ...rp]);
  for (const [key, src] of loaderMap(RIGHT_PANEL, 'PANEL_LOADERS: Record')) {
    if (manifest[src]) measured[`panel:${key}`] = sum(minus(closure(src), withPanel));
  }
} else {
  throw new Error(`bundle-budget: no manifest entry "${RP_KEY}" — is the right panel still a lazy chunk?`);
}

// ---- lazy-only: heavy components out of named pages' static import sets ----
/** Heavy component → the page keys (shell/pages.svelte.ts) that must reach it
 *  only through a dynamic import. */
const LAZY_ONLY = {
  'src/lib/components/Terminal.svelte': [
    'vault',
    'product',
    'settings',
    'git',
    'workflows',
    'personal-agents',
    'canvas',
    'browser',
    'kubernetes',
    'aws',
    'skills-eval',
  ],
  'src/lib/components/CodeEditor.svelte': ['settings'],
};
/** npm package → the page keys whose STATIC import set must not reach it
 *  (same rule as LAZY_ONLY, for a whole package). `three` (~650 kB) is only
 *  ever behind a dynamic import: Home's Classrooms widget and the Design Hall
 *  3D surfaces load it on first mount (home/school/scene.ts, scene3d/build.ts). */
const LAZY_ONLY_PKG = {
  three: ['home', 'agents'],
};
const ruleFailures = [];
/** Output files holding `src`'s own code: its chunk when it is a dynamic entry
 *  (with Rollup's `_<Name>-<hash>.js` split when it is ALSO imported
 *  statically somewhere), or the shared `_<Name>-<hash>.js` chunk when it is
 *  only ever imported statically. */
function heavyFiles(src) {
  const name = src.split('/').pop().replace(/\.svelte$/, '');
  const shared = new RegExp(`^_${name}-[\\w-]+\\.js$`);
  const files = new Set();
  for (const [k, c] of Object.entries(manifest)) {
    if (k === src || shared.test(k)) files.add(c.file);
  }
  return files;
}
for (const [src, keys] of Object.entries(LAZY_ONLY)) {
  const heavy = heavyFiles(src);
  if (heavy.size === 0) continue; // not bundled at all — nothing can leak
  for (const key of keys) {
    const page = pages.get(key);
    if (!page || !manifest[page]) {
      ruleFailures.push(`lazy-only: page "${key}" is not in shell/pages.svelte.ts LOADERS — update LAZY_ONLY`);
      continue;
    }
    const leaked = [...closure(page)].filter((f) => heavy.has(f));
    if (leaked.length) {
      ruleFailures.push(
        `lazy-only: page:${key} statically imports ${src} (${leaked.join(', ')}) — load it through a dynamic import (LazyTerminal / lazyComponent)`,
      );
    }
  }
}

for (const [pkg, keys] of Object.entries(LAZY_ONLY_PKG)) {
  const prefix = `node_modules/${pkg}/`;
  const heavy = new Set(
    Object.entries(manifest)
      .filter(([k, c]) => k.includes(prefix) || (c.src ?? '').includes(prefix))
      .map(([, c]) => c.file),
  );
  if (heavy.size === 0) continue;
  for (const key of keys) {
    const page = pages.get(key);
    if (!page || !manifest[page]) {
      ruleFailures.push(`lazy-only: page "${key}" is not in shell/pages.svelte.ts LOADERS — update LAZY_ONLY_PKG`);
      continue;
    }
    const leaked = [...closure(page)].filter((f) => heavy.has(f));
    if (leaked.length) {
      ruleFailures.push(`lazy-only: page:${key} statically imports the ${pkg} package (${leaked.join(', ')}) — load it through a dynamic import`);
    }
  }
}

// ---- duplicates: one bundled copy per npm package ----
/** package name → the node_modules prefixes it was bundled from. */
const pkgLocations = new Map();
for (const c of Object.values(manifest)) {
  const m = (c.src ?? '').match(/^(.*node_modules\/)((?:@[^/]+\/)?[^/]+)\//);
  if (!m) continue;
  if (!pkgLocations.has(m[2])) pkgLocations.set(m[2], new Set());
  pkgLocations.get(m[2]).add(m[1]);
}
for (const [name, at] of pkgLocations) {
  if (at.size > 1) {
    ruleFailures.push(
      `duplicates: ${name} is bundled from ${at.size} copies (${[...at].join(', ')}) — dedupe with package.json "overrides"`,
    );
  }
}

const budget = existsSync(BUDGET) ? JSON.parse(readFileSync(BUDGET, 'utf8')) : {};
const kb = (n) => `${(n / 1024).toFixed(1)} KB`;

if (args.has('--json')) {
  console.log(JSON.stringify(measured, null, 2));
  process.exit(0);
}

if (ruleFailures.length) {
  console.error(`bundle-budget: ${ruleFailures.length} structural rule failure(s):\n  ${ruleFailures.join('\n  ')}`);
}

if (args.has('--update')) {
  const sorted = Object.fromEntries(Object.entries(measured).sort(([a], [b]) => a.localeCompare(b)));
  writeFileSync(BUDGET, `${JSON.stringify(sorted, null, 2)}\n`);
  console.log(`bundle-budget: recorded ${Object.keys(sorted).length} targets in scripts/bundle-budget.json`);
  process.exit(0);
}

const failures = [];
const shrunk = [];
const rows = [];
for (const [name, bytes] of Object.entries(measured)) {
  const cap = budget[name];
  let status = 'ok';
  if (cap === undefined) {
    status = 'NO BUDGET';
    failures.push(`${name}: ${kb(bytes)} has no budget — run \`node scripts/bundle-budget.mjs --update\``);
  } else if (bytes > cap * (1 + TOLERANCE)) {
    status = 'OVER';
    failures.push(`${name}: ${kb(bytes)} gzip > ${kb(cap)} budget (+${(((bytes - cap) / cap) * 100).toFixed(1)} %)`);
  } else if (bytes < cap * (1 - TOLERANCE)) {
    shrunk.push(name);
  }
  rows.push(`${name.padEnd(24)} ${kb(bytes).padStart(10)}  ${cap === undefined ? '—'.padStart(10) : kb(cap).padStart(10)}  ${status}`);
}
console.log(`${'target'.padEnd(24)} ${'gzip'.padStart(10)}  ${'budget'.padStart(10)}`);
console.log(rows.join('\n'));
if (shrunk.length) {
  console.log(`\nbundle-budget: ${shrunk.length} target(s) shrank past the tolerance (${shrunk.join(', ')}) — ratchet with --update.`);
}
if (ruleFailures.length) process.exit(1);
if (failures.length) {
  console.error(`\nbundle-budget: ${failures.length} over budget:\n  ${failures.join('\n  ')}`);
  console.error('Lazy-load what grew (dynamic import), or — when the growth is deliberate — record it with --update.');
  process.exit(1);
}
console.log('\nbundle-budget: within budget');
