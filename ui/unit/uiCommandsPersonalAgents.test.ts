// Agent UI control — Personal Agents (`pa_*`): the pure confirm/argument
// helpers, plus structural pins on the handler file (the Slack act-as-me
// grant is ALWAYS the outward confirm, never the rememberable write confirm).

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import {
  agentChanges,
  cadenceText,
  checkCadence,
  checkPermission,
  createConfirmText,
  PaArgError,
  personaSummary,
  slackAsMeConfirm,
} from '../src/lib/uiCommands/personalAgentsPlan.ts';

const HANDLERS = readFileSync(fileURLToPath(new URL('../src/lib/uiCommands/personalAgents.ts', import.meta.url)), 'utf8');
const CATALOG = JSON.parse(readFileSync(fileURLToPath(new URL('../../docs/contracts/ui-commands.json', import.meta.url)), 'utf8')) as {
  commands: { name: string; risk: string; module: string; route: string }[];
};

/** The body of one handler (`async name(` … up to the next handler). */
function handlerBody(name: string): string {
  const start = HANDLERS.indexOf(`async ${name}(`);
  assert.ok(start >= 0, `${name} handler not found`);
  const next = HANDLERS.indexOf('\n  async pa_', start + 1);
  return HANDLERS.slice(start, next < 0 ? undefined : next);
}

test('cadence + permission checks reject malformed schedules', () => {
  assert.deepEqual(checkCadence({ cadence: 'cron', expr: '0 7 * * 0-5' }), { cadence: 'cron', expr: '0 7 * * 0-5' });
  assert.throws(() => checkCadence({ cadence: 'cron', expr: '0 7 *' }), PaArgError);
  assert.throws(() => checkCadence({ cadence: 'hourly' }), PaArgError);
  assert.throws(() => checkCadence('daily'), PaArgError);
  assert.throws(() => checkCadence({ cadence: 'once' }), PaArgError);
  assert.equal(checkPermission(undefined), undefined);
  assert.equal(checkPermission('read_only'), 'read_only');
  assert.throws(() => checkPermission('admin'), PaArgError);
  assert.equal(cadenceText({ cadence: 'cron', expr: '0 7 * * 0-5' }, 'Asia/Jerusalem'), 'cron 0 7 * * 0-5 (Asia/Jerusalem)');
});

test('the create confirm names the template, the edited persona and the schedule', () => {
  const t = { title: 'Slack daily digest (as me)', name: 'Slack digest', soul_md: '<!-- m -->\n# Slack daily digest\nbody', directive: 'Write my digest', schedule: { cadence: 'daily', at: '18:00' } };
  const text = createConfirmText(
    {
      template: 'slack-daily-digest',
      name: 'Slack helper',
      soul_md: '# Slack helper\nMine now',
      provider: 'claude',
      schedule: { schedule: { cadence: 'cron', expr: '0 7 * * 0-5' }, timezone: 'Asia/Jerusalem', permission: 'directed' },
    },
    t,
    'UTC',
  );
  assert.match(text, /“Slack helper” from the template “Slack daily digest \(as me\)”/);
  assert.match(text, /Persona: “Slack helper” \(\d+ characters\) — edited from the template/);
  assert.match(text, /Provider: claude/);
  assert.match(text, /cron 0 7 \* \* 0-5 \(Asia\/Jerusalem\); permission directed/);
  assert.match(text, /directive “Write my digest”/, 'an empty directive falls back to the template’s');
  // No override: the template's own schedule, in the user's timezone.
  assert.match(createConfirmText({ template: 'x' }, t, 'Europe/London'), /daily at 18:00 \(Europe\/London\)/);
  assert.match(createConfirmText({ name: 'Blank' }, undefined, 'UTC'), /No schedule/);
});

test('agent edits list only the fields that change, persona summarized', () => {
  const agent = { name: 'A', avatar: '', soul_md: '# Old', provider: 'claude', model: '', enabled: true };
  const same = agentChanges(agent, { name: 'A', provider: 'claude', enabled: true });
  assert.deepEqual(same, { body: {}, lines: [] });
  const { body, lines } = agentChanges(agent, { name: 'B', soul_md: '# New persona', enabled: false });
  assert.deepEqual(body, { name: 'B', soul_md: '# New persona', enabled: false });
  assert.equal(lines.length, 3);
  assert.match(lines.join('\n'), /persona: “Old” .* → “New persona”/);
  assert.match(personaSummary(''), /no persona/);
});

test('the Slack act-as-me confirm says where, what and who', () => {
  const on = slackAsMeConfirm('Slack helper', 'Claude · s1', [{ team: 'Acme', user: 'itzik' }]);
  assert.match(on.where, /Slack: Acme \(as itzik\)/);
  assert.match(on.what ?? '', /AS YOU/);
  assert.match(on.who ?? '', /under your name/);
  assert.equal(on.danger, true);
  const off = slackAsMeConfirm('Slack helper', 'Claude · s1', []);
  assert.equal(off.verb, 'Turn off');
});

test('pa_set_slack_as_me always uses the outward confirm (never the rememberable one)', () => {
  const body = handlerBody('pa_set_slack_as_me');
  assert.match(body, /confirmOutward\(slackAsMeConfirm\(/);
  assert.doesNotMatch(body, /confirmWrite/);
  const spec = CATALOG.commands.find((c) => c.name === 'pa_set_slack_as_me');
  assert.equal(spec?.risk, 'outward');
});

test('pa_create fills the sheet, confirms, and only then submits it', () => {
  const body = handlerBody('pa_create');
  const fill = body.indexOf('sheet.fill(');
  const confirm = body.indexOf('ctx.confirmWrite(');
  const submit = body.indexOf('sheet.submit()');
  assert.ok(fill > 0 && confirm > fill && submit > confirm, 'fill → confirm → submit');
  assert.match(body.slice(confirm, submit), /sheet\.close\(\)/, 'a declined confirm closes the sheet');
});

test('every pa_* command is a Personal Agents route on the scheduled-tasks pane', () => {
  const pa = CATALOG.commands.filter((c) => c.name.startsWith('pa_'));
  assert.equal(pa.length, 10);
  for (const c of pa) {
    assert.equal(c.module, 'scheduled-tasks', c.name);
    assert.equal(c.route, 'personal-agents', c.name);
  }
});
