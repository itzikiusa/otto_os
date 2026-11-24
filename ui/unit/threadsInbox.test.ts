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
  assert.equal(statusWord('failed', { limited: true }), 'Limited');
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
  assert.deepEqual(['needs-you', 'working', 'running', 'failed', 'stopped', 'idle'].map((k) => subagentWord(k)), ['Needs you', 'Working', 'Working', 'Failed', 'Stopped', 'Done']);
  assert.deepEqual(
    subagentRollup([{ title: 'tests', key: 'working' }, { title: 'api', key: 'needs-you' }, { title: 'docs', key: 'failed' }, { title: 'lint', key: 'idle' }]),
    { total: 4, working: 1, needsYou: 1, failed: 1, needing: 'api' },
  );
  // A subagent stopped on its usage limit says Limited and rolls up like Failed.
  assert.equal(subagentWord('failed', { limited: true }), 'Limited');
  assert.deepEqual(
    childRollup([{ title: 'docs', word: 'Limited' }, { title: 'lint', word: 'Failed' }]),
    { total: 2, working: 0, needsYou: 0, failed: 2, needing: null },
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

// A thread waiting on the person is "Approval" in the main list, never in the
// Working fold — whichever source says it waits (each pinned on its own).
test('runtime approval: the row’s awaiting_permission status (runtime_waiting) makes it needs-you', async () => {
  const { sessionState } = await import('../src/lib/status.ts');
  const st = sessionState({ kind: 'agent', status: 'working', meta: { runtime: 'structured', runtime_waiting: true } } as never, 'working');
  assert.equal(st.key, 'needs-you');
  assert.equal(inboxClass(st.key, false), 'needs');
  assert.equal(statusWord('needs', { kinds: ['command'] }), 'Approval');
});

test('a pending request known live lifts a still-"working" row out of Working', () => {
  assert.equal(inboxClass('working', false, 1), 'needs');
  assert.equal(inboxClass('working', false, 0), 'working');
});

test('MCP ask: a waiting approval card counts as a pending permission request → Approval', () => {
  // threadInbox.svelte.ts kindsOf adds 'permission' for a waiting MCP card.
  const kinds = ['permission'];
  assert.equal(inboxClass('working', false, kinds.length), 'needs');
  assert.equal(waitingWord(kinds), 'Approval');
});

test('subagent approval: a waiting subagent keeps its family out of Working, its parent needs you', async () => {
  const { childRollup } = await import('../src/modules/threads/runtime/childThreads.ts');
  const { subagentWord } = await import('../src/modules/threads/threadInbox.ts');
  const kid = s('kid', 2, 'lead');
  const lead = s('lead', 1);
  const cls: Record<string, string> = { lead: 'working', kid: inboxClass('needs-you', false) };
  const { main, working } = inboxSections([lead, kid], (x) => cls[x.id] as never, parentOf);
  assert.deepEqual(working, []);
  assert.equal(main.map((r) => r.session.id)[0], 'lead');
  // The parent's own row: its subagents' roll-up says one needs the person.
  assert.equal(childRollup([{ title: 'kid', word: subagentWord('needs-you') }]).needsYou, 1);
});
