// "Show what he uses" (lib/sidebarMore.ts): pinned + used + badged + current stay; the rest go under More.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { DEFAULT_SHOWN, barOrder, shownModules, splitMore, usedModules } from '../src/lib/sidebarMore.ts';

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

test('used modules rank by phone use, then overall use', () => {
  const ids = usedModules(
    [
      { module: 'threads', count: 9, device: 'desktop' },
      { module: 'agents', count: 50, device: 'desktop' },
      { module: 'git', count: 4, device: 'phone' },
      { module: 'database', count: 2, device: 'phone' },
      { module: 'connections', count: 3, device: 'phone' },
      { module: 'rooms', count: 1, device: 'desktop' },
    ],
    navId,
  );
  assert.deepEqual(ids, ['connections', 'git', 'agents', 'threads', 'rooms']);
});

test('the phone bar: pinned, then most used, then the rest in sidebar order (not sidebar order first)', () => {
  const mods = ['home', 'agents', 'threads', 'rooms', 'git', 'connections', 'usage'].map((id) => ({ id }));
  const ids = (m: { id: string }[]) => m.map((x) => x.id);
  assert.deepEqual(ids(barOrder(mods, [], ['agents', 'git', 'connections', 'usage', 'home'])).slice(0, 4), ['agents', 'git', 'connections', 'usage']);
  assert.deepEqual(ids(barOrder(mods, ['rooms'], ['agents', 'git'])).slice(0, 4), ['rooms', 'agents', 'git', 'home']);
  assert.deepEqual(ids(barOrder(mods, [], null)).slice(0, 4), ['agents', 'git', 'connections', 'threads']);
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
