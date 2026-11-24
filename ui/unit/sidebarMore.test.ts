// "Show what he uses" (lib/sidebarMore.ts): pinned + used + badged + current stay; the rest go under More.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { DEFAULT_SHOWN, shownModules, splitMore, usedModules } from '../src/lib/sidebarMore.ts';

const navId = (m: string) => (m === 'database' || m === 'brokers' ? 'connections' : m);

test('used modules: counted ones, mapped onto sidebar entries, once each; shell/other never', () => {
  const ids = usedModules(
    [
      { module: 'agents', count: 40 },
      { module: 'database', count: 3 },
      { module: 'connections', count: 1 },
      { module: 'aws', count: 0 },
      { module: 'other', count: 9 },
      { module: 'shell', count: 2 },
    ],
    navId,
  );
  assert.deepEqual(ids, ['agents', 'connections']);
});

test('shown: favorites ∪ used ∪ badged ∪ active; no usage yet → the defaults', () => {
  const none = shownModules({ favorites: ['vault'], used: null, badged: [], active: 'home' });
  assert.deepEqual([...none].sort(), [...DEFAULT_SHOWN, 'vault'].sort());
  assert.deepEqual([...shownModules({ favorites: [], used: [], badged: [], active: 'git' })].sort(), [...DEFAULT_SHOWN].sort());
  const used = shownModules({ favorites: ['vault'], used: ['agents', 'kubernetes'], badged: ['workflows'], active: 'aws' });
  assert.deepEqual([...used].sort(), ['agents', 'aws', 'kubernetes', 'vault', 'workflows']);
});

test('split: sections keep shown modules in order, emptied sections drop, the rest go to More in order', () => {
  const sections = [
    { group: 'work', modules: [{ id: 'home' }, { id: 'agents' }, { id: 'rooms' }] },
    { group: 'automate', modules: [{ id: 'loops' }, { id: 'workflows' }] },
    { group: 'insight', modules: [{ id: 'usage' }] },
  ];
  const { sections: kept, more } = splitMore(sections, new Set(['agents', 'home', 'usage']));
  assert.deepEqual(kept.map((s) => [s.group, s.modules.map((m) => m.id)]), [
    ['work', ['home', 'agents']],
    ['insight', ['usage']],
  ]);
  assert.deepEqual(more.map((m) => m.id), ['rooms', 'loops', 'workflows']);
  assert.equal(sections[0].modules.length, 3, 'the input is not mutated');
});
