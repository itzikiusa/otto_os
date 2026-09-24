import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  delta,
  extractMetrics,
  firstSentence,
  indexPathFrom,
  itemStatus,
  parseActionItem,
  parseIndex,
  parseSummary,
  periodKey,
  reportMetrics,
  siblingPath,
  statusTone,
} from '../src/modules/insights/insightsParse.ts';

// Synthetic samples in the SHAPE the bundled `insights` skill writes
// (summary-<kind>-<start>_<end>.md): an H1, a headline paragraph, a few short
// paragraphs, then "## Action Plan (carried into the ledger)" with numbered
// items. The wording drifts run to run — these cover the variants seen.

const DAILY = `# Insights summary — Wed 23 Sep 2026 (daily)

Wednesday was a busy build day: 173 Claude sessions (0 Codex, 0 agy), 268 user turns of which ~5 threads were human, 2,197 tool calls, 56 tool errors and 14 commits. Achievement rate held at 94% across the classified sessions.
Reflections regressed: 36 runs vs the ≤5 cap, 92% with zero tool calls.
Spend was $41.20 across all providers.

## Action Plan (carried into the ledger)

1. **Reflection trigger: hard cap 5/day, session-id dedupe** — runs / zero-tool: 36, 92% → ≤5, <20% — effort S — act-20260906-01 + act-20260921-01 (regressed)
2. **Size-tiered quick-review workflow** — lens sessions per small-PR run: ~5.7 → ≤3 — effort M — new act-20260923-01
3. **Review-context prelude once per change** — lens Bash / lens cd-share: 83.5%, 52.9% → <65%, <10% — effort M — act-20260914-01 + act-20260910-01 (improved)
4. **Raise \`--emit-cap\` to 180 for daily runs** — facet coverage: 23% (40 of 173) → 100% — effort S — act-20260903-02 (regressed)
`;

const WEEKLY_DRIFT = `# Insights summary — week of 14 Sep 2026 (weekly)

81 sessions and 81 messages this week; e.g. the review lenses dominated. No prior period to compare.

## Action Plan

1. **Tag the automated reflection sessions** — targets human-cohort overlapping messages: 96% (all-session) → <30% — effort S — new
2. **Hold a weekly dedupe-and-merge pass** — targets merged skill edits: 0 merged → ≤5 merged edits/week — effort M — carried, regressed
3. **Hoist the repeated build probes** — targets Bash share of tool calls: 87.3% (315/361) → <65% — effort M — carried, improved 9.0pp
4. Replace sleep waits with Monitor until-loops -- targets blocked retries: 2 → 0 -- effort S -- new
`;

test('parses the title, headline and headline metrics of a daily summary', () => {
  const p = parseSummary(DAILY);
  assert.equal(p.title, 'Insights summary — Wed 23 Sep 2026 (daily)');
  assert.match(p.headline ?? '', /^Wednesday was a busy build day: 173 Claude sessions/);
  assert.ok((p.headline ?? '').endsWith('14 commits.'));
  assert.deepEqual(p.metrics, { sessions: 173, turns: 268, toolErrors: 56, spend: 41.2, achievement: 94 });
});

test('parses action items: title, metric, current → target, effort, ids and status', () => {
  const p = parseSummary(DAILY);
  assert.equal(p.actions.length, 4);
  const [a1, a2, a3, a4] = p.actions;
  assert.equal(a1.title, 'Reflection trigger: hard cap 5/day, session-id dedupe');
  assert.equal(a1.metric, 'runs / zero-tool');
  assert.equal(a1.current, '36, 92%');
  assert.equal(a1.target, '≤5, <20%');
  assert.equal(a1.effort, 'S');
  assert.deepEqual(a1.ids, ['act-20260906-01', 'act-20260921-01']);
  assert.deepEqual(a1.statuses, ['regressed']);
  assert.deepEqual(a2.statuses, ['new']);
  assert.equal(a2.effort, 'M');
  assert.equal(a2.current, '~5.7');
  assert.deepEqual(a3.statuses, ['improved']);
  assert.equal(a4.title, 'Raise --emit-cap to 180 for daily runs');
  assert.equal(a4.metric, 'facet coverage');
});

test('the Preview body drops the H1 and the parsed Action Plan', () => {
  const p = parseSummary(DAILY);
  assert.ok(!p.body.includes('# Insights summary'));
  assert.ok(!p.body.includes('Action Plan'));
  assert.ok(p.body.includes('Reflections regressed'));
  // The headline sentence isn't repeated at the top of the body…
  assert.ok(!p.body.includes('Wednesday was a busy build day'));
  // …but the rest of its paragraph is kept.
  assert.ok(p.body.startsWith('Achievement rate held at 94%'));
});

test('tolerates format drift: "targets", tag words without ids, "--" separators, no bold', () => {
  const p = parseSummary(WEEKLY_DRIFT);
  assert.equal(p.actions.length, 4);
  assert.equal(p.actions[0].metric, 'human-cohort overlapping messages');
  assert.deepEqual(p.actions[0].ids, []);
  assert.deepEqual(p.actions[1].statuses, ['carried', 'regressed']);
  assert.deepEqual(p.actions[2].statuses, ['carried', 'improved']);
  assert.equal(p.actions[3].title, 'Replace sleep waits with Monitor until-loops');
  assert.equal(p.actions[3].effort, 'S');
  assert.equal(p.actions[3].current, '2');
  assert.equal(p.actions[3].target, '0');
  // "e.g." does not end the headline sentence.
  assert.match(p.headline ?? '', /e\.g\. the review lenses dominated\.$/);
  assert.equal(p.metrics.sessions, 81);
  assert.equal(p.metrics.turns, 81);
  assert.equal(p.metrics.toolErrors, undefined);
});

