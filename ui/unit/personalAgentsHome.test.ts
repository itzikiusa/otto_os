import { test } from 'node:test';
import assert from 'node:assert/strict';
import { focusAgent, currentModels, firstLine, nextRunWords, plainText, railOrder, starterPrompts } from '../src/modules/personal-agents/agentHome.ts';
import { daysOf, nextForDays, scheduleForDays, timezones, workdays } from '../src/modules/personal-agents/cadence.ts';
import { paTab } from '../src/lib/uiCommands/personalAgentsPort.ts';
import { errorText, isLostConversation, isRunHandoff, plainToolError, runNoticeOf, toolWords, workLabel } from '../src/modules/threads/runtime/timeline.ts';

test('a time reads one way everywhere: today, tomorrow, else weekday + date', () => {
  const now = new Date(2026, 10, 6, 10, 0); // Fri 6 Nov 10:00 local
  const at = (d: number, h: number) => new Date(2026, 10, d, h, 0).toISOString();
  const time = (d: number, h: number) => new Date(2026, 10, d, h, 0).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' });
  assert.equal(nextRunWords(at(6, 14), now), `Today, ${time(6, 14)}`);
  assert.equal(nextRunWords(at(7, 9), now), `Tomorrow, ${time(7, 9)}`);
  assert.ok(nextRunWords(at(9, 9), now).endsWith(`, ${time(9, 9)}`));
  assert.ok(nextRunWords(at(20, 9), now).endsWith(time(20, 9)));
  assert.equal(nextRunWords('not a time', now), '');
});

test('older settings slugs land on About, Activity or Memory', () => {
  assert.equal(paTab(undefined), 'about');
  for (const s of ['overview', 'general', 'chat', 'schedules', 'routines', 'autonomy', 'advanced']) assert.equal(paTab(s), 'about');
  for (const s of ['runs', 'history', 'activity']) assert.equal(paTab(s), 'activity');
  for (const s of ['knowledge', 'context', 'memory']) assert.equal(paTab(s), 'memory');
  assert.equal(paTab('nope'), 'about');
});

test('the rail keeps its order when an agent is paused', () => {
  const a = [{ id: 'b', name: 'Bravo', enabled: true }, { id: 'a', name: 'Alpha', enabled: false }, { id: 'm', name: 'Zed', enabled: true }];
  assert.deepEqual(railOrder(a, (id) => id === 'm').map((x) => x.id), ['m', 'a', 'b']);
});

test('routine days round-trip through the schedule and preview the next run in its timezone', () => {
  assert.deepEqual(scheduleForDays([0, 1, 2, 3, 4], '09:00'), { cadence: 'cron', expr: '0 9 * * 0,1,2,3,4' });
  assert.deepEqual(scheduleForDays([0, 1, 2, 3, 4, 5, 6], '09:00'), { cadence: 'daily', at: '09:00' });
  assert.deepEqual(daysOf({ cadence: 'cron', expr: '0 9 * * 0-4' }), { days: [0, 1, 2, 3, 4], at: '09:00' });
  assert.deepEqual(daysOf({ cadence: 'weekly', weekday: 0, at: '08:00' }), { days: [1], at: '08:00' }); // weekly counts from Monday
  assert.equal(daysOf({ cadence: 'cron', expr: '*/5 * * * *' }), null);
  const fri = new Date('2026-11-06T19:30:00Z'); // Fri 21:30 in Jerusalem
  assert.equal(nextForDays([0, 1, 2, 3, 4], '09:00', 'Asia/Jerusalem', fri)?.toISOString(), '2026-11-08T07:00:00.000Z');
  assert.equal(nextForDays([1], '09:00', 'America/New_York', fri)?.toISOString(), '2026-11-09T14:00:00.000Z');
});

test('agent chats read in words: plain summaries, tool names, starter chips', () => {
  assert.equal(plainText('# Owned routine fired\n\nRoutine **executed** as [asked](x).'), 'Owned routine fired Routine executed as asked.');
  assert.equal(toolWords('mcp__otto__otto_routine_create'), 'Created a routine');
  assert.equal(toolWords('mcp__github__list_pulls'), 'List pulls (github)');
  assert.equal(toolWords('Bash'), 'Bash');
  assert.ok(!starterPrompts(false, false).some((p) => p.startsWith('Run your routine')));
  assert.ok(starterPrompts(false, true).some((p) => p.startsWith('Run your routine')));
  // Paused with a routine: no run chip, and no "set up a routine" either.
  assert.deepEqual(starterPrompts(false, true, true), ['What can you do for me?']);
});

