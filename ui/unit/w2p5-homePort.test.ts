import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { loadSource } from './sourceHarness.ts';

// architecture-1-13: the `otto.ui_home_*` handlers live in lib/ and reach the
// Home module's store only through `homePort`, which the store binds when it
// loads. A handler navigates to Home, then WAITS for the port (the Home chunk
// may still be loading), and drives whatever store bound it.

class UiCommandError extends Error {
  code: string;
  constructor(code: string, message: string) { super(message); this.code = code; }
}

function load() {
  const handlers: Record<string, (args: any, ctx: any) => Promise<any>> = {};
  const states: Record<string, () => any> = {};
  const uiCommands = {
    UiCommandError,
    whenMounted: async () => null,
    registerUiCommands: (_m: string, map: Record<string, any>) => { Object.assign(handlers, map); return () => {}; },
    registerUiState: (m: string, fn: () => any) => { states[m] = fn; return () => {}; },
  };
  const pagePort = loadSource(new URL('../src/lib/uiCommands/pagePort.ts', import.meta.url), {
    '../uiCommands': uiCommands,
    '../confirm.svelte': { confirmer: {} },
  });
  const homePort = loadSource(new URL('../src/lib/uiCommands/homePort.ts', import.meta.url), { './pagePort': pagePort });
  const kinds = loadSource(new URL('../src/lib/home/kinds.ts', import.meta.url), {});
  const router = { parts: ['agents'] as string[], go(m: string) { this.parts = [m]; } };
  loadSource(new URL('../src/lib/uiCommands/home.ts', import.meta.url), {
    '../uiCommands': uiCommands,
    '../router.svelte': { router },
    '../stores/auth.svelte': { auth: { me: { id: 'u1' }, can: () => true } },
    '../home/kinds': kinds,
    './pagePort': pagePort,
    './homePort': homePort,
  }, { CSS: { escape: (s: string) => s } });
  return { handlers, states, homePort, router };
}

function fakeStore() {
  const calls: string[] = [];
  const view = { id: 'v1', name: 'Main', boxes: [{ id: 'b1', kind: 'sessions', w: 6, h: 3, config: {} }] };
  return {
    calls,
    views: [view],
    active: view,
    activeIndex: 0,
    zoomedId: null as string | null,
    autoRotate: false,
    maxBoxes: 8,
    ensureDefault() { calls.push('ensureDefault'); },
    goTo(i: number) { calls.push(`goTo:${i}`); },
    toggleZoom(id: string) { calls.push(`zoom:${id}`); this.zoomedId = this.zoomedId === id ? null : id; },
    addBox() { return null; },
    removeBox() {},
  };
}

const ctx = (signal = new AbortController().signal) => ({ signal, confirmWrite: async () => true });

test('a handler navigates to Home and waits for the store to bind the port', async () => {
  const { handlers, homePort, router } = load();
  const store = fakeStore();
  const pending = handlers.home_state({}, ctx());
  // The Home chunk "loads" after the navigation: the port is bound later.
  await new Promise((r) => setTimeout(r, 120));
  assert.deepEqual(router.parts, ['home'], 'navigated first');
  homePort.registerHomePort(store);
  const out = await pending;
  assert.equal(out.views.length, 1);
  assert.equal(out.views[0].name, 'Main');
  assert.equal(out.views[0].active, true);
  assert.deepEqual(store.calls, ['ensureDefault'], 'drives the bound store');
});

test('zoom drives the bound store, not a module import', async () => {
  const { handlers, homePort } = load();
  const store = fakeStore();
  homePort.registerHomePort(store);
  const out = await handlers.home_zoom_box({ box_id: 'b1' }, ctx());
  assert.equal(out.zoomed_box, 'b1');
  assert.deepEqual(store.calls, ['ensureDefault', 'goTo:0', 'zoom:b1']);
});

test('an aborted command stops waiting for an unbound port', async () => {
  const { handlers } = load();
  const ac = new AbortController();
  const pending = handlers.home_state({}, ctx(ac.signal));
  setTimeout(() => ac.abort(), 60);
  await assert.rejects(pending, (e: any) => e.code === 'cancelled_by_user');
});

test('ui_state reports an empty layout until the store has loaded', () => {
  const { states, homePort } = load();
  assert.deepEqual(JSON.parse(JSON.stringify(states.home())), { view: null, views: 0, boxes: [], zoomed_box: null });
  homePort.registerHomePort(fakeStore());
  assert.equal(states.home().views, 1);
  assert.deepEqual({ ...states.home().view }, { index: 1, name: 'Main' });
});

test('lib/ reaches Home only through the port (no modules/ import)', () => {
  for (const f of ['../src/lib/uiCommands/home.ts', '../src/lib/stores/assistant.svelte.ts', '../src/lib/uiCommands/homePort.ts']) {
    assert.doesNotMatch(readFileSync(new URL(f, import.meta.url), 'utf8'), /from '[^']*modules\//, f);
  }
  assert.match(readFileSync(new URL('../src/modules/home/home.svelte.ts', import.meta.url), 'utf8'), /^registerHomePort\(home\);$/m);
});