test('falls back cleanly: no H1, no action plan, empty and garbage input', () => {
  const plain = parseSummary('Just a paragraph with 3 sessions.\n\n- a bullet');
  assert.equal(plain.title, null);
  assert.equal(plain.actions.length, 0);
  assert.equal(plain.metrics.sessions, 3);
  assert.ok(plain.body.includes('- a bullet'));

  const empty = parseSummary('');
  assert.deepEqual(empty, { title: null, headline: null, metrics: {}, actions: [], body: '' });
  assert.equal(parseSummary(undefined).actions.length, 0);

  // An Action Plan heading with prose instead of a list keeps the section in the body.
  const prose = parseSummary('# T\n\nText.\n\n## Action Plan\n\nNothing to do this week.');
  assert.equal(prose.actions.length, 0);
  assert.ok(prose.body.includes('## Action Plan'));
  assert.ok(prose.body.includes('Nothing to do this week.'));
});

test('metrics ignore action-plan numbers and bare $ rates', () => {
  assert.deepEqual(extractMetrics('Keep it under $2/day. The cap was 5 runs.'), {});
  assert.equal(extractMetrics('It cost $3.10 today').spend, 3.1);
  assert.equal(extractMetrics('tool errors: 12').toolErrors, 12);
  assert.equal(extractMetrics('82% achievement across 10 sessions').achievement, 82);
  assert.equal(extractMetrics('achievement rate 140%').achievement, undefined);
});

test('firstSentence keeps decimals and abbreviations intact', () => {
  assert.equal(firstSentence('Rose 9.0pp vs. last week. Then more.'), 'Rose 9.0pp vs. last week.');
  assert.equal(firstSentence('No full stop here'), 'No full stop here');
});

test('parseActionItem with no separators yields just a title', () => {
  const a = parseActionItem('**Write the FAQ note**', 1);
  assert.equal(a.title, 'Write the FAQ note');
  assert.equal(a.metric, null);
  assert.equal(a.effort, null);
  assert.deepEqual(a.statuses, []);
});

test('parseIndex reads series + ledger and survives junk', () => {
  const idx = parseIndex({
    series: [
      { period_key: 'daily:20260923_20260923', kind: 'daily', start: '2026-09-23', end: '2026-09-23', headline: { total_sessions: 173, total_messages: 268, achievement_rate: 100, tool_error_total: 0 } },
      null,
      'junk',
    ],
    action_ledger: [
      { id: 'act-20260906-01', action: 'Cap reflections', status: 'OPEN', target_metric: 'runs', target_value: '≤5', opened_value: '28', latest_value: '36', effort: 'S' },
      { action: 'no id — dropped' },
    ],
  });
  assert.equal(idx.series.length, 1);
  assert.deepEqual(idx.series[0].metrics, { sessions: 173, turns: 268, toolErrors: 0, achievement: 100 });
  assert.equal(idx.ledger.size, 1);
  assert.equal(idx.ledger.get('act-20260906-01')?.status, 'open');
  assert.deepEqual(parseIndex(null).series, []);
  assert.deepEqual(parseIndex({ series: 'nope' }).series, []);
});

test('reportMetrics prefers the summary, then the index row', () => {
  const idx = parseIndex({ series: [{ period_key: 'k', headline: { total_sessions: 170, tool_error_total: 0, achievement_rate: 100 } }] });
  const { values, source } = reportMetrics({ sessions: 173, toolErrors: 56 }, idx.series[0]);
  assert.deepEqual(values, { sessions: 173, toolErrors: 56, achievement: 100 });
  assert.equal(source.toolErrors, 'summary');
  assert.equal(source.achievement, 'index');
});

test('delta tones follow whether up is good for the metric', () => {
  assert.equal(delta('toolErrors', 10, 20)?.tone, 'good');
  assert.equal(delta('toolErrors', 30, 20)?.tone, 'bad');
  assert.equal(delta('achievement', 90, 80)?.tone, 'good');
  assert.equal(delta('sessions', 90, 80)?.tone, 'neutral');
  assert.equal(delta('spend', 5, 5)?.direction, 'flat');
  assert.equal(delta('spend', 5, 0)?.pct, null);
  assert.equal(delta('spend', undefined, 3), null);
});

test('item status: ledger + summary words, regressed wins', () => {
  const p = parseSummary(DAILY);
  const ledger = parseIndex({ action_ledger: [{ id: 'act-20260923-01', status: 'closed' }] }).ledger;
  assert.equal(itemStatus(p.actions[0], ledger), 'regressed');
  assert.equal(itemStatus(p.actions[1], ledger), 'closed');
  assert.equal(itemStatus(parseActionItem('**x**', 1)), null);
  assert.equal(statusTone('regressed'), 'warning');
  assert.equal(statusTone('improved'), 'success');
  assert.equal(statusTone('new'), 'info');
  assert.equal(statusTone('carried'), 'neutral');
});

test('period keys and sibling artifact paths', () => {
  assert.equal(periodKey({ kind: 'weekly', period_start: '2026-09-14', period_end: '2026-09-20' }), 'weekly:20260914_20260920');
  const html = '/data/insights/daily/report-daily-20260923_20260923.html';
  assert.equal(siblingPath(html, 'summary'), '/data/insights/daily/summary-daily-20260923_20260923.md');
  assert.equal(siblingPath(html, 'metrics'), '/data/insights/daily/metrics-daily-20260923_20260923.json');
  assert.equal(indexPathFrom(html), '/data/insights/index.json');
  assert.equal(siblingPath('/x/other.html', 'summary'), null);
  assert.equal(indexPathFrom('report.html'), null);
});
