#!/usr/bin/env node
// Fail CI when a Playwright run reports any FLAKY test (passed only on retry).
// Retries are 0 in playwright.config.ts, so a non-zero `flaky` count means a
// spec opted back into retries — which hides a real intermittent failure.
//
//   node scripts/e2e-flaky-check.mjs [--gate] [--summary] [e2e/.results-0.json ...]
//
// --gate     the BLOCKING smoke gate (ci.yml `e2e-gate`): also fail when any
//            test was skipped or `expected` (passed) fell below the committed
//            floor GATE_MIN_EXPECTED in e2e/gate-specs.ts. A gate spec that
//            self-skips (a `project.name` guard, an env gate) would otherwise
//            pass vacuously (S12-301).
// --ratchet  the advisory suite's red-count ratchet (ci.yml
//            `e2e-advisory-ratchet`, all four shards' reports): fail when the
//            failed-test count rises above e2e/advisory-red-baseline.json.
// --summary  append a pass/fail/skip table to $GITHUB_STEP_SUMMARY, so an
//            advisory shard's red count is visible on the run page (S12-304).
//
// With no file arguments it reads every e2e/.results-*.json. A missing report
// is an error: the gate must never pass vacuously because the reporter did not run.
import { appendFileSync, existsSync, readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

/** Fold Playwright JSON reports into totals + offending test names. */
export function tally(reports) {
  const t = { expected: 0, unexpected: 0, flaky: 0, skipped: 0, flakyNames: [], failedNames: [] };
  const walk = (suite, trail) => {
    for (const spec of suite.specs ?? []) {
      for (const test of spec.tests ?? []) {
        const name = `${[...trail, spec.title].join(' › ')} [${test.projectName}]`;
        if (test.status === 'flaky') t.flakyNames.push(name);
        if (test.status === 'unexpected') t.failedNames.push(name);
      }
    }
    for (const child of suite.suites ?? []) walk(child, [...trail, child.title]);
  };
  for (const report of reports) {
    const stats = report.stats ?? {};
    t.expected += stats.expected ?? 0;
    t.unexpected += stats.unexpected ?? 0;
    t.flaky += stats.flaky ?? 0;
    t.skipped += stats.skipped ?? 0;
    for (const s of report.suites ?? []) walk(s, [s.title]);
  }
  return t;
}

/**
 * The verdict for a run: a list of error lines (empty = pass). Flaky always
 * fails; in gate mode a skipped test or an `expected` count below `minExpected`
 * fails too.
 */
export function verdict(t, { gate = false, minExpected = 0 } = {}) {
  const errors = [];
  if (t.flaky > 0) {
    for (const n of t.flakyNames) errors.push(`flaky test: ${n}`);
    errors.push(`${t.flaky} flaky test(s) — fix the race instead of retrying`);
  }
  if (gate) {
    if (t.skipped > 0) {
      errors.push(
        `${t.skipped} gate test(s) SKIPPED — a gate spec must run every test on the gate projects ` +
          `(check its test.skip / project.name guard, or move it out of e2e/gate-specs.ts)`,
      );
    }
    if (t.expected < minExpected) {
      errors.push(
        `only ${t.expected} gate test(s) passed, below the floor GATE_MIN_EXPECTED=${minExpected} ` +
          `(e2e/gate-specs.ts) — a spec stopped running; lower the floor only when a spec leaves the gate`,
      );
    }
  }
  return errors;
}

/**
 * The red-count ratchet: an error when `unexpected` rose above the baseline,
 * a notice when it fell (so the PR that fixes failures lowers the baseline).
 */
export function ratchet(t, maxFailed) {
  if (t.unexpected > maxFailed) {
    return {
      error:
        `advisory e2e red count rose: ${t.unexpected} failed > baseline ${maxFailed} ` +
        `(e2e/advisory-red-baseline.json) — fix the new failures; never raise the baseline to land one`,
    };
  }
  if (t.unexpected < maxFailed) {
    return { notice: `advisory e2e red count fell to ${t.unexpected} (baseline ${maxFailed}) — lower the baseline` };
  }
  return {};
}

/** Markdown for $GITHUB_STEP_SUMMARY. */
export function summaryMarkdown(t, label) {
  const lines = [
    `### ${label}`,
    '',
    '| passed | failed | flaky | skipped |',
    '|---:|---:|---:|---:|',
    `| ${t.expected} | ${t.unexpected} | ${t.flaky} | ${t.skipped} |`,
    '',
  ];
  if (t.failedNames.length) {
    lines.push('<details><summary>Failed tests</summary>', '');
    for (const n of t.failedNames.slice(0, 200)) lines.push(`- ${n}`);
    lines.push('', '</details>', '');
  }
  return lines.join('\n');
}

async function main() {
  const args = process.argv.slice(2);
  const gate = args.includes('--gate');
  const summary = args.includes('--summary');
  const useRatchet = args.includes('--ratchet');
  const files = args.filter((a) => !a.startsWith('--'));
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
  const reports = files.map((f) => JSON.parse(readFileSync(f, 'utf8')));
  const t = tally(reports);
  console.log(
    `[flaky-check] ${files.join(', ')}: expected=${t.expected} unexpected=${t.unexpected} ` +
      `flaky=${t.flaky} skipped=${t.skipped}`,
  );
  let minExpected = 0;
  if (gate) ({ GATE_MIN_EXPECTED: minExpected } = await import('../e2e/gate-specs.ts'));
  if (summary && process.env.GITHUB_STEP_SUMMARY) {
    const label = process.env.OTTO_E2E_SUMMARY_LABEL ?? (gate ? 'E2E smoke gate' : 'E2E');
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, summaryMarkdown(t, label));
  }
  const errors = verdict(t, { gate, minExpected });
  if (useRatchet) {
    const baseline = JSON.parse(readFileSync(new URL('../e2e/advisory-red-baseline.json', import.meta.url), 'utf8'));
    const r = ratchet(t, baseline.max_failed);
    if (r.error) errors.push(r.error);
    if (r.notice) console.log(`::notice::${r.notice}`);
    if (summary && process.env.GITHUB_STEP_SUMMARY) {
      appendFileSync(process.env.GITHUB_STEP_SUMMARY, `Red count ${t.unexpected} vs baseline ${baseline.max_failed}.\n`);
    }
  }
  if (errors.length) {
    for (const e of errors) console.error(`::error::${e}`);
    process.exit(1);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await main();
