// Sidebar ordering logic (lib/sidebar.ts): Favorites extraction, section
// order, RBAC filtering and unknown ids. node:test with Node's built-in type
// stripping — the `.ts` extension on the import is required in strip mode.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  SIDEBAR_GROUPS,
  SIDEBAR_MODULES,
  availableModules,
  favoriteModules,
  insertFavorite,
  mergeSavedOrder,
  moveAmong,
  reconcileNewSidebarModules,
  reorderAmong,
  resolveGroupOrder,
  resolveOrder,
  sidebarSections,
  visibleOrder,
  type SidebarModule,
  type SidebarSection,
  moduleHome,
  navIdForModule,
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
  assert.deepEqual(ids(work.modules), ['home', 'threads', 'rooms']);
  // Unfavoriting puts a module back in its section, in its saved slot.
  const back = sidebarSections(all(), ['workflows', 'git', 'connections']);
  assert.deepEqual(ids(back.find((s) => s.group.id === 'work')!.modules), ['home', 'agents', 'threads', 'rooms']);
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
  assert.ok(!secs.flatMap((s) => ids(s.modules)).includes('rooms'), 'Rooms requires agent visibility');
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

test('reconcileNewSidebarModules: surfaces newly shipped Threads in custom sidebars and respects later hides', () => {
  const preThreadsOrder = SIDEBAR_MODULES.map((m) => m.id).filter((id) => id !== 'threads');

  // 1. User customized order + collapsed Work + pinned Agents in Favorites before Threads shipped.
  const firstBoot = reconcileNewSidebarModules({
    order: preThreadsOrder,
    hidden: ['home', 'rooms'],
    favorites: ['agents', 'git', 'connections'],
    collapsedGroups: ['work', 'automate'],
    groupOrder: [],
    knownModules: null,
  });
  assert.equal(firstBoot.changed, true);
  // Placed right after 'agents' in the saved order.
  assert.equal(firstBoot.order.indexOf('threads'), firstBoot.order.indexOf('agents') + 1);
  // Surfaced above 'agents' in Favorites so Agents's inline session tree doesn't bury it.
  assert.deepEqual(firstBoot.favorites, ['threads', 'agents', 'git', 'connections']);
  // Work unfolded; Automate stays collapsed.
  assert.deepEqual(firstBoot.collapsedGroups, ['automate']);
  assert.deepEqual(firstBoot.knownModules, SIDEBAR_MODULES.map((m) => m.id));

  // 2. Subsequent boot after user explicitly hides & unfavorites Threads: choice sticks!
  const afterUserHid = reconcileNewSidebarModules({
    order: firstBoot.order,
    hidden: [...firstBoot.hidden, 'threads'],
    favorites: ['agents', 'git', 'connections'],
    collapsedGroups: ['work', 'automate'],
    groupOrder: [],
    knownModules: firstBoot.knownModules,
  });
  assert.equal(afterUserHid.changed, false);
  assert.ok(afterUserHid.hidden.includes('threads'));
  assert.ok(!afterUserHid.favorites.includes('threads'));
  assert.deepEqual(afterUserHid.collapsedGroups, ['work', 'automate']);

  // 3. User who only used Favorites + collapsed Work (empty saved order) still gets Threads surfaced.
  const favOnlyUpgrade = reconcileNewSidebarModules({
    order: [],
    hidden: [],
    favorites: ['agents', 'git'],
    collapsedGroups: ['work'],
    groupOrder: [],
    knownModules: null,
  });
  assert.deepEqual(favOnlyUpgrade.favorites, ['threads', 'agents', 'git']);
  assert.deepEqual(favOnlyUpgrade.collapsedGroups, []);
});

test('Personal Agents keeps the scheduled-tasks id but opens on its Agents tab', () => {
  const m = SIDEBAR_MODULES.find((x) => x.id === 'scheduled-tasks');
  assert.equal(m?.label, 'Personal Agents');
  assert.equal(moduleHome('scheduled-tasks'), 'personal-agents');
  assert.equal(moduleHome('git'), 'git');
  assert.equal(navIdForModule('personal-agents'), 'scheduled-tasks');
});

// ── A module that becomes available later keeps its place (owner report) ──

const only = (keep: (id: string) => boolean) => all().filter((m) => keep(m.id));

test('resolveOrder: an id missing from the saved order lands after its section predecessor, not at the bottom', () => {
  // Saved while Kubernetes was gated off; Infrastructure reordered by the user.
  const saved = ids(all()).filter((id) => id !== 'kubernetes');
  const swapped = saved.map((id) => (id === 'aws' ? 'api' : id === 'api' ? 'aws' : id));
  const infra = ids(resolveOrder(all(), swapped).filter((m) => m.group === 'infra'));
  // Registry: connections, aws, kubernetes, api … — Kubernetes follows AWS
  // wherever the user put AWS.
  assert.deepEqual(infra, ['connections', 'api', 'aws', 'kubernetes', 'browser', 'mcp']);
});

