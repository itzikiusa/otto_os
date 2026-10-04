import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

function fixture(api: Record<string, unknown>) {
  const { browser } = loadSource(new URL('../src/lib/stores/browser.svelte.ts', import.meta.url), {
    '../api/browser': api, '../nativeBrowser': { nativeBrowserAvailable: false },
    './browserLive.svelte': { browserLive: {} }, '../lazyModule': { announceModule() {} },
  });
  browser.wsId = 'ws';
  browser.tabs = [{ id: 'A', url: 'https://a.test' }, { id: 'B', url: 'https://b.test' }];
  browser.activeId = 'A';
  return browser;
}

test('late annotation read cannot replace another active tab marks', async () => {
  const a = deferred<unknown>();
  const browser = fixture({ listAnnotations: (_ws: string, url: string) => url.includes('a.test') ? a.promise : Promise.resolve([{ id: 'B-mark' }]) });
  const pending = browser.loadAnnotations('https://a.test');
  browser.activeId = 'B'; await browser.loadAnnotations('https://b.test');
  a.resolve([{ id: 'A-mark' }]); await pending;
  assert.equal(browser.annotations[0].id, 'B-mark');
});

test('creating a mark then switching tabs does not append to the new page', async () => {
  const a = deferred<unknown>();
  const browser = fixture({ createAnnotation: () => a.promise });
  const pending = browser.createAnnotation({ url: 'https://a.test', selector: 'p', text: 'A', excerpt: '' });
  browser.activeId = 'B'; browser.annotations = [{ id: 'B-mark' }];
  a.resolve({ id: 'A-mark' }); await pending;
  assert.equal(browser.annotations.length, 1);
  assert.equal(browser.annotations[0].id, 'B-mark');
});
