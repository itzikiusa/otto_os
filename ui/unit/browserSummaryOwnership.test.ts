import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';
import { loadErrorText } from '../src/lib/loadError.ts';

function fixture(summarize: (...args: any[]) => Promise<unknown>) {
  const { browser } = loadSource(new URL('../src/lib/stores/browser.svelte.ts', import.meta.url), {
    '../loadError': { loadErrorText }, '../api/browser': {
      summarize, listTabs: async (ws: string) => [{ id: `${ws}-tab`, url: 'https://example.test', mode: 'reader' }],
      getPage: async (_ws: string, url: string) => ({ url, title: url, markdown: '' }),
      listAnnotations: async () => [],
    },
    '../nativeBrowser': { nativeBrowserAvailable: false }, './browserLive.svelte': { browserLive: {} },
    '../lazyModule': { announceModule() {} },
  });
  return browser;
}

for (const transition of ['workspace', 'tab', 'leave-and-return']) {
  test(`summary cannot cross a ${transition} transition even at the same URL`, async () => {
    const result = deferred<unknown>();
    const browser = fixture(() => result.promise);
    await browser.loadTabs('A');
    const pending = browser.runSummarize('https://example.test');
    if (transition === 'workspace') await browser.loadTabs('B');
    else {
      browser.tabs.push({ id: 'other', url: 'https://example.test', mode: 'live' });
      browser.select('other');
      if (transition === 'leave-and-return') browser.select('A-tab');
    }
    result.resolve({ summary: 'Old page summary' });
    await pending;
    assert.equal(browser.summary, '');
  });
}

test('superseding a summary aborts its request and preserves the current busy state', async () => {
  const first = deferred<unknown>(), second = deferred<unknown>();
  const signals: AbortSignal[] = [];
  const browser = fixture((_ws, _url, signal) => { signals.push(signal); return signals.length === 1 ? first.promise : second.promise; });
  await browser.loadTabs('A');
  const a = browser.runSummarize('https://example.test');
  const b = browser.runSummarize('https://example.test');
  const aborted = signals[0].aborted;
  first.resolve({ summary: 'Old' }); await a;
  const stillBusy = browser.summarizing;
  const oldShown = browser.summary;
  second.resolve({ summary: 'Current' }); await b;
  assert.equal(aborted, true);
  assert.equal(stillBusy, true);
  assert.equal(oldShown, '');
  assert.equal(browser.summary, 'Current');
  assert.equal(browser.summarizing, false);
});

test('deselect clears reader loading and error state immediately', async () => {
  const browser = fixture(async () => ({ summary: '' }));
  await browser.loadTabs('A');
  browser.loadingPage = true; browser.pageError = 'Old error';
  browser.deselect();
  assert.equal(browser.loadingPage, false);
  assert.equal(browser.pageError, '');
});
