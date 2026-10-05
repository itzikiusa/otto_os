#!/usr/bin/env node
// Fail CI when a Playwright run reports any FLAKY test (passed only on retry).
// Retries are 0 in playwright.config.ts, so a non-zero `flaky` count means a
// spec opted back into retries — which hides a real intermittent failure.
//
//   node scripts/e2e-flaky-check.mjs [e2e/.results-0.json ...]
//
// With no arguments it reads every e2e/.results-*.json. A missing report is an
// error: the gate must never pass vacuously because the reporter did not run.
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const files = process.argv.slice(2);
if (files.length === 0) {
  const dir = 'e2e';
  for (const f of existsSync(dir) ? readdirSync(dir) : []) {
    if (/^\.results-.*\.json$/.test(f)) files.push(join(dir, f));
  }
}
if (files.length === 0) {
  console.error('[flaky-check] no Playwright JSON report found (e2e/.results-*.json)');
  process.exit(1);
}

let flaky = 0;
const names = [];
const walk = (suite, trail) => {
  for (const spec of suite.specs ?? []) {
    for (const t of spec.tests ?? []) {
      if (t.status === 'flaky') names.push(`${[...trail, spec.title].join(' › ')} [${t.projectName}]`);
    }
  }
  for (const child of suite.suites ?? []) walk(child, [...trail, child.title]);
};
for (const f of files) {
  const report = JSON.parse(readFileSync(f, 'utf8'));
  const stats = report.stats ?? {};
  flaky += stats.flaky ?? 0;
  for (const s of report.suites ?? []) walk(s, [s.title]);
  console.log(
    `[flaky-check] ${f}: expected=${stats.expected ?? 0} unexpected=${stats.unexpected ?? 0} ` +
      `flaky=${stats.flaky ?? 0} skipped=${stats.skipped ?? 0}`,
  );
}
if (flaky > 0) {
  for (const n of names) console.error(`::error::flaky test: ${n}`);
  console.error(`[flaky-check] ${flaky} flaky test(s) — fix the race instead of retrying`);
  process.exit(1);
}
