// Native live tabs: a re-mounted Browser view (the v2 side panel re-mounts on
// every panel-tab switch) must only SHOW an existing webview, never navigate it
// again — that reloaded the page and lost its scroll, form and app state.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { liveWebviews, NATIVE_NAV_JS, sameLiveUrl } from '../src/modules/browser/liveWebviews.ts';

test('a webview that already shows the tab URL is only shown, across view re-mounts', () => {
  assert.equal(liveWebviews.action('t1', 'https://a.example'), 'navigate');
  liveWebviews.set('t1', 'https://a.example');
  // The module outlives any one BrowserView: a fresh view asks the same map.
  assert.equal(liveWebviews.action('t1', 'https://a.example'), 'show');
  assert.equal(liveWebviews.action('t1', 'https://a.example/'), 'show', 'normalised');
  assert.equal(liveWebviews.action('t1', 'https://a.example/next'), 'navigate', 'the tab moved on');
  // An in-page navigation reported by the shell is what the webview shows now.
  liveWebviews.set('t1', 'https://a.example/next');
  assert.equal(liveWebviews.action('t1', 'https://a.example/next'), 'show');
  // A closed webview is opened afresh.
  liveWebviews.forget('t1');
  assert.equal(liveWebviews.action('t1', 'https://a.example/next'), 'navigate');
});

test('the most-recently-shown cap evicts only opened webviews beyond it', () => {
  for (const id of ['m1', 'm2', 'm3']) liveWebviews.set(id, `https://${id}.example`);
  assert.deepEqual(liveWebviews.touch('m1', 2), []);
  assert.deepEqual(liveWebviews.touch('m2', 2), []);
  assert.deepEqual(liveWebviews.touch('m3', 2), ['m1']);
  assert.equal(liveWebviews.action('m1', 'https://m1.example'), 'navigate', 'evicted is forgotten');
  assert.equal(liveWebviews.action('m3', 'https://m3.example'), 'show');
  // Re-activating the evicted tab never evicts the tab being shown.
  liveWebviews.set('m1', 'https://m1.example');
  assert.deepEqual(liveWebviews.touch('m1', 2), ['m2']);
  for (const id of ['m1', 'm2', 'm3']) liveWebviews.forget(id);
});

test('url comparison tolerates whitespace and unparsable input', () => {
  assert.equal(sameLiveUrl(undefined, 'https://a.example'), false);
  assert.equal(sameLiveUrl(' https://a.example ', 'https://a.example/'), true);
  assert.equal(sameLiveUrl('not a url', 'not a url'), true);
  assert.equal(sameLiveUrl('https://a.example/#x', 'https://a.example/'), false);
});

test('native history and stop commands are plain page scripts', () => {
  assert.deepEqual(NATIVE_NAV_JS, { back: 'history.back()', forward: 'history.forward()', stop: 'window.stop()' });
});