test('a report title is its first line only, never heading + body glued', () => {
  assert.equal(firstLine('# Owned routine fired\n\nRoutine executed as instructed.'), 'Owned routine fired');
  assert.equal(firstLine('\n\n**3 PRs** need you'), '3 PRs need you');
  assert.equal(firstLine(''), '');
  assert.equal(firstLine(null), '');
});

test('a new routine starts on the local working week', () => {
  assert.deepEqual(workdays('Asia/Jerusalem'), [0, 1, 2, 3, 4]);
  assert.deepEqual(workdays('Europe/London'), [1, 2, 3, 4, 5]);
});

test('a lost provider conversation is a calm note, and ids stay out of errors', () => {
  const err = (message: string) => ({ id: 'e', type: 'error', status: 'failed', failure: { class: 'provider', message } }) as never;
  assert.ok(isLostConversation(err('No conversation found with session ID: d45dfa17-7c72-47e2-b25a-b8cb8894bab9')));
  assert.ok(!isLostConversation(err('rate limited')));
  assert.equal(errorText('No conversation found with session ID: d45dfa17-7c72-47e2-b25a-b8cb8894bab9'), 'No conversation found');
  assert.equal(errorText('failed for 6a0b1c2d-1111-2222-3333-444455556666 today'), 'failed for … today');
});

test('a routine kept in another timezone reads in the viewer’s time, its own zone in brackets', async () => {
  const prev = process.env.TZ;
  process.env.TZ = 'Asia/Jerusalem';
  try {
    const { routineWhen } = await import('../src/modules/personal-agents/cadence.ts');
    const now = new Date('2026-11-06T10:00:00Z');
    assert.equal(routineWhen({ schedule: { cadence: 'daily', at: '20:42' }, timezone: 'UTC' }, now), 'Daily at 22:42 (20:42 UTC)');
    // In the viewer's own zone: no zone at all.
    assert.equal(routineWhen({ schedule: { cadence: 'daily', at: '09:00' }, timezone: 'Asia/Jerusalem' }, now), 'Daily at 09:00');
  } finally {
    process.env.TZ = prev;
  }
});

test('a Claude alias reads as its model name even without a live catalog', async () => {
  const { formatModelLabel } = await import('../src/lib/providerModels.ts');
  assert.equal(formatModelLabel('claude', 'opus', null), 'Claude Opus');
  assert.equal(formatModelLabel('claude', 'my-custom-id', null), 'my-custom-id');
});

test('a run-result notice carries its run id; the marker never shows', () => {
  const ok = runNoticeOf('Morning brief ran · Two PRs need review [otto-run:01M6S3XKH81K8FWZGQ2E8DK2NG]');
  assert.deepEqual(ok, { text: 'Morning brief ran · Two PRs need review', runId: '01M6S3XKH81K8FWZGQ2E8DK2NG', outcome: 'ok' });
  assert.equal(runNoticeOf('Manual run failed · the agent exited [otto-run:r1]')?.outcome, 'failed');
  // A success notice is just "<routine> ran" now (the agent's follow-up says what it found).
  assert.deepEqual(runNoticeOf('Morning brief ran [otto-run:r2]'), { text: 'Morning brief ran', runId: 'r2', outcome: 'ok' });
  assert.equal(runNoticeOf('Brief was stopped [otto-run:r1]')?.outcome, 'stopped');
  // A routine named "failed builds" that ran is still ok.
  assert.equal(runNoticeOf('Watch failed builds ran · none [otto-run:r1]')?.outcome, 'ok');
  // Old notices (no marker) and other notices stay plain.
  assert.equal(runNoticeOf('Say hi ran · Hi'), null);
  assert.equal(runNoticeOf('The earlier conversation could not be resumed'), null);
  assert.equal(
    workLabel({ id: 'n', type: 'system_notice', message: '# Brief ran · hi [otto-run:r1]' } as never),
    'Brief ran · hi',
  );
});

