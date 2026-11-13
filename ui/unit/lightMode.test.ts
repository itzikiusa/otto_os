// The phone's light version (lib/lightMode.svelte.ts): an opt-in, per-device
// trim to five modules. These tests pin the module set, the routes it still
// opens, that the scope narrows every navigation surface fed by
// availableModules / goToEntries (but not the unscoped bookkeeping), that it
// needs a phone-sized screen, and that pageKeyOf gates hidden modules before
// their chunk could load.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import {
  availableModules,
  goToEntries,
  inModuleScope,
  navIdForModule,
  routeAllowed,
  setModuleScope,
} from '../src/lib/sidebar.ts';
import * as routeKeys from '../src/lib/routeKeys.ts';

const all = () => true;
const ids = (list: { id: string }[]) => list.map((m) => m.id);

interface Fixture {
  light: Record<string, any>;
  viewport: { isPhone: boolean; isTablet: boolean };
  store: Map<string, string>;
  went: string[];
  router: { parts: string[]; go: (p: string) => void };
}

/** Load lightMode with a fake device: storage, viewport, router. The real
 *  sidebar module receives its scope, as in the app. */
function loadLight(opts: { stored?: string; phone?: boolean; tablet?: boolean; route?: string[] } = {}): Fixture {
  const store = new Map<string, string>();
  if (opts.stored !== undefined) store.set('otto_phone_light', opts.stored);
  const viewport = { isPhone: opts.phone ?? true, isTablet: opts.tablet ?? false };
  const went: string[] = [];
  const router = { parts: opts.route ?? [], go: (p: string) => went.push(p) };
  const light = loadSource(new URL('../src/lib/lightMode.svelte.ts', import.meta.url), {
    './storage': { lsGet: (k: string) => store.get(k) ?? null, lsSet: (k: string, v: string) => store.set(k, v) },
    './stores/viewport.svelte': { viewport },
    './desktop': { isPopout: false, isEmbedded: false },
    './sidebar': { navIdForModule, setModuleScope },
    './router.svelte': { router },
  });
  return { light, viewport, store, went, router };
}

test('the light version is exactly Threads, Connections, Git, API and Usage, Threads first', () => {
  const { light } = loadLight();
  assert.deepEqual([...light.LIGHT_MODULES], ['threads', 'connections', 'git', 'api', 'usage']);
  assert.equal(light.LIGHT_HOME, 'threads');
  setModuleScope(null);
});

test('routes: the five (and Connections’ own views), Settings and guides; nothing else, never the bare route', () => {
  const { light } = loadLight();
  for (const r of ['threads', 'connections', 'database', 'brokers', 'git', 'api', 'usage', 'settings', 'walkthroughs']) {
    assert.ok(light.lightRouteAllowed(r), r);
  }
  for (const r of ['', 'agents', 'home', 'kubernetes', 'aws', 'plugin', 'rooms', 'product', 'design', 'vault', 'projects']) {
    assert.ok(!light.lightRouteAllowed(r), r);
  }
  setModuleScope(null);
});

test('default is the full app: nothing is scoped until the person turns light on', () => {
  const { light } = loadLight();
  assert.equal(light.lightMode.chosen, false);
  assert.equal(light.lightMode.active, false);
  assert.equal(availableModules(all, []).length, availableModules(all, [], { unscoped: true }).length);
  setModuleScope(null);
});

test('light on a phone: availableModules, ⌘K Go-to and their extras narrow to the five; unscoped keeps every module', () => {
  const { light } = loadLight({ stored: '1' });
  assert.equal(light.lightMode.active, true);
  assert.deepEqual(ids(availableModules(all, [{ id: 'plugin/demo', icon: 'box', label: 'Demo' }])).sort(), ['api', 'connections', 'git', 'threads', 'usage']);
  const goto = goToEntries(availableModules(all, [])).map((e) => e.id);
  assert.ok(goto.includes('core.go-threads') && goto.includes('core.go-usage'));
  assert.ok(goto.includes('core.go-brokers'), 'Message Brokers belongs to Connections');
  for (const hidden of ['core.go-agents', 'core.go-home', 'core.go-kubernetes', 'core.go-walkthroughs']) assert.ok(!goto.includes(hidden), hidden);
  assert.ok(ids(availableModules(all, [], { unscoped: true })).includes('agents'), 'known-module bookkeeping sees everything');
  // The role gate is separate and unchanged.
  assert.equal(routeAllowed('kubernetes', all), true);
  setModuleScope(null);
  assert.ok(inModuleScope('agents'));
});

