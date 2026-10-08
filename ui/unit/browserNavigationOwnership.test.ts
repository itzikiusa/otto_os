import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';
import { loadErrorText } from '../src/lib/loadError.ts';

function fixture(overrides: Record<string, unknown> = {}) {
  const pageReads: string[] = [];
  const writes: { id: string; url?: string; title?: string }[] = [];
  const api = {
    listTabs: async (ws: string) => [{ id: `${ws}-existing`, url: `https://${ws}.test`, mode: 'reader' }],
    listAnnotations: async () => [],
    getPage: async (_ws: string, url: string) => { pageReads.push(url); return { url, title: url, markdown: '' }; },
    navigateTab: async (id: string, patch: Record<string, string>) => {
      writes.push({ id, ...patch }); return { id, url: 'https://created.test', ...patch };
    },
    ...overrides,
  };
  const { browser } = loadSource(new URL('../src/lib/stores/browser.svelte.ts', import.meta.url), {
    '../loadError': { loadErrorText }, '../api/browser': api, '../nativeBrowser': { nativeBrowserAvailable: false },
    './browserLive.svelte': { browserLive: {} }, '../lazyModule': { announceModule() {} },
  });
  return { browser, pageReads, writes };
}

for (const method of ['openTab', 'openLiveTab']) {
  for (const returnToOrigin of [false, true]) {
    test(`${method} completion cannot enter a ${returnToOrigin ? 'new visit to the original' : 'different'} workspace`, async () => {
      const created = deferred<unknown>();
      const { browser, pageReads } = fixture({ createTab: () => created.promise });
      await browser.loadTabs('A');
      const pending = browser[method]('https://created.test');
      await browser.loadTabs('B');
      if (returnToOrigin) await browser.loadTabs('A');
      const readsBeforeCompletion = pageReads.length;
      const selected = browser.activeId;
      created.resolve({ id: 'A-created', url: 'https://created.test', mode: 'reader' });
      await pending;
      assert.equal(browser.activeId, selected, 'a stale creation must not steal current selection');
      assert.equal(browser.tabs.some((tab: { id: string }) => tab.id === 'A-created'), false);
      assert.equal(pageReads.length, readsBeforeCompletion, 'stale creation must not fetch its URL into the current workspace');
    });
  }
}

test('reader navigation never persists a title obtained from another active tab', async () => {
  const a = deferred<unknown>();
  const { browser, writes } = fixture({
    getPage: (_ws: string, url: string) => url.includes('a-new') ? a.promise : Promise.resolve({ url, title: 'Title B' }),
  });
  await browser.loadTabs('A');
  browser.tabs.push({ id: 'B-tab', url: 'https://b.test', mode: 'reader' });
  const pending = browser.navigate('https://a-new.test');
  browser.select('B-tab');
  // Await a real B reader refresh, rather than assuming a number of microtasks
  // for select()'s fire-and-forget request and its Promise.all continuation.
  await browser.loadPage('https://b.test');
  assert.equal(browser.page.title, 'Title B');
  a.resolve({ url: 'https://a-new.test', title: 'Title A' });
  await pending;
  for (const write of writes.filter((write) => write.id === 'A-existing')) {
    assert.equal(write.title, 'Title A', 'A may never adopt B reader metadata');
  }
  assert.equal(browser.page.title, 'Title B');
});

test('older reader navigation completing last cannot replace a newer URL', async () => {
  const old = deferred<unknown>();
  const { browser, writes } = fixture({
    getPage: (_ws: string, url: string) => url.includes('old') ? old.promise : Promise.resolve({ url, title: 'New title' }),
  });
  await browser.loadTabs('A');
  const first = browser.navigate('https://old.test');
  await browser.navigate('https://new.test');
  old.resolve({ url: 'https://old.test', title: 'Old title' });
  await first;
  assert.equal(browser.activeTab.url, 'https://new.test');
  assert.equal(browser.page.title, 'New title');
  assert.equal(writes.at(-1)?.url, 'https://new.test', 'the superseded request must not persist last');
});

