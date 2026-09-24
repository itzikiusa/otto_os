import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

// Seed Insights reports straight into the ISOLATED test daemon's data dir
// (`<OTTO_DATA_DIR>/insights/<kind>/…`, docs/features/insights.md §5). There is
// no API to create a report — the `insights` skill writes these files — so the
// spec writes the same triple the skill would (report HTML, summary markdown,
// metrics JSON) plus the rolling index.json. All content is synthetic.

export function daemonDataDir(): string {
  const slot = process.env.OTTO_E2E_SLOT ?? '0';
  const meta = JSON.parse(readFileSync(join(process.cwd(), 'e2e', `.auth-${slot}`, 'daemon.json'), 'utf8')) as {
    dataDir: string;
  };
  return meta.dataDir;
}

interface Day {
  date: string; // YYYY-MM-DD
  sessions: number;
  turns: number;
  errors: number;
  achievement: number;
  spend: number;
}

const DAYS: Day[] = [
  { date: '2026-09-14', sessions: 64, turns: 142, errors: 31, achievement: 78, spend: 18.4 },
  { date: '2026-09-15', sessions: 71, turns: 150, errors: 28, achievement: 81, spend: 21.1 },
  { date: '2026-09-16', sessions: 58, turns: 120, errors: 35, achievement: 74, spend: 16.9 },
  { date: '2026-09-17', sessions: 90, turns: 201, errors: 22, achievement: 85, spend: 27.3 },
  { date: '2026-09-18', sessions: 84, turns: 188, errors: 19, achievement: 88, spend: 25.0 },
  { date: '2026-09-19', sessions: 40, turns: 76, errors: 9, achievement: 90, spend: 9.8 },
  { date: '2026-09-20', sessions: 33, turns: 61, errors: 7, achievement: 92, spend: 7.2 },
  { date: '2026-09-21', sessions: 102, turns: 230, errors: 26, achievement: 86, spend: 31.6 },
  { date: '2026-09-22', sessions: 118, turns: 251, errors: 41, achievement: 83, spend: 36.0 },
  { date: '2026-09-23', sessions: 131, turns: 268, errors: 24, achievement: 91, spend: 33.4 },
];

const ymd = (d: string) => d.replace(/-/g, '');
const weekday = (d: string) =>
  new Date(`${d}T00:00:00Z`).toLocaleDateString('en-GB', { weekday: 'short', day: '2-digit', month: 'short', year: 'numeric', timeZone: 'UTC' });

function dailySummary(d: Day, i: number): string {
  const regressedFirst = d.errors > 30;
  return `# Insights summary — ${weekday(d.date)} (daily)

A ${d.sessions > 100 ? 'heavy' : 'steady'} build day: ${d.sessions} Claude sessions (0 Codex, 0 agy), ${d.turns} user turns, ${d.turns * 8} tool calls and ${d.errors} tool errors. Achievement rate was ${d.achievement}% across the classified sessions, and spend was $${d.spend.toFixed(2)} for the day.
Review workflows ran ${3 + (i % 4)} times; the size-tiered quick review kept small PRs to ${2 + (i % 2)} lenses.
Most tool errors came from repeated \`cd\` + build probes inside lens sessions — ${Math.round(d.errors * 0.6)} of ${d.errors}.

| Area | Sessions | Share |
|---|---|---|
| Code review | ${Math.round(d.sessions * 0.55)} | 55% |
| Feature work | ${Math.round(d.sessions * 0.3)} | 30% |
| Ops | ${Math.round(d.sessions * 0.15)} | 15% |

## Action Plan (carried into the ledger)

1. **Hoist build probes into one review-context prelude per change** — lens Bash share: ${60 + (i % 20)}% → <45% — effort M — act-20260914-01 (${regressedFirst ? 'regressed' : 'improved'})
2. **Cap reflection runs at 5 per day with a session-id dedupe** — runs / zero-tool: ${12 + i}, 80% → ≤5, <20% — effort S — act-20260915-02 (regressed)
3. **Size-tiered quick review: small PRs get gates + 2 lenses** — lens sessions per small-PR run: ~${(5.7 - i * 0.2).toFixed(1)} → ≤3 — effort M — new act-${ymd(d.date)}-01
4. **Count is_error tool results in the collector** — collector vs transcript errors: 0 vs ${d.errors} → match the transcript — effort S — act-20260916-01 (improved)
`;
}

