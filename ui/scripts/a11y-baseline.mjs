#!/usr/bin/env node
// Merge an axe recording (test-results/a11y-record.jsonl, written by
// e2e/helpers.ts `expectAccessible` under OTTO_A11Y_RECORD=1) into the
// ratcheted serious-violation allowlist e2e/a11y-baseline.json (S19-304).
// Each recorded page's entry is REPLACED by the union of rule ids seen for it
// in this recording (every project/viewport), so fixed rules drop out; pages
// not in the recording keep their entry. Review the diff: an entry should
// only ever shrink.
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const UI = join(import.meta.dirname, '..');
const rec = join(UI, 'test-results/a11y-record.jsonl');
const out = join(UI, 'e2e/a11y-baseline.json');
if (!existsSync(rec)) {
  console.error(`a11y-baseline: no recording at ${rec} — run the e2e scans with OTTO_A11Y_RECORD=1 first`);
  process.exit(1);
}
const seen = new Map();
for (const line of readFileSync(rec, 'utf8').split('\n')) {
  if (!line.trim()) continue;
  const { key, serious } = JSON.parse(line);
  const set = seen.get(key) ?? new Set();
  for (const id of serious) set.add(id);
  seen.set(key, set);
}
const base = existsSync(out) ? JSON.parse(readFileSync(out, 'utf8')) : {};
for (const [key, set] of seen) base[key] = [...set].sort();
const sorted = Object.fromEntries(Object.entries(base).sort(([a], [b]) => a.localeCompare(b)));
writeFileSync(out, `${JSON.stringify(sorted, null, 2)}\n`);
console.log(`a11y-baseline: ${seen.size} page(s) recorded → ${out}`);