test('overlapping live navigations persist the newest URL even when the first write is slow', async () => {
  const firstWrite = deferred<void>();
  let savedUrl = '';
  const { browser } = fixture({ navigateTab: async (id: string, patch: { url: string; title: string }) => {
    if (patch.url === 'https://old.test') await firstWrite.promise;
    savedUrl = patch.url;
    return { id, mode: 'live', ...patch };
  } });
  await browser.loadTabs('A'); browser.tabs[0].mode = 'live';
  const old = browser.navigate('https://old.test');
  // Let the first transport start before submitting the next navigation.
  await Promise.resolve();
  const newer = browser.navigate('https://new.test');
  await Promise.resolve();
  firstWrite.resolve();
  await Promise.all([old, newer]);
  assert.equal(savedUrl, 'https://new.test', 'response ownership alone does not protect persisted state');
  assert.equal(browser.activeTab.url, 'https://new.test');
});

for (const outcome of ['success', 'failure']) {
  test(`closing the last active tab invalidates a late reader ${outcome} before close completes`, async () => {
    const page = deferred<unknown>();
    const closing = deferred<void>();
    const { browser, writes } = fixture({ getPage: () => page.promise, closeTab: () => closing.promise,
      listAnnotations: async () => [{ id: 'closed-mark' }] });
    await browser.loadTabs('A');
    const reading = browser.navigate('https://closed.test');
    const close = browser.closeTab(browser.activeId);
    if (outcome === 'success') page.resolve({ title: 'Closed content', markdown: 'Closed body' });
    else page.reject(new Error('Closed page failure'));
    await reading;
    const duringClose = { page: browser.page, error: browser.pageError, loading: browser.loadingPage, annotations: browser.annotations.length };
    closing.resolve(); await close;
    assert.equal(duringClose.page, null, 'invalidation must happen before the close HTTP response');
    assert.equal(duringClose.error, '');
    assert.equal(duringClose.loading, false);
    assert.equal(duringClose.annotations, 0);
    assert.equal(browser.activeId, null);
    assert.equal(browser.page, null);
    assert.equal(browser.pageError, '');
    assert.equal(browser.loadingPage, false);
    assert.equal(browser.annotations.length, 0);
    assert.equal(writes.length, 0, 'navigation must not patch the closing tab');
  });
}

test('closing a background tab leaves the active reader request valid', async () => {
  const page = deferred<unknown>();
  const { browser } = fixture({ getPage: () => page.promise, closeTab: async () => {},
    listAnnotations: async () => [{ id: 'active-mark' }] });
  await browser.loadTabs('A');
  browser.tabs.push({ id: 'background', url: 'https://background.test', mode: 'reader' });
  const reading = browser.navigate('https://active.test');
  await browser.closeTab('background');
  page.resolve({ title: 'Active content', markdown: 'Active body' }); await reading;
  assert.equal(browser.activeId, 'A-existing');
  assert.equal(browser.page.markdown, 'Active body');
  assert.equal(browser.annotations[0].id, 'active-mark');
  assert.equal(browser.loadingPage, false);
});

test('failed active close restores the previous readable content and selection', async () => {
  const { browser } = fixture({ closeTab: async () => { throw new Error('Close unavailable'); } });
  await browser.loadTabs('A');
  browser.page = { title: 'Original page', markdown: 'Keep this body' };
  browser.annotations = [{ id: 'original-mark' }];
  await assert.rejects(browser.closeTab('A-existing'), /Close unavailable/);
  assert.equal(browser.activeId, 'A-existing');
  assert.equal(browser.page.markdown, 'Keep this body');
  assert.equal(browser.annotations[0].id, 'original-mark');
  assert.equal(browser.tabs.length, 1);
});