function weeklySummary(start: string, end: string, sessions: number, errors: number, ach: number): string {
  return `# Insights summary — week of ${weekday(start)} (weekly)

${sessions} sessions and ${sessions * 2} messages this week, ${errors} tool errors; achievement rate ${ach}%.
The review lenses dominated the week; human-driven sessions were a small share.

## Action Plan

1. **Tag automated reflection sessions** — targets human-cohort overlap: 96% → <30% — effort S — new
2. **Weekly dedupe-and-merge pass over skill edits** — targets merged edits: 0 → ≤5/week — effort M — carried, regressed
3. **Replace sleep waits with until-loops** — targets blocked retries: 2 → 0 — effort S — carried, improved
`;
}

function reportHtml(title: string, d: { sessions: number; errors: number; achievement: number }): string {
  return `<!doctype html><html><head><meta charset="utf-8"><title>${title}</title>
<style>body{font:14px/1.5 -apple-system,system-ui,sans-serif;margin:24px;color:#1d1d1f;background:#fff}
.glance{background:#fff8e1;border:1px solid #f1d38a;border-radius:8px;padding:12px 16px}
.bar{height:10px;background:#4f7cff;border-radius:4px;margin:4px 0}</style></head>
<body><h1>${title}</h1><div class="glance"><strong>At a glance</strong><p>${d.sessions} sessions · ${d.errors} tool errors · ${d.achievement}% achieved</p></div>
<h2>Tool errors by hour</h2><div id="chart"></div>
<script>for(let i=0;i<6;i++){const b=document.createElement('div');b.className='bar';b.style.width=(20+i*12)+'%';document.getElementById('chart').appendChild(b);}
try{localStorage.getItem('otto_token');document.body.dataset.storage='reachable'}catch(e){document.body.dataset.storage='blocked'}</script>
</body></html>`;
}

/** Write 10 daily + 3 weekly reports and an index.json. Idempotent. */
export function seedInsights(dataDir = daemonDataDir()): void {
  const root = join(dataDir, 'insights');
  const daily = join(root, 'daily');
  const weekly = join(root, 'weekly');
  mkdirSync(daily, { recursive: true });
  mkdirSync(weekly, { recursive: true });
  const series: unknown[] = [];
  DAYS.forEach((d, i) => {
    const stem = `daily-${ymd(d.date)}_${ymd(d.date)}`;
    writeFileSync(join(daily, `summary-${stem}.md`), dailySummary(d, i));
    writeFileSync(join(daily, `metrics-${stem}.json`), JSON.stringify({ combined: { stats: { total_sessions: d.sessions } } }));
    // The oldest day has no HTML yet (a run still writing) — exercises the fallback.
    if (i > 0) writeFileSync(join(daily, `report-${stem}.html`), reportHtml(`Daily insights — ${d.date}`, d));
    series.push({
      period_key: `daily:${ymd(d.date)}_${ymd(d.date)}`,
      kind: 'daily',
      start: d.date,
      end: d.date,
      // The collector under-counts errors (0) — the summary's number must win.
      headline: { total_sessions: d.sessions, total_messages: d.turns, achievement_rate: d.achievement, tool_error_total: 0 },
    });
  });
  const weeks: [string, string, number, number, number][] = [
    ['2026-08-31', '2026-09-06', 402, 180, 79],
    ['2026-09-07', '2026-09-13', 455, 162, 82],
    ['2026-09-14', '2026-09-20', 440, 151, 84],
  ];
  for (const [s, e, sessions, errors, ach] of weeks) {
    const stem = `weekly-${ymd(s)}_${ymd(e)}`;
    writeFileSync(join(weekly, `summary-${stem}.md`), weeklySummary(s, e, sessions, errors, ach));
    writeFileSync(join(weekly, `report-${stem}.html`), reportHtml(`Weekly insights — ${s}`, { sessions, errors, achievement: ach }));
  }
  const ledger = [
    { id: 'act-20260914-01', opened_period: 'daily:20260914_20260914', action: 'Review-context prelude', target_metric: 'lens_bash_share', target_value: '<45%', opened_value: '83.5%', latest_value: '61%', status: 'improved', effort: 'M', last_checked_period: 'daily:20260923_20260923' },
    { id: 'act-20260915-02', opened_period: 'daily:20260915_20260915', action: 'Cap reflection runs', target_metric: 'reflection_runs', target_value: '≤5/day', opened_value: '28', latest_value: '21', status: 'open', effort: 'S', last_checked_period: 'daily:20260923_20260923' },
    { id: 'act-20260916-01', opened_period: 'daily:20260916_20260916', action: 'Collector counts is_error', target_metric: 'collector_parity', target_value: 'match', opened_value: '0 vs 35', latest_value: '0 vs 24', status: 'open', effort: 'S', last_checked_period: 'daily:20260923_20260923' },
  ];
  writeFileSync(join(root, 'index.json'), JSON.stringify({ series, action_ledger: ledger, updated_at: '2026-09-24T07:00:00' }, null, 2));
}