test('light needs a phone-sized screen: a tablet or desktop with the stored choice is unchanged', () => {
  const f = loadLight({ stored: '1', phone: false, tablet: true });
  assert.equal(f.light.lightMode.chosen, true);
  assert.equal(f.light.lightMode.phoneSized, false, 'a tablet held upright is not a phone');
  assert.equal(f.light.lightMode.active, false);
  assert.ok(ids(availableModules(all, [])).includes('agents'));
  f.viewport.isTablet = false;
  assert.equal(f.light.lightMode.active, false, 'desktop');
  setModuleScope(null);
});

test('set() remembers the choice on this device and leaves a hidden page for Threads', () => {
  const f = loadLight({ route: ['agents', 's1'] });
  f.light.lightMode.set(true);
  assert.equal(f.store.get('otto_phone_light'), '1');
  assert.equal(f.light.lightMode.active, true);
  assert.deepEqual(f.went, ['threads']);
  // Already on a light page: stay there.
  f.router.parts = ['git', 'r1'];
  f.light.lightMode.set(true);
  assert.deepEqual(f.went, ['threads']);
  // Back to the full app: remembered, no navigation.
  f.light.lightMode.set(false);
  assert.equal(f.store.get('otto_phone_light'), '0');
  assert.equal(f.light.lightMode.active, false);
  assert.deepEqual(f.went, ['threads']);
  setModuleScope(null);
});

test('openFullOnce: a terminal-session link opens in the full app for that visit only, and the choice is kept', () => {
  const f = loadLight({ stored: '1', route: ['agents', 's1'] });
  assert.equal(f.light.lightMode.active, true);
  f.light.lightMode.openFullOnce('agents/s1');
  assert.equal(f.light.lightMode.active, false, 'the session shows');
  assert.equal(f.store.get('otto_phone_light'), '1', 'the device choice is unchanged');
  f.router.parts = ['threads'];
  assert.equal(f.light.lightMode.active, true, 'light is back once they move on');
  f.router.parts = ['agents', 's2'];
  assert.equal(f.light.lightMode.active, true, 'another session is still gated');
  f.router.parts = ['agents', 's1'];
  f.light.lightMode.set(true);
  assert.equal(f.light.lightMode.active, true, 'switching clears the visit');
  setModuleScope(null);
});

test('a stored choice from an earlier visit is read at boot', () => {
  assert.equal(loadLight({ stored: '1' }).light.lightMode.chosen, true);
  assert.equal(loadLight({ stored: '0' }).light.lightMode.chosen, false);
  assert.equal(loadLight({ stored: 'yes' }).light.lightMode.chosen, false);
  setModuleScope(null);
});

/** pages.svelte.ts with a controllable light switch (no chunk is loaded). */
function loadPages(active: boolean) {
  const lightMode = { active };
  const pages = loadSource(new URL('../src/shell/pages.svelte.ts', import.meta.url), {
    '../lib/telemetry': { startMeasurement: () => null },
    '../lib/loadError': { loadErrorText: String },
    'svelte/reactivity': { SvelteMap: Map },
    '../lib/stores/auth.svelte': { auth: { phase: 'loading', me: null, capabilitiesError: null, can: () => true } },
    '../lib/sidebar': { routeAllowed },
    '../lib/routeKeys': routeKeys,
    '../lib/lightMode.svelte': {
      LIGHT_HOME: 'threads',
      lightMode,
      lightRouteAllowed: (first: string) =>
        first !== '' && (['settings', 'walkthroughs'].includes(first) || ['threads', 'connections', 'git', 'api', 'usage'].includes(navIdForModule(first))),
    },
  });
  return { pages, lightMode };
}

test('pageKeyOf: light lands on Threads and gates hidden modules; the full app is unchanged', () => {
  const { pages, lightMode } = loadPages(true);
  assert.equal(pages.pageKeyOf([]), 'threads');
  assert.equal(pages.pageKeyOf(['plugin']), 'threads');
  assert.equal(pages.pageKeyOf(['threads', 't1']), 'threads');
  assert.equal(pages.pageKeyOf(['connections']), 'database');
  assert.equal(pages.pageKeyOf(['brokers', 'c1']), 'brokers');
  assert.equal(pages.pageKeyOf(['settings', 'appearance']), 'settings');
  for (const hidden of [['agents', 's1'], ['home'], ['kubernetes'], ['plugin', 'demo'], ['projects']]) {
    assert.equal(pages.pageKeyOf(hidden), pages.ROUTE_GATE, hidden.join('/'));
  }
  assert.equal(pages.pageKeyOf(['nope']), pages.ROUTE_GATE);
  assert.equal(pages.pageKeyOf(['s', 'x']), null, 'the share view is outside the shell either way');

  lightMode.active = false;
  assert.equal(pages.pageKeyOf([]), 'agents');
  assert.equal(pages.pageKeyOf(['agents', 's1']), 'agents');
  assert.equal(pages.pageKeyOf(['kubernetes']), 'kubernetes');
  assert.equal(pages.pageKeyOf(['projects']), 'agents');
});
