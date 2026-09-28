#!/usr/bin/env node
// Summarize a load-test run: per (phase, N, UI state) mean CPU% and RSS per
// process group, page metrics, WS bytes/s and API requests/s.
// Usage: node scripts/loadtest/analyze.mjs <run dir> [<run dir> ...]
import fs from 'node:fs';
import path from 'node:path';

const GROUPS = ['ottod', 'clickhouse', 'agents', 'daemon-children', 'renderer', 'gpu', 'browser-main', 'browser-utility'];
const mean = (xs) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : NaN);
const f1 = (x) => (Number.isFinite(x) ? x.toFixed(1) : '-');
for (const dir of process.argv.slice(2)) {
  const rows = fs.readFileSync(path.join(dir, 'samples.jsonl'), 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l));
  const keyOf = (r) => `${r.phase}|${r.n}|${r.state}`;
  const buckets = new Map();
  for (const r of rows) {
    if (!buckets.has(keyOf(r))) buckets.set(keyOf(r), []);
    buckets.get(keyOf(r)).push(r);
  }
  console.log(`\n## ${dir}  (${rows.length} samples)`);
  console.log('phase|N|state|k| ' + GROUPS.map((g) => `${g} cpu%/rssMB`).join(' | ') + ' | heapMB | domEl | lay/s | styleMs/s | scriptMs/s | taskMs/s | jank>50ms/s | termWS KB/s | eventsWS KB/s | api req/s | threads d/ch/r');
  for (const [k, rs] of buckets) {
    // skip the first sample of a phase for CPU (its delta spans the UI transition)
    const cpuRows = rs.length > 2 ? rs.slice(1) : rs;
    const cells = GROUPS.map((g) => {
      const c = mean(cpuRows.map((r) => r.groups[g]?.cpuPct).filter((x) => x != null));
      const m = rs[rs.length - 1].groups[g]?.rssMB;
      return `${f1(c)}/${m ?? '-'}`;
    });
    const P = (fn) => mean(cpuRows.map((r) => (r.page ? fn(r.page) : NaN)).filter(Number.isFinite));
    const wsRate = (pat) =>
      P((p) => Object.entries(p.ws ?? {}).filter(([u]) => u.includes(pat)).reduce((a, [, v]) => a + v.bytes, 0) / 5 / 1024);
    const req = P((p) => Object.values(p.req ?? {}).reduce((a, b) => a + b, 0) / 5);
    const last = rs[rs.length - 1];
    const th = `${last.groups.ottod?.threads ?? '-'}/${last.groups.clickhouse?.threads ?? '-'}/${last.groups.renderer?.threads ?? '-'}`;
    console.log(
      [k.replaceAll('|', ' | '), rs.length, ...cells, f1(P((p) => p.jsHeapMB)), f1(P((p) => p.domNow)), f1(P((p) => p.layoutsPerS)), f1(P((p) => p.styleMsPerS)), f1(P((p) => p.scriptMsPerS)), f1(P((p) => p.taskMsPerS)), f1(P((p) => (p.frames?.jank ?? 0) / 5)), f1(wsRate('/ws/term')), f1(wsRate('/ws/events')), f1(req), th].join(' | '),
    );
  }
  // top API endpoints per bucket (by count) — polling / storms
  console.log('\n### top API endpoints per phase (total calls in phase)');
  for (const [k, rs] of buckets) {
    const agg = {};
    for (const r of rs) for (const [u, c] of Object.entries(r.page?.req ?? {})) agg[u] = (agg[u] ?? 0) + c;
    const top = Object.entries(agg).sort((a, b) => b[1] - a[1]).slice(0, 6).map(([u, c]) => `${u.replace(/\/api\/v1/, '')}×${c}`);
    if (top.length) console.log(`${k}: ${top.join(', ')}`);
  }
}
