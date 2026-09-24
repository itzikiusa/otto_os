// Insights report files, as the `insights` skill writes them
// (<data_dir>/insights/{daily,weekly}/summary|report|metrics + index.json).
// There is no API to create a report, so the capture writes these. Synthetic.
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const pad = (n) => String(n).padStart(2, '0');
const ymd = (d) => `${d.getUTCFullYear()}${pad(d.getUTCMonth() + 1)}${pad(d.getUTCDate())}`;
const iso = (d) => d.toISOString().slice(0, 10);

function summary(day, s) {
  return `# Insights summary — ${day.toUTCString().slice(0, 16)} (daily)

A ${s.sessions > 60 ? 'busy' : 'steady'} build day: ${s.sessions} agent sessions (${s.codex} Codex), ${s.turns} user turns and ${s.errors} tool errors. ${s.ach}% of classified sessions reached their goal; spend was $${s.spend.toFixed(2)}.
Reviews ran ${s.reviews} times and caught ${s.findings} findings before merge.

| Area | Sessions | Share |
|---|---|---|
| Feature work | ${Math.round(s.sessions * 0.45)} | 45% |
| Code review | ${Math.round(s.sessions * 0.35)} | 35% |
| Ops | ${Math.round(s.sessions * 0.2)} | 20% |

## Action Plan (carried into the ledger)

1. **Run the test suite once per change, not per lens** — repeated test runs: ${14 + s.i} → ≤4 — effort M — act-0001 (improved)
2. **Pin the flaky cart test clock** — flaky retries: ${6 - (s.i % 3)} → 0 — effort S — act-0002 (improved)
3. **Split review prompts by PR size** — lens sessions on small PRs: ~5 → ≤3 — effort M — new
4. **Cache the dependency install in CI** — CI minutes/day: ${40 + s.i * 2} → <25 — effort S — act-0004 (open)
`;
}

function report(title, s) {
  return `<!doctype html><html><head><meta charset="utf-8"><title>${title}</title>
<style>body{font:15px/1.55 -apple-system,system-ui,sans-serif;margin:28px;color:#1d1d1f;background:#fff}
h1{font-size:24px;margin:0 0 14px}.glance{background:#eef5ff;border:1px solid #bcd6ff;border-radius:10px;padding:14px 18px}
.kpis{display:flex;gap:14px;margin:16px 0}.kpi{flex:1;border:1px solid #e5e5ea;border-radius:10px;padding:12px}.kpi b{display:block;font-size:26px}
.bar{height:12px;background:linear-gradient(90deg,#0a84ff,#7c3aed);border-radius:6px;margin:6px 0}</style></head>
<body><h1>${title}</h1><div class="glance"><strong>At a glance</strong><p>${s.sessions} sessions · ${s.errors} tool errors · ${s.ach}% achieved · $${s.spend.toFixed(2)}</p></div>
<div class="kpis"><div class="kpi">Sessions<b>${s.sessions}</b></div><div class="kpi">Achieved<b>${s.ach}%</b></div><div class="kpi">Findings caught<b>${s.findings}</b></div></div>
<h2>Sessions by hour</h2>${[30, 55, 80, 64, 92, 48, 36].map((w) => `<div class="bar" style="width:${w}%"></div>`).join('')}
</body></html>`;
}

export function writeInsights(root, now = new Date()) {
  const daily = join(root, 'daily');
  const weekly = join(root, 'weekly');
  mkdirSync(daily, { recursive: true });
  mkdirSync(weekly, { recursive: true });
  const series = [];
  for (let i = 9; i >= 1; i--) {
    const d = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate() - i));
    const k = 9 - i;
    const s = { i: k, sessions: 38 + ((k * 17) % 45), codex: 6 + (k % 5), turns: 90 + k * 13, errors: 26 - k * 2, ach: 78 + k * 1.5, spend: 12 + ((k * 7) % 14), reviews: 3 + (k % 4), findings: 5 + (k % 6) };
    s.ach = Math.round(s.ach);
    const stem = `daily-${ymd(d)}_${ymd(d)}`;
    writeFileSync(join(daily, `summary-${stem}.md`), summary(d, s));
    writeFileSync(join(daily, `metrics-${stem}.json`), JSON.stringify({ combined: { stats: { total_sessions: s.sessions } } }));
    writeFileSync(join(daily, `report-${stem}.html`), report(`Daily insights — ${iso(d)}`, s));
    series.push({ period_key: `daily:${ymd(d)}_${ymd(d)}`, kind: 'daily', start: iso(d), end: iso(d), headline: { total_sessions: s.sessions, total_messages: s.turns, achievement_rate: s.ach, tool_error_total: s.errors } });
  }
  for (let w = 3; w >= 1; w--) {
    const e = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate() - (w - 1) * 7 - 1));
    const st = new Date(e.getTime() - 6 * 86400000);
    const s = { i: w, sessions: 310 + w * 23, codex: 60, turns: 900, errors: 120 - w * 10, ach: 84 - w, spend: 120 + w * 9, reviews: 22, findings: 31 };
    const stem = `weekly-${ymd(st)}_${ymd(e)}`;
    writeFileSync(join(weekly, `summary-${stem}.md`), summary(e, s).replace('(daily)', '(weekly)'));
    writeFileSync(join(weekly, `report-${stem}.html`), report(`Weekly insights — ${iso(st)}`, s));
  }
  const ledger = [
    { id: 'act-0001', opened_period: series[0].period_key, action: 'One test run per change', target_metric: 'test_runs', target_value: '≤4', opened_value: '14', latest_value: '6', status: 'improved', effort: 'M', last_checked_period: series.at(-1).period_key },
    { id: 'act-0002', opened_period: series[1].period_key, action: 'Pin the flaky clock', target_metric: 'flaky_retries', target_value: '0', opened_value: '6', latest_value: '1', status: 'improved', effort: 'S', last_checked_period: series.at(-1).period_key },
    { id: 'act-0004', opened_period: series[3].period_key, action: 'Cache CI installs', target_metric: 'ci_minutes', target_value: '<25', opened_value: '48', latest_value: '41', status: 'open', effort: 'S', last_checked_period: series.at(-1).period_key },
  ];
  writeFileSync(join(root, 'index.json'), JSON.stringify({ series, action_ledger: ledger, updated_at: new Date().toISOString().slice(0, 19) }, null, 2));
}
