import { test } from 'node:test';
import assert from 'node:assert/strict';
import { cadenceLabel, cronHuman, everyLabel } from '../src/lib/cadence.ts';
import { loadSource } from './sourceHarness.ts';

// ux-design-2-04: one humane cadence vocabulary across scheduled tasks,
// personal agents and workflow triggers.
test('cadenceLabel reads cadences as people say them', () => {
  assert.equal(cadenceLabel({ cadence: 'interval', every_min: 1440 }, 'UTC'), 'Every day');
  assert.equal(cadenceLabel({ cadence: 'interval', every_min: 120 }, 'UTC'), 'Every 2 hours');
  assert.equal(cadenceLabel({ cadence: 'interval', every_min: 15 }, 'UTC'), 'Every 15 min');
  assert.equal(everyLabel(2880), 'Every 2 days');
  assert.equal(cadenceLabel({ cadence: 'daily', at: '03:00' }, 'Europe/London'), 'Daily at 03:00 · Europe/London');
  assert.equal(cadenceLabel({ cadence: 'weekly', at: '09:00', weekday: 0 }, ''), 'Mondays at 09:00 · UTC');
  assert.equal(cadenceLabel({ cadence: 'cron', expr: '0 9 * * 1' }, 'UTC'), 'Cron 0 9 * * 1 · UTC');
  assert.equal(cadenceLabel({ cadence: 'cron', expr: '0 9 * * 1' }, 'UTC', { cronExpr: false }), 'Cron');
  assert.equal(cadenceLabel({ cadence: 'once', run_at: '2026-10-22T09:30' }, 'UTC'), 'Once at 2026-10-22 09:30 · UTC');
  assert.doesNotMatch(cadenceLabel({ cadence: 'interval', every_min: 1440 }, 'UTC'), /1440/);
});

test('cronHuman says a common cron in words and refuses the unusual', () => {
  assert.equal(cronHuman('0 9 * * 0,1,2,3,4'), 'Sun–Thu at 09:00');
  assert.equal(cronHuman('0 9 * * 0-4'), 'Sun–Thu at 09:00');
  assert.equal(cronHuman('15 10 * * 1-5'), 'Weekdays at 10:15');
  assert.equal(cronHuman('0 8 * * 5,6,0'), 'Fri–Sun at 08:00');
  assert.equal(cronHuman('0 10 * * 6,7'), 'Weekends at 10:00');
  assert.equal(cronHuman('30 7 * * *'), 'Every day at 07:30');
  assert.equal(cronHuman('0 9 * * 1'), 'Mondays at 09:00');
  assert.equal(cronHuman('0 9 * * 1,3,5'), 'Mon, Wed, Fri at 09:00');
  assert.equal(cronHuman('0 8 1 * *'), 'Monthly on day 1 at 08:00');
  assert.equal(cronHuman('5 * * * *'), 'Every hour at :05');
  for (const odd of ['*/5 * * * *', '0 9 * 1 *', '0 9-17 * * *', '0 9 1 * 1', 'nope', '0 25 * * *']) {
    assert.equal(cronHuman(odd), null, odd);
  }
  assert.equal(cadenceLabel({ cadence: 'cron', expr: '0 9 * * 0-4' }, 'Asia/Jerusalem', { cronWords: true }), 'Sun–Thu at 09:00 · Asia/Jerusalem');
  assert.equal(cadenceLabel({ cadence: 'cron', expr: '*/5 * * * *' }, 'UTC', { cronWords: true }), 'Cron */5 * * * * · UTC');
});

function outward() {
  const asked: { message: string; opts: Record<string, unknown> }[] = [];
  const mod = loadSource(new URL('../src/lib/confirmOutward.ts', import.meta.url), {
    './confirm.svelte': {
      confirmer: { ask: async (message: string, opts: Record<string, unknown>) => { asked.push({ message, opts }); return true; } },
    },
  });
  return { mod, asked };
}

// ux-design-2-01: a run that delivers says where, what and who first.
test('confirmRunAndDeliver asks only when the report leaves Otto', async () => {
  const { mod, asked } = outward();
  assert.equal(await mod.confirmRunAndDeliver('Scout', { type: 'none' }), true);
  assert.equal(await mod.confirmRunAndDeliver('Scout', null), true);
  assert.equal(asked.length, 0);
  assert.equal(await mod.confirmRunAndDeliver('Scout', { type: 'slack', chat_id: 'C42' }), true);
  assert.equal(asked.length, 1);
  assert.equal(asked[0].opts.confirmLabel, 'Run and deliver');
  assert.match(asked[0].message, /Where: the Slack channel C42/);
  assert.match(asked[0].message, /The report of “Scout”/);
  assert.match(asked[0].message, /Who sees it: Everyone in that Slack channel\./);
  assert.equal(mod.destWho('email'), mod.destWho({ type: 'email' }));
  assert.equal(mod.savedDestWhere({ type: 'webhook', url: 'https://x.test' }), 'https://x.test');
});
