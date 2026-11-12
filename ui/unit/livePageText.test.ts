import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

function load(opts: { native: boolean; renderer: string; result: unknown }) {
  const evals: { id: string; js: string }[] = [];
  const m = loadSource(new URL('../src/modules/browser/livePageText.ts', import.meta.url), {
    '../../lib/nativeBrowser': {
      nativeBrowserAvailable: opts.native,
      nativeBrowser: { eval: async (id: string, js: string) => { evals.push({ id, js }); return opts.result; } },
    },
    '../../lib/stores/browserLive.svelte': { browserLive: { renderer: opts.renderer } },
  });
  return { m, evals };
}

const live = { id: 't1', mode: 'live' };

test('a live tab in the desktop webview sends its visible text', async () => {
  const { m, evals } = load({ native: true, renderer: 'native', result: JSON.stringify('  Inbox\n3 unread ') });
  assert.equal(await m.livePageText(live), 'Inbox\n3 unread');
  assert.equal(evals.length, 1);
  assert.equal(evals[0].id, 't1');
  assert.match(evals[0].js, /innerText/);
  assert.match(evals[0].js, /slice\(0,30000\)/);
});

test('raw (unquoted) eval results are taken as they are, and capped', async () => {
  const { m } = load({ native: true, renderer: 'native', result: 'y'.repeat(40_000) });
  assert.equal((await m.livePageText(live)).length, m.LIVE_TEXT_MAX);
  assert.equal(m.decodeEvalText('{"a":1}'), '{"a":1}');
  assert.equal(m.decodeEvalText(undefined), '');
});

test('reader tabs, the streamed renderer, the web build and an empty page send nothing', async () => {
  for (const [opts, tab] of [
    [{ native: true, renderer: 'native', result: '"x"' }, { id: 't1', mode: 'reader' }],
    [{ native: true, renderer: 'remote', result: '"x"' }, live],
    [{ native: false, renderer: 'native', result: '"x"' }, live],
    [{ native: true, renderer: 'native', result: undefined }, live],
    [{ native: true, renderer: 'native', result: '"  "' }, live],
  ] as const) {
    const { m } = load(opts);
    assert.equal(await m.livePageText(tab), undefined, JSON.stringify([opts, tab]));
  }
  const { m } = load({ native: true, renderer: 'native', result: '"x"' });
  assert.equal(await m.livePageText(null), undefined);
});
