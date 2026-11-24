// Threads' one "Related threads" control (relatedThreads.ts) and the toolbar
// ⋯ that took over the toolbar's view buttons (threadsPageMenu.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { relatedRows, relatedStatus } from '../src/modules/threads/relatedThreads.ts';
import { threadsPageMenu, RELATED_LABEL } from '../src/modules/threads/threadsPageMenu.ts';

const t = (id: string, status = 'idle') => ({ id, title: `T ${id}`, provider: 'codex', status });

test('related threads: parent first, each thread once, closed linked ones left out, self never', () => {
  const rows = relatedRows('me', {
    parent: t('p'),
    linked: [t('me'), t('p'), t('a', 'working'), t('gone', 'closed')],
    workers: [t('w'), t('a')],
    side: t('s'),
  });
  assert.deepEqual(rows.map((r) => [r.id, r.kind]), [['p', 'parent'], ['a', 'linked'], ['w', 'worker'], ['s', 'side']]);
  assert.deepEqual(relatedRows('me', {}), []);
});

test('related status words and tones', () => {
  assert.deepEqual(relatedStatus('working'), { tone: 'live', text: 'Working' });
  assert.deepEqual(relatedStatus('awaiting_permission'), { tone: 'ask', text: 'Needs you' });
  assert.deepEqual(relatedStatus('idle'), { tone: 'idle', text: 'Done' });
  assert.deepEqual(relatedStatus('dormant'), { tone: 'off', text: 'Dormant' });
});

const noop = () => {};
const acts = { newThread: noop, related: noop, toggleCard: noop, layout: noop, toggleTerminal: noop };
const labels = (items: { label?: string; separator?: boolean }[]) => items.map((i) => (i.separator ? '—' : i.label));

test('toolbar ⋯ holds what left the toolbar: related, terminal, a View flyout for layout and card', () => {
  const items = threadsPageMenu({ threadItems: [{ label: 'Rename…' }], layout: 'split', terminalOpen: true, cardOn: false, threadId: 'x', sidebarVisible: true }, acts);
  assert.deepEqual(labels(items), ['Rename…', '—', RELATED_LABEL, 'Terminal drawer', 'View']);
  const view = items.find((i) => i.label === 'View')?.children ?? [];
  assert.deepEqual(labels(view), ['Single thread', 'Split', 'Grid', '—', 'Thread details']);
  assert.equal(view.find((i) => i.label === 'Split')?.checked, true);
  assert.equal(items.find((i) => i.label === 'Terminal drawer')?.checked, true);
});

test('toolbar ⋯ on the stage and on a phone; Related appears once', () => {
  // The sidebar's pencil (and ⌘T) start a thread while it shows.
  const stage = threadsPageMenu({ threadItems: [], layout: 'single', terminalOpen: false, cardOn: null, threadId: null, sidebarVisible: true }, acts);
  assert.deepEqual(labels(stage), ['Terminal drawer', 'View']);
  const phone = threadsPageMenu({ threadItems: [], layout: 'single', terminalOpen: false, cardOn: null, threadId: null, compact: true }, acts);
  assert.deepEqual(labels(phone), ['New thread', 'Terminal drawer']);
  const legacy = threadsPageMenu({ threadItems: [{ label: RELATED_LABEL }], layout: 'single', terminalOpen: false, cardOn: true, threadId: 'x' }, acts);
  assert.equal(legacy.filter((i) => i.label === RELATED_LABEL).length, 1);
});

test('View carries the per-device "while working" choice, on a phone too (without layouts)', () => {
  let picked = '';
  const a = { ...acts, setMidTurn: (m: string) => (picked = m) };
  const desk = threadsPageMenu({ threadItems: [], layout: 'single', terminalOpen: false, cardOn: null, threadId: 'x', sidebarVisible: true, midTurn: 'queue' }, a);
  const view = desk.find((i) => i.label === 'View')?.children ?? [];
  assert.deepEqual(labels(view), ['Single thread', 'Split', 'Grid', '—', 'While working, messages queue', 'While working, messages steer']);
  assert.equal(view.find((i) => i.label === 'While working, messages queue')?.checked, true);
  view.find((i) => i.label === 'While working, messages steer')?.action?.();
  assert.equal(picked, 'steer');
  const phone = threadsPageMenu({ threadItems: [], layout: 'single', terminalOpen: false, cardOn: null, threadId: 'x', compact: true, midTurn: 'steer' }, a);
  const pview = phone.find((i) => i.label === 'View')?.children ?? [];
  assert.deepEqual(labels(pview), ['While working, messages queue', 'While working, messages steer']);
  assert.equal(pview[1].checked, true);
  assert.equal(pview[1].hint, undefined, 'no ⌘↵ hint on a touch screen');
});

test('Jump to… is offered on a phone / tablet and on any touch screen, never on a desktop with a pointer', () => {
  const acts = { newThread() {}, related() {}, toggleCard() {}, layout() {}, toggleTerminal() {}, jump() {} };
  const has = (s: Record<string, unknown>) => threadsPageMenu({ threadItems: [], layout: 'single', terminalOpen: false, cardOn: null, threadId: 'x', ...s } as never, acts).some((i) => i.label === 'Jump to…');
  assert.equal(has({}), false);
  assert.equal(has({ compact: true }), true);
  assert.equal(has({ touch: true }), true);
});
