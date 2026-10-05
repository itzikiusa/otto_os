import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import ts from 'typescript';

// The Database Explorer prefs moved out of the 4.4k-line database store into
// lib/stores/dbPrefs.svelte.ts so Settings → Appearance doesn't load the store.
// These pin the contract that move must keep: identical storage keys (saved
// prefs survive), the same defaults, persistence, and the keep-alive
// subscription the store uses to start/stop its poller.
function load(saved: Record<string, string> = {}) {
  const store = new Map(Object.entries(saved));
  const localStorage = {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
  };
  const context: any = {exports: {}, localStorage, $state: Object.assign((v: unknown) => v, {raw: (v: unknown) => v})};
  const src = readFileSync(new URL('../src/lib/stores/dbPrefs.svelte.ts', import.meta.url), 'utf8');
  runInNewContext(ts.transpileModule(src, {compilerOptions: {module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022}}).outputText, context);
  return {m: context.exports, store};
}

test('storage keys are the database-store-era ones', () => {
  const {m} = load();
  assert.equal(m.WARM_ON_CLICK_KEY, 'otto_db_warm_on_click');
  assert.equal(m.KEEP_ALIVE_KEY, 'otto_db_keep_alive');
});

test('defaults: warm in the background, keep-alive on', () => {
  const {m} = load();
  assert.equal(m.dbPrefs.warmRestored, 'background');
  assert.equal(m.dbPrefs.keepAlive, true);
});

test('previously saved prefs are read back', () => {
  const {m} = load({otto_db_warm_on_click: '1', otto_db_keep_alive: '0'});
  assert.equal(m.dbPrefs.warmRestored, 'on-click');
  assert.equal(m.dbPrefs.keepAlive, false);
});

test('setters persist and notify keep-alive subscribers', () => {
  const {m, store} = load();
  const seen: boolean[] = [];
  const off = m.dbPrefs.onKeepAliveChange((on: boolean) => seen.push(on));
  m.dbPrefs.setWarmRestored('on-click');
  m.dbPrefs.setKeepAlive(false);
  assert.equal(store.get('otto_db_warm_on_click'), '1');
  assert.equal(store.get('otto_db_keep_alive'), '0');
  assert.deepEqual(seen, [false]);
  off();
  m.dbPrefs.setKeepAlive(true);
  assert.deepEqual(seen, [false]);
  assert.equal(m.dbPrefs.keepAlive, true);
});

test('Settings → Appearance reads the prefs module, never the database store', () => {
  const src = readFileSync(new URL('../src/modules/settings/Appearance.svelte', import.meta.url), 'utf8');
  assert.doesNotMatch(src, /stores\/database\.svelte/);
});
