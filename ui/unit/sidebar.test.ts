// Sidebar ordering logic (lib/sidebar.ts): Favorites extraction, section
// order, RBAC filtering and unknown ids. node:test with Node's built-in type
// stripping — the `.ts` extension on the import is required in strip mode.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  SIDEBAR_GROUPS,
  availableModules,
  favoriteModules,
  insertFavorite,
  moveAmong,
  reorderAmong,
  resolveGroupOrder,
  resolveOrder,
  sidebarSections,
  visibleOrder,
  type SidebarModule,
  type SidebarSection,
} from '../src/lib/sidebar.ts';

const all = () => availableModules(() => true, []);
const ids = (mods: SidebarModule[]) => mods.map((m) => m.id);
const sectionOf = (secs: SidebarSection[], id: string) =>
  secs.find((s) => s.modules.some((m) => m.id === id))?.group.id;

test('no favorites: no Favorites section, shipped section order', () => {
  const secs = sidebarSections(all(), []);
  assert.deepEqual(
    secs.map((s) => s.group.id),
    SIDEBAR_GROUPS.filter((g) => g.id !== 'plugins').map((g) => g.id),
  );
  assert.ok(!secs.some((s) => s.group.id === 'favorites'));
});

test('favorites come first, in the saved order, and leave their own sections', () => {
  const favs = ['agents', 'workflows', 'git', 'connections'];
  const secs = sidebarSections(all(), favs);
  assert.equal(secs[0].group.id, 'favorites');
  assert.equal(secs[0].group.label, 'Favorites');
  assert.deepEqual(ids(secs[0].modules), favs);
  // Each favorite is listed exactly once — only under Favorites.
  const flat = secs.flatMap((s) => ids(s.modules));
  assert.equal(new Set(flat).size, flat.length);
  for (const id of favs) assert.equal(sectionOf(secs, id), 'favorites');
  // The rest of each section keeps its order.
  const work = secs.find((s) => s.group.id === 'work')!;
  assert.deepEqual(ids(work.modules), ['home', 'assistant', 'history', 'run-with-otto', 'mission-control']);
  // Unfavoriting puts a module back in its section, in its saved slot.
  const back = sidebarSections(all(), ['workflows', 'git', 'connections']);
  assert.deepEqual(ids(back.find((s) => s.group.id === 'work')!.modules).slice(0, 3), ['home', 'assistant', 'agents']);
});

test('a section emptied by favorites disappears; a user order still applies inside Favorites', () => {
  const secs = sidebarSections(all(), ['usage', 'insights']);
  assert.deepEqual(ids(secs[0].modules), ['usage', 'insights']);
  assert.ok(!secs.some((s) => s.group.id === 'insight'));
});

test('favorites respect RBAC, features and plugins: an unavailable favorite is skipped', () => {
  // Only `git` is granted among the gated features; ungated modules pass.
  const mods = availableModules((f) => f === 'git', [{ id: 'plugin/demo', icon: 'box', label: 'Demo' }]);
  const secs = sidebarSections(mods, ['agents', 'git', 'plugin/gone', 'vault', 'plugin/demo', 'nope']);
  assert.deepEqual(ids(secs[0].modules), ['git', 'vault', 'plugin/demo']);
  assert.ok(!secs.flatMap((s) => ids(s.modules)).includes('agents'));
  // A plugin favorite leaves Plugins, which (now empty) is dropped.
  assert.ok(!secs.some((s) => s.group.id === 'plugins'));
});

test('favorites + hidden: hidden favorites vanish outside edit mode, stay listed while editing', () => {
  const resolved = resolveOrder(all(), []);
  const visible = visibleOrder(resolved, ['git']);
  assert.deepEqual(ids(sidebarSections(visible, ['git', 'vault'])[0].modules), ['vault']);
  assert.deepEqual(ids(sidebarSections(resolved, ['git', 'vault'])[0].modules), ['git', 'vault']);
  // Every favorite hidden → no Favorites section at all.
  assert.notEqual(sidebarSections(visibleOrder(resolved, ['git', 'vault']), ['git', 'vault'])[0].group.id, 'favorites');
});

