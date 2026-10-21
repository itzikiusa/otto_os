// test-quality-1-08: the report-frame link bridge and the plugin frame's
// trust rules, tested by BEHAVIOUR (they used to be pinned by regexes over the
// component source, which pass with the check in a comment or after the post).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { OPEN_GAP_MS, reportLinkBridge } from '../src/modules/insights/reportFrame.ts';
import { pluginFrameAction, pluginSlugOk, postPluginInit } from '../src/modules/plugins/pluginFrame.ts';

const frame = { name: 'report-frame' };
const other = { name: 'some-other-window' };
const open = (url: string) => ({ source: frame, data: { type: 'otto-insights-open', url } });

function bridge(opts: { active?: boolean | undefined } = {}) {
  let t = 10_000;
  const opened: string[] = [];
  const onMessage = reportLinkBridge({
    frameWindow: () => frame,
    open: (u) => void opened.push(u),
    now: () => t,
    userActive: () => ('active' in opts ? opts.active : true),
  });
  return { onMessage, opened, advance: (ms: number) => void (t += ms) };
}

test('report bridge: opens an http(s) link from its own frame on a gesture', () => {
  const b = bridge();
  assert.equal(b.onMessage(open('https://example.com/a')), true);
  assert.deepEqual(b.opened, ['https://example.com/a']);
});

test('report bridge: a foreign source is ignored', () => {
  const b = bridge();
  assert.equal(b.onMessage({ source: other, data: open('https://x.dev').data }), false);
  assert.equal(b.onMessage({ source: null, data: open('https://x.dev').data }), false);
  assert.deepEqual(b.opened, []);
});

test('report bridge: no frame mounted yet means nothing opens', () => {
  const opened: string[] = [];
  const onMessage = reportLinkBridge({ frameWindow: () => null, open: (u) => void opened.push(u) });
  assert.equal(onMessage({ source: null, data: open('https://x.dev').data }), false);
  assert.deepEqual(opened, []);
});

test('report bridge: non-http(s) URLs are refused', () => {
  const b = bridge();
  for (const url of ['javascript:alert(1)', 'file:///etc/passwd', 'otto://settings', 'data:text/html,x', '//evil.com', ' https://x.dev']) {
    assert.equal(b.onMessage(open(url)), false, url);
  }
  assert.equal(b.onMessage({ source: frame, data: { type: 'other', url: 'https://x.dev' } }), false);
  assert.equal(b.onMessage({ source: frame, data: { type: 'otto-insights-open', url: 42 } }), false);
  assert.deepEqual(b.opened, []);
});

test('report bridge: no user gesture means no open; a browser without the API is not blocked', () => {
  const idle = bridge({ active: false });
  assert.equal(idle.onMessage(open('https://x.dev')), false);
  assert.deepEqual(idle.opened, []);
  const noApi = bridge({ active: undefined });
  assert.equal(noApi.onMessage(open('https://x.dev')), true);
});

test('report bridge: a second open inside OPEN_GAP_MS is refused, after it allowed', () => {
  const b = bridge();
  assert.equal(b.onMessage(open('https://a.dev')), true);
  b.advance(OPEN_GAP_MS - 1);
  assert.equal(b.onMessage(open('https://b.dev')), false);
  b.advance(1);
  assert.equal(b.onMessage(open('https://c.dev')), true);
  assert.deepEqual(b.opened, ['https://a.dev', 'https://c.dev']);
});

test('plugin slugs: plain identifiers only', () => {
  for (const ok of ['team-performance', 'a', 'x1-y2']) assert.equal(pluginSlugOk(ok), true, ok);
  for (const bad of ['', '..', '../ui', '%2e%2e', 'A', '1abc', 'a/b', 'a.b', 'a'.repeat(65)]) {
    assert.equal(pluginSlugOk(bad), false, bad);
  }
});

test('plugin init: an invalid slug never gets otto:init (or the token)', () => {
  const posts: Record<string, unknown>[] = [];
  const deps = { baseUrl: 'http://127.0.0.1:7700', token: 'secret-token', providers: [] };
  for (const bad of ['..', '%2e%2e', '../ui', '']) {
    assert.equal(postPluginInit(bad, (e) => void posts.push(e), deps), false, bad);
  }
  // Not deepEqual(posts, []): its assertion signature would narrow `posts` to never[].
  assert.equal(posts.length, 0);
  assert.equal(postPluginInit('team-performance', (e) => void posts.push(e), deps), true);
  assert.equal(posts.length, 1);
  assert.equal(posts[0].token, 'secret-token');
  assert.equal(posts[0].apiBase, 'http://127.0.0.1:7700/api/v1/plugins/team-performance');
});

test('plugin frame: messages from another source are ignored', () => {
  const pluginWin = { name: 'plugin' };
  const keydown = { type: 'otto:keydown', key: 'k', metaKey: true };
  assert.equal(pluginFrameAction({ source: other, data: keydown }, pluginWin), null);
  assert.equal(pluginFrameAction({ source: null, data: keydown }, pluginWin), null);
  assert.equal(pluginFrameAction({ source: pluginWin, data: keydown }, undefined), null);
  assert.deepEqual(pluginFrameAction({ source: pluginWin, data: keydown }, pluginWin), { kind: 'keydown', init: keydown });
  assert.deepEqual(pluginFrameAction({ source: pluginWin, data: { type: 'otto:focus-leave' } }, pluginWin), { kind: 'focus-leave' });
  assert.equal(pluginFrameAction({ source: pluginWin, data: { type: 'otto:keydown', key: 1 } }, pluginWin), null);
  assert.equal(pluginFrameAction({ source: pluginWin, data: null }, pluginWin), null);
});