test('a routine report handed to its agent is hidden; other framed messages are not', () => {
  assert.ok(isRunHandoff('[Message from Scheduled · Brief (t-run)]: Otto: "Brief" ran (it is now Sat 7 Nov 09:00).\n\nTwo PRs'));
  assert.ok(isRunHandoff('[Message from Manual run (t-run)]: Otto: "Manual run" finished (it is now Sat 7 Nov 09:00).\n\nok'));
  assert.ok(!isRunHandoff('[Message from Scout (t1)]: Report from Scout: found 2 bugs'));
  assert.ok(!isRunHandoff('What did my brief say?'));
});

test('round 8: denied tool errors read in words; models list the newest per family; zones open short', () => {
  assert.equal(
    plainToolError('{"decision":"denied","executed":false,"reason":"the Otto MCP server is disabled"}'),
    'Not allowed: the Otto MCP server is disabled',
  );
  assert.equal(plainToolError('{"error":"boom"}'), 'boom');
  assert.equal(plainToolError('plain failure'), 'plain failure');
  const ids = ['claude-opus-5-5', 'claude-sonnet-5-5', 'claude-haiku-5-5', 'claude-haiku-4-5-20251001', 'claude-opus-4-8', 'claude-sonnet-4-6'];
  const m = (keep = '') => currentModels(ids.map((id) => ({ id })), keep).map((x) => x.id);
  assert.deepEqual(m(), ['claude-opus-5-5', 'claude-sonnet-5-5', 'claude-haiku-5-5']);
  assert.deepEqual(m('claude-opus-4-8'), ['claude-opus-5-5', 'claude-sonnet-5-5', 'claude-haiku-5-5', 'claude-opus-4-8']);
  const z = timezones('Europe/Paris');
  assert.equal(z.top[0], 'Europe/Paris');
  assert.ok(z.top.includes('UTC') && z.top.length < 20);
  assert.ok(!z.rest.some((x) => z.top.includes(x)));
  // The long list is grouped by region, every zone once, regions sorted.
  assert.equal(z.regions.flatMap(([, zs]) => zs).length, z.rest.length);
  assert.ok(z.regions.every(([r, zs]) => zs.every((x) => x.startsWith(`${r}/`) || (r === 'Other' && !x.includes('/')))));
  assert.deepEqual(z.regions.map(([r]) => r), [...z.regions.map(([r]) => r)].sort((a, b) => a.localeCompare(b)));
});

test('a routine name round-trips as the directive’s first line', async () => {
  const { joinDirective, splitDirective } = await import('../src/modules/personal-agents/agentHome.ts');
  assert.equal(joinDirective('Morning PR brief', 'List my open PRs.'), 'Morning PR brief\n\nList my open PRs.');
  assert.equal(joinDirective('  ', 'List my open PRs.'), 'List my open PRs.');
  assert.deepEqual(splitDirective('Morning PR brief\n\nList my open PRs.'), { name: 'Morning PR brief', instruction: 'List my open PRs.' });
  // No blank line after the first line: no name.
  assert.deepEqual(splitDirective('Brief me on my PRs\nand failing CI'), { name: '', instruction: 'Brief me on my PRs\nand failing CI' });
});

test('a run reads as what it found, not its report heading', async () => {
  const { leadLine } = await import('../src/lib/plainText.ts');
  assert.equal(leadLine('# Late Brief — Sat 7 Nov\n\nYou have 6 routines; two overlap.'), 'You have 6 routines; two overlap.');
  assert.equal(leadLine('# Only a heading'), 'Only a heading');
  assert.equal(leadLine('Plain **first** line\nmore'), 'Plain first line');
});


test('Focus opens the primary agent and keeps explicit agent deep links', () => {
  const agents = [{ id: 'scout', name: 'Alpha' }, { id: 'main', name: 'Otto' }];
  assert.equal(focusAgent(agents, '', 'main')?.id, 'main');
  assert.equal(focusAgent(agents, 'scout', 'main')?.id, 'scout');
  assert.equal(focusAgent(agents, 'deleted', 'main'), null);
  assert.equal(focusAgent([], '', null), null);
  assert.equal(focusAgent(agents, '', null)?.id, 'scout');
});
