// The Threads sidebar inbox (threadInbox.ts): the words a row shows, the
// Working fold, and the main list's order.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import type { Session } from '../src/lib/api/types';

const hook =
  'export async function resolve(s, c, n) {' +
  "  if (/^\\.{1,2}\\//.test(s) && !/\\.[a-z]+$/i.test(s)) { try { return await n(s + '.ts', c); } catch {} }" +
  '  return n(s, c);' +
  '}';
register('data:text/javascript,' + encodeURIComponent(hook));

const { inboxClass, inboxSections, statusWord, waitingWord, elapsed } = await import('../src/modules/threads/threadInbox.ts');

const s = (id: string, h: number, parent?: string): Session =>
  ({ id, title: id, last_active_at: `2026-11-23T${String(h).padStart(2, '0')}:00:00Z`, meta: parent ? { parent_session_id: parent } : {} }) as unknown as Session;
const parentOf = (x: Session): string | null => ((x.meta as Record<string, unknown>).parent_session_id as string | undefined) ?? null;

test('a state and the unread flag give the class', () => {
  assert.equal(inboxClass('needs-you', true), 'needs');
  assert.equal(inboxClass('failed', false), 'failed');
  for (const k of ['working', 'stale']) assert.equal(inboxClass(k, true), 'working');
  // Idle with background work running: calm, not Working (D-06).
  assert.equal(inboxClass('running', false), 'rest');
  assert.equal(inboxClass('running', true), 'done');
  assert.equal(inboxClass('idle', true), 'done');
  assert.equal(inboxClass('stopped', false), 'rest');
});

test('the status word vocabulary', () => {
  assert.equal(statusWord('needs', { kinds: ['command'] }), 'Approval');
  assert.equal(statusWord('needs', { kinds: ['question'] }), 'Question');
  assert.equal(statusWord('needs', {}), 'Approval');
  assert.equal(statusWord('failed'), 'Failed');
  assert.equal(statusWord('done'), 'Done');
  assert.equal(statusWord('working'), 'Working');
  assert.equal(statusWord('working', { workingSince: 1000, now: 19_500 }), 'Working 18s');
  assert.equal(statusWord('rest'), null);
  assert.equal(waitingWord(['question', 'mcp_elicitation']), 'Question');
  assert.equal(waitingWord(['question', 'file_change']), 'Approval');
  assert.equal(elapsed(125_000), '2m');
  assert.equal(elapsed(3_900_000), '1h 5m');
});

test('needs you, then failed, then done, then most recent; working folds away', () => {
  const cls: Record<string, string> = { old: 'rest', recent: 'rest', done: 'done', fail: 'failed', ask: 'needs', run: 'working' };
  const list = [s('old', 1), s('recent', 9), s('done', 2), s('fail', 3), s('ask', 4), s('run', 10)];
  const { main, working } = inboxSections(list, (x) => cls[x.id] as never, parentOf);
  assert.deepEqual(main.map((r) => r.session.id), ['ask', 'fail', 'done', 'recent', 'old']);
  assert.deepEqual(working.map((r) => r.session.id), ['run']);
});

test('subagents: their own words and the roll-up the parent shows', async () => {
  const { subagentWord } = await import('../src/modules/threads/threadInbox.ts');
  const { childRollup } = await import('../src/modules/threads/runtime/childThreads.ts');
  const subagentRollup = (subs: { title: string; key: string }[]) => childRollup(subs.map((s) => ({ title: s.title, word: subagentWord(s.key) })));
  assert.deepEqual(['needs-you', 'working', 'running', 'failed', 'stopped', 'idle'].map(subagentWord), ['Needs you', 'Working', 'Working', 'Failed', 'Stopped', 'Done']);
  assert.deepEqual(
    subagentRollup([{ title: 'tests', key: 'working' }, { title: 'api', key: 'needs-you' }, { title: 'docs', key: 'failed' }, { title: 'lint', key: 'idle' }]),
    { total: 4, working: 1, needsYou: 1, failed: 1, needing: 'api' },
  );
});

test('a family moves together: a working lead with a waiting worker stays in the main list', () => {
  const cls: Record<string, string> = { lead: 'working', kid: 'needs', lead2: 'working', kid2: 'rest' };
  const list = [s('lead', 5), s('kid', 4, 'lead'), s('lead2', 6), s('kid2', 3, 'lead2')];
  const { main, working } = inboxSections(list, (x) => cls[x.id] as never, parentOf);
  assert.deepEqual(main.map((r) => [r.session.id, r.child]), [['lead', false], ['kid', true]]);
  assert.deepEqual(working.map((r) => [r.session.id, r.child]), [['lead2', false], ['kid2', true]]);
  const folded = inboxSections(list, (x) => cls[x.id] as never, parentOf, (id) => id === 'lead');
  assert.deepEqual(folded.main.map((r) => r.session.id), ['lead']);
  assert.deepEqual(folded.main[0].kids, ['kid']);
});
