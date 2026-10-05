#!/usr/bin/env node
// Summarize the isolated Playwright workload. Read measurements, never a live DB.
// node scripts/perf/telemetry-report.mjs <component-load.json> [summary.json]
import { readFileSync, writeFileSync } from 'node:fs';
const [input, output] = process.argv.slice(2);
if (!input) throw new Error('Pass the component-load.json produced by desktop-telemetry-perf.spec.ts');
const data = JSON.parse(readFileSync(input, 'utf8'));
const mean = (values) => values.reduce((a, b) => a + b, 0) / values.length;
const percentile = (values, fraction) => [...values].sort((a, b) => a - b)[Math.min(values.length - 1, Math.floor(values.length * fraction))];
const round = (n) => Math.round(n * 100) / 100;
const groups = [];
for (const enabled of [false, true]) {
  // Early format also sampled navigation. Its first 2×module-count total rows
  // mark warm-up/measurement; exclude those from sustained resource statistics.
  const totals = data.samples.filter((s) => s.enabled === enabled && s.process === 'total');
  const navigationEnd = totals[data.modules.length * 2 - 1]?.at ?? 0;
  const sustained = data.samples.filter((s) => s.enabled === enabled && (s.phase ? s.phase === 'load' : s.at > navigationEnd));
  for (const concurrency of [1, 3, 5]) {
    const phase = sustained.filter((s) => s.concurrency === concurrency);
    for (const process of [...new Set(phase.map((s) => s.process))]) {
      const samples = phase.filter((s) => s.process === process);
      const cpu = samples.map((s) => s.cpu_percent);
      const rss = samples.map((s) => s.rss_mb);
      const time = samples.map((s) => (s.at - samples[0].at) / 60000);
      const mt = mean(time), mr = mean(rss);
      const denominator = time.reduce((sum, t) => sum + (t - mt) ** 2, 0);
      const slope = denominator ? time.reduce((sum, t, i) => sum + (t - mt) * (rss[i] - mr), 0) / denominator : 0;
      groups.push({ enabled, concurrency, process, samples: samples.length,
        seconds: round((samples.at(-1).at - samples[0].at) / 1000),
        mean_cpu_percent: round(mean(cpu)), p95_cpu_percent: round(percentile(cpu, 0.95)), max_cpu_percent: round(Math.max(...cpu)),
        mean_rss_mb: round(mr), max_rss_mb: round(Math.max(...rss)),
        start_rss_mb: round(mean(rss.slice(0, 10))), end_rss_mb: round(mean(rss.slice(-10))),
        rss_slope_mb_per_minute: round(slope) });
    }
  }
}
for (const enabled of [false, true]) for (const concurrency of [1, 3, 5]) {
  const requests = data.request_phases?.find((phase) => phase.enabled === enabled && phase.concurrency === concurrency);
  if (!requests || !(requests.requests > 0 && requests.response_bytes > 0 && requests.seconds > 0)) {
    throw new Error(`Missing achieved throughput: enabled=${enabled}, concurrency=${concurrency}`);
  }
  for (const process of ['daemon', 'browser', 'total', ...(data.request_client === 'node-fetch' ? ['load-driver'] : []), ...(enabled ? ['collector', 'clickhouse'] : [])]) {
    if (!groups.some((g) => g.enabled === enabled && g.concurrency === concurrency && g.process === process && g.samples >= 10)) {
      throw new Error(`Incomplete resource coverage: enabled=${enabled}, concurrency=${concurrency}, process=${process}`);
    }
  }
}
const navigation = data.modules.map((module) => {
  const off = data.rows.find((r) => !r.enabled && r.module === module)?.duration_ms;
  const on = data.rows.find((r) => r.enabled && r.module === module)?.duration_ms;
  if (!Number.isFinite(off) || !Number.isFinite(on)) throw new Error(`Missing navigation for ${module}`);
  return { module, off_ms: round(off), on_ms: round(on), delta_ms: round(on - off) };
});
const report = { engine: data.engine, request_client: data.request_client ?? 'playwright', browser_trace: data.browser_trace ?? true, module_count: data.modules.length, navigation, resources: groups,
  request_phases: data.request_phases ?? [],
  limitations: ['Sequential off/on phases share one browser and machine; cache, GC and unrelated host work can affect deltas.', 'Navigation has one measured sample per module per mode, so it is not a per-module latency percentile.', 'Requests are closed-loop with a 50 ms pause; compare achieved throughput and response bytes alongside CPU, not as fixed-rate exclusive request cost.', 'ps reports platform CPU estimates; RSS slope over a short phase is not proof of a memory leak.', 'Fixture agents exercise PTYs/API concurrency, not actual provider inference or remote databases.'] };
const json = JSON.stringify(report, null, 2) + '\n';
if (output) writeFileSync(output, json); else process.stdout.write(json);
