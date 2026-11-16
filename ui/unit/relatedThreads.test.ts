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
  assert.deepEqual(relatedStatus('idle'), { tone: 'idle', text: 'Idle' });
  assert.deepEqual(relatedStatus('dormant'), { tone: 'off', text: 'Dormant' });
});

const noop = () => {};
const acts = { newThread: noop, related: noop, toggleCard: noop, layout: noop, toggleTerminal: noop };
const labels = (items: { label?: string; separator?: boolean }[]) => items.map((i) => (i.separator ? '—' : i.label));

test('toolbar ⋯ holds what left the toolbar: new, related, card, terminal, layout', () => {
  const items = threadsPageMenu({ threadItems: [{ label: 'Rename…' }], layout: 'split', terminalOpen: true, cardOn: false, threadId: 'x' }, acts);
  assert.deepEqual(labels(items), ['Rename…', '—', 'New thread', RELATED_LABEL, '—', 'Project card', 'Terminal drawer', '—', 'Single thread', 'Split', 'Grid']);
  assert.equal(items.find((i) => i.label === 'Split')?.checked, true);
  assert.equal(items.find((i) => i.label === 'Terminal drawer')?.checked, true);
});

test('toolbar ⋯ on the stage and on a phone; Related appears once', () => {
  const stage = threadsPageMenu({ threadItems: [], layout: 'single', terminalOpen: false, cardOn: null, threadId: null }, acts);
  assert.deepEqual(labels(stage), ['New thread', '—', 'Terminal drawer', '—', 'Single thread', 'Split', 'Grid']);
  const legacy = threadsPageMenu({ threadItems: [{ label: RELATED_LABEL }], layout: 'single', terminalOpen: false, cardOn: true, threadId: 'x' }, acts);
  assert.equal(legacy.filter((i) => i.label === RELATED_LABEL).length, 1);
});