test('resolveOrder: the first module of a section anchors before its successor', () => {
  const reversed = ids(all()).filter((id) => id !== 'connections').reverse();
  const infra = ids(resolveOrder(all(), reversed).filter((m) => m.group === 'infra'));
  // Saved reversed: mcp, browser, api, kubernetes, aws — connections goes
  // before its nearest placed successor (aws).
  assert.deepEqual(infra, ['mcp', 'browser', 'api', 'kubernetes', 'connections', 'aws']);
});

test('resolveOrder: a new plugin anchors after the plugin before it; empty saved order = registry order', () => {
  const plugins = [
    { id: 'plugin/a', icon: 'zap' as const, label: 'A' },
    { id: 'plugin/b', icon: 'zap' as const, label: 'B' },
    { id: 'plugin/c', icon: 'zap' as const, label: 'C' },
  ];
  const mods = availableModules(() => true, plugins);
  const saved = [...ids(all()), 'plugin/c', 'plugin/a'];
  const got = ids(resolveOrder(mods, saved).filter((m) => m.group === 'plugins'));
  assert.deepEqual(got, ['plugin/c', 'plugin/a', 'plugin/b']);
  assert.deepEqual(ids(resolveOrder(all(), [])), ids(all()));
});

test('mergeSavedOrder: ids not on screen keep their saved slot; retired ids are dropped', () => {
  const saved = ['home', 'agents', 'threads', 'git', 'kubernetes', 'plugin/x', 'history', 'api'];
  // Kubernetes + the plugin were gated off while the user moved git to the front.
  const next = ['git', 'home', 'agents', 'threads', 'api'];
  assert.deepEqual(mergeSavedOrder(next, saved), ['git', 'kubernetes', 'plugin/x', 'home', 'agents', 'threads', 'api']);
  // Nothing missing: unchanged.
  assert.deepEqual(mergeSavedOrder(['b', 'a'], []), ['b', 'a']);
  // A missing id saved first stays first.
  assert.deepEqual(mergeSavedOrder(['agents', 'home'], ['kubernetes', 'home', 'agents']), ['kubernetes', 'agents', 'home']);
});

test('saving while a module is gated off, then granting it, puts it back where it was', () => {
  const withoutK8s = only((id) => id !== 'kubernetes');
  // The saved order had Kubernetes right after Connections.
  const saved = ['connections', 'kubernetes', 'aws', 'api', 'browser', 'mcp'];
  const before = resolveOrder(withoutK8s, saved);
  const moved = moveAmong(ids(before), 'mcp', -1)!;
  const stored = mergeSavedOrder(moved, saved);
  const infra = ids(resolveOrder(all(), stored).filter((m) => m.group === 'infra'));
  assert.deepEqual(infra, ['connections', 'kubernetes', 'aws', 'api', 'mcp', 'browser']);
});

test('reconcileNewSidebarModules: a module is known only once it has been available', () => {
  const builtins = SIDEBAR_MODULES.map((m) => m.id);
  const withoutK8s = builtins.filter((id) => id !== 'kubernetes');
  // Boot before auth (no modules known to be available): nothing surfaced, nothing new marked.
  const boot = reconcileNewSidebarModules({
    order: withoutK8s,
    hidden: [],
    favorites: [],
    collapsedGroups: ['infra'],
    groupOrder: [],
    knownModules: withoutK8s,
    available: [],
  });
  assert.equal(boot.changed, false);
  assert.ok(!boot.knownModules.includes('kubernetes'));
  // Signed in without the Kubernetes grant: still not known.
  const again = (r: typeof boot, available: string[]) =>
    reconcileNewSidebarModules({ order: r.order, hidden: r.hidden, favorites: r.favorites, collapsedGroups: r.collapsedGroups, groupOrder: [], knownModules: r.knownModules, available });
  const gated = again(boot, withoutK8s);
  assert.ok(!gated.knownModules.includes('kubernetes'));
  assert.ok(!gated.order.includes('kubernetes'));
  // Granted later: surfaced like a newly shipped module — placed after AWS, its
  // section unfolded — and only now recorded as known.
  const granted = again(gated, builtins);
  assert.equal(granted.changed, true);
  assert.equal(granted.order.indexOf('kubernetes'), granted.order.indexOf('aws') + 1);
  assert.deepEqual(granted.collapsedGroups, []);
  assert.deepEqual(granted.knownModules, builtins);
  // Omitting `available` keeps the old "every registry id is available" rule.
  assert.deepEqual(reconcileNewSidebarModules({ order: [], hidden: [], favorites: [], collapsedGroups: [], groupOrder: [], knownModules: null }).knownModules, builtins);
});