test('duplicate favorite ids render once', () => {
  assert.deepEqual(ids(favoriteModules(all(), ['git', 'git', 'vault', 'git'])), ['git', 'vault']);
});

test('section order: saved first, unknown / duplicate / favorites ignored, missing appended in default order', () => {
  const order = (saved: string[]) => resolveGroupOrder(saved).map((g) => g.id);
  assert.deepEqual(order([]), ['work', 'automate', 'build', 'infra', 'insight', 'plugins']);
  assert.deepEqual(order(['insight', 'work']), ['insight', 'work', 'automate', 'build', 'infra', 'plugins']);
  assert.deepEqual(
    order(['favorites', 'bogus', 'build', 'build', 'plugins']),
    ['build', 'plugins', 'work', 'automate', 'infra', 'insight'],
  );
  // Favorites stays first whatever the section order says.
  const secs = sidebarSections(all(), ['git'], ['insight', 'favorites', 'work']);
  assert.deepEqual(secs.map((s) => s.group.id), ['favorites', 'insight', 'work', 'automate', 'build', 'infra']);
});

test('moveAmong: swaps with the nearest present neighbour, null at the ends', () => {
  const list = ['a', 'b', 'c', 'd'];
  assert.deepEqual(moveAmong(list, 'b', -1), ['b', 'a', 'c', 'd']);
  assert.deepEqual(moveAmong(list, 'b', 1), ['a', 'c', 'b', 'd']);
  assert.equal(moveAmong(list, 'a', -1), null);
  assert.equal(moveAmong(list, 'd', 1), null);
  assert.equal(moveAmong(list, 'zz', 1), null);
  // 'c' isn't rendered (RBAC / empty section): 'b' hops over it.
  const shown = (id: string) => id !== 'c';
  assert.deepEqual(moveAmong(list, 'b', 1, shown), ['a', 'd', 'c', 'b']);
  assert.equal(moveAmong(['a', 'b', 'c'], 'b', 1, shown), null);
  // Section moves over the resolved group list.
  const groups = resolveGroupOrder([]).map((g) => g.id);
  const noPlugins = (id: string) => id !== 'plugins';
  assert.equal(moveAmong(groups, 'insight', 1, noPlugins), null);
  assert.deepEqual(moveAmong(groups, 'insight', -1, noPlugins)?.slice(0, 5), ['work', 'automate', 'build', 'insight', 'infra']);
});

test('reorderAmong: drag lands before the target going up, after it going down', () => {
  assert.deepEqual(reorderAmong(['a', 'b', 'c', 'd'], 'd', 'b'), ['a', 'd', 'b', 'c']);
  assert.deepEqual(reorderAmong(['a', 'b', 'c', 'd'], 'a', 'c'), ['b', 'c', 'a', 'd']);
  assert.equal(reorderAmong(['a', 'b'], 'a', 'a'), null);
  assert.equal(reorderAmong(['a', 'b'], 'a', 'zz'), null);
});

test('insertFavorite: appends, inserts before a target, never duplicates', () => {
  assert.deepEqual(insertFavorite([], 'git'), ['git']);
  assert.deepEqual(insertFavorite(['agents', 'git'], 'vault'), ['agents', 'git', 'vault']);
  assert.deepEqual(insertFavorite(['agents', 'git'], 'vault', 'git'), ['agents', 'vault', 'git']);
  assert.deepEqual(insertFavorite(['agents', 'git'], 'vault', 'missing'), ['agents', 'git', 'vault']);
  assert.deepEqual(insertFavorite(['agents', 'git', 'vault'], 'vault', 'agents'), ['vault', 'agents', 'git']);
});
