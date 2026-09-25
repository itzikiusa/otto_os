// Side-by-side pane pure helpers (node:test, Node's built-in type stripping —
// the `.ts` extension on the import is required in strip mode).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  PANE_MIN_PX,
  SIDE_NS,
  SPLIT_DEFAULT,
  SPLIT_MAX,
  SPLIT_MIN,
  clampShare,
  embedSrc,
  embeddedKeyTarget,
  fitsTwoPanes,
  isEmbedSearch,
  leadingFromPointer,
  leadingShare,
  nudgeLeading,
  paneKey,
  parseSaved,
  readGuestMsg,
  readHostMsg,
  restorableRoute,
  routeOf,
  serializeSaved,
  sideMenuTarget,
  sideShareFor,
  targetOrigin,
} from '../src/lib/sidePane.ts';
import { activeNavId } from '../src/lib/sidebar.ts';

test('isEmbedSearch: only embed=1 marks the side pane document', () => {
  assert.equal(isEmbedSearch('?embed=1'), true);
  assert.equal(isEmbedSearch('?win=main&embed=1'), true);
  assert.equal(isEmbedSearch(''), false);
  assert.equal(isEmbedSearch('?embed=0'), false);
  assert.equal(isEmbedSearch('?popout=1'), false);
  assert.equal(isEmbedSearch('?embed=true'), false);
});

test('routeOf strips the hash prefix in every spelling', () => {
  assert.equal(routeOf('#/git/x'), 'git/x');
  assert.equal(routeOf('/git/x'), 'git/x');
  assert.equal(routeOf('git/x'), 'git/x');
  assert.equal(routeOf('#'), '');
});

test('paneKey groups routes by the sidebar entry that owns them', () => {
  assert.equal(paneKey(''), 'agents');
  assert.equal(paneKey('#/'), 'agents');
  assert.equal(paneKey('agents/01HSESSION'), 'agents');
  assert.equal(paneKey('connections/abc'), 'connections');
  assert.equal(paneKey('database/abc'), 'connections');
  assert.equal(paneKey('brokers/c1/topic'), 'connections');
  assert.equal(paneKey('canvas/scene'), 'design');
  assert.equal(paneKey('plugin/jira/x'), 'plugin/jira');
  assert.equal(paneKey('history/abc'), 'history');
  assert.equal(paneKey('settings/appearance'), 'settings');
});

test('paneKey matches the sidebar highlight (activeNavId) for every route shape', () => {
  for (const r of ['', 'agents', 'agents/x', 'git/r/pr/7', 'database/x', 'brokers', 'canvas', 'plugin/a/b', 'home', 'settings/x']) {
    assert.equal(paneKey(r), activeNavId(r === '' ? [] : r.split('/')), r);
  }
});

test('clampShare keeps the split within 25–75% and both panes above the floor', () => {
  assert.equal(clampShare(0.5), 0.5);
  assert.equal(clampShare(0.1), SPLIT_MIN);
  assert.equal(clampShare(0.9), SPLIT_MAX);
  assert.equal(clampShare(Number.NaN), SPLIT_DEFAULT);
  // 1000px wide: 360px min → 0.36..0.64
  assert.equal(clampShare(0.25, 1000), 0.36);
  assert.equal(clampShare(0.75, 1000), 0.64);
  assert.equal(clampShare(0.5, 1000), 0.5);
  // Too narrow for the floor on both sides → 50/50.
  assert.equal(clampShare(0.3, 700), 0.5);
  // Wide: the ratio limits win.
  assert.equal(clampShare(0.1, 4000), SPLIT_MIN);
});

test('fitsTwoPanes needs room for two minimum-width panes and the divider', () => {
  assert.equal(fitsTwoPanes(PANE_MIN_PX * 2), false);
  assert.equal(fitsTwoPanes(PANE_MIN_PX * 2 + 1), true);
  assert.equal(fitsTwoPanes(1200), true);
});

test('leading/side share convert both ways for both placements', () => {
  assert.equal(leadingShare(0.3, 'trailing'), 0.7);
  assert.equal(leadingShare(0.3, 'leading'), 0.3);
  assert.ok(Math.abs(sideShareFor(0.7, 'trailing') - 0.3) < 1e-9);
  assert.equal(sideShareFor(0.3, 'leading'), 0.3);
});

test('leadingFromPointer is logical: mirrored in RTL', () => {
  assert.equal(leadingFromPointer(250, 0, 1000), 0.25);
  assert.equal(leadingFromPointer(250, 0, 1000, true), 0.75);
  assert.equal(leadingFromPointer(100, 100, 100), SPLIT_DEFAULT);
});

test('nudgeLeading: arrows step (visually), Home/End limits, Enter resets', () => {
  assert.equal(nudgeLeading(0.5, 'ArrowRight'), 0.52);
  assert.equal(nudgeLeading(0.5, 'ArrowLeft'), 0.48);
  assert.equal(nudgeLeading(0.5, 'ArrowRight', { shift: true }), 0.6);
  assert.equal(nudgeLeading(0.5, 'ArrowRight', { rtl: true }), 0.48);
  assert.equal(nudgeLeading(0.5, 'Home'), SPLIT_MIN);
  assert.equal(nudgeLeading(0.5, 'End'), SPLIT_MAX);
  assert.equal(nudgeLeading(0.3, 'Enter'), SPLIT_DEFAULT);
  assert.equal(nudgeLeading(0.5, 'ArrowUp'), null);
  assert.equal(nudgeLeading(0.5, 'a'), null);
});

test('saved state: round-trips, clamps, and never restores a one-time route', () => {
  const s = parseSaved(serializeSaved({ route: 'connections/abc', share: 0.4, placement: 'leading' }));
  assert.deepEqual(s, { route: 'connections/abc', share: 0.4, placement: 'leading' });
  assert.deepEqual(parseSaved(null), { route: null, share: SPLIT_DEFAULT, placement: 'trailing' });
  assert.deepEqual(parseSaved('not json'), { route: null, share: SPLIT_DEFAULT, placement: 'trailing' });
  assert.deepEqual(parseSaved('{"route":"#/git","share":2,"placement":"sideways"}'), {
    route: 'git',
    share: SPLIT_MAX,
    placement: 'trailing',
  });
  assert.equal(parseSaved('{"route":"s/sid/token"}').route, null);
  assert.equal(parseSaved('{"route":"snip/1"}').route, null);
  assert.equal(parseSaved('{"route":42}').route, null);
  assert.equal(restorableRoute(''), null);
  assert.equal(restorableRoute('x'.repeat(600)), null);
  assert.equal(restorableRoute('#/vault/notes'), 'vault/notes');
});

test('embedSrc: same page, embed flag, no window identity, the route in the hash', () => {
  assert.equal(embedSrc({ pathname: '/', search: '' }, 'connections'), '/?embed=1#/connections');
  assert.equal(
    embedSrc({ pathname: '/index.html', search: '?popout=1&win=popout-2&x=1' }, '#/git/r1'),
    '/index.html?x=1&embed=1#/git/r1',
  );
  assert.equal(embedSrc({ pathname: '', search: '' }, 'home'), '/?embed=1#/home');
});

test('embeddedKeyTarget: window verbs go to the host, pane verbs stay', () => {
  for (const a of ['palette', 'askOtto', 'settings', 'newSession', 'toggleRail', 'broadcast', 'snip', 'hardReload', 'appZoomIn', 'toggleSidePane', 'shortcuts']) {
    assert.equal(embeddedKeyTarget(a, 'connections'), 'host', a);
  }
  for (const a of ['find', 'navBack', 'navForward', 'termZoomIn']) {
    assert.equal(embeddedKeyTarget(a, 'connections'), 'local', a);
  }
  // Session verbs: the pane's own on Agents…
  for (const a of ['closeTab', 'nextTab', 'jumpSession', 'splitVertical', 'toggleRight']) {
    assert.equal(embeddedKeyTarget(a, 'agents'), 'local', a);
  }
  // …elsewhere ⌘W closes the pane and the rest are the window's.
  assert.equal(embeddedKeyTarget('closeTab', 'git'), 'close-pane');
  assert.equal(embeddedKeyTarget('nextTab', 'git'), 'host');
  assert.equal(embeddedKeyTarget('toggleRight', 'git'), 'host');
});

test('sideMenuTarget: only a focused pane takes the pane-scoped menu items', () => {
  assert.equal(sideMenuTarget('close-tab', false, 'git'), 'main');
  assert.equal(sideMenuTarget('close-tab', true, null), 'main');
  assert.equal(sideMenuTarget('select-all', true, 'git'), 'side');
  assert.equal(sideMenuTarget('close-tab', true, 'git'), 'close-pane');
  assert.equal(sideMenuTarget('close-tab', true, 'agents'), 'side');
  assert.equal(sideMenuTarget('session-kill', true, 'agents'), 'side');
  assert.equal(sideMenuTarget('session-kill', true, 'git'), 'main');
  assert.equal(sideMenuTarget('settings', true, 'agents'), 'main');
  assert.equal(sideMenuTarget('zoom-in', true, 'git'), 'main');
});

test('readGuestMsg validates everything the host acts on', () => {
  assert.deepEqual(readGuestMsg({ ns: SIDE_NS, type: 'route', route: '#/git/r' }), { ns: SIDE_NS, type: 'route', route: 'git/r' });
  assert.deepEqual(readGuestMsg({ ns: SIDE_NS, type: 'key', action: 'palette' }), { ns: SIDE_NS, type: 'key', action: 'palette' });
  assert.deepEqual(readGuestMsg({ ns: SIDE_NS, type: 'key', action: 'jumpSession', index: 3 }), {
    ns: SIDE_NS,
    type: 'key',
    action: 'jumpSession',
    index: 3,
  });
  assert.deepEqual(readGuestMsg({ ns: SIDE_NS, type: 'close' }), { ns: SIDE_NS, type: 'close' });
  assert.deepEqual(readGuestMsg({ ns: SIDE_NS, type: 'swap' }), { ns: SIDE_NS, type: 'swap' });
  assert.deepEqual(readGuestMsg({ ns: SIDE_NS, type: 'promote' }), { ns: SIDE_NS, type: 'promote' });
  assert.equal(readGuestMsg({ ns: 'other', type: 'close' }), null);
  assert.equal(readGuestMsg({ ns: SIDE_NS, type: 'route' }), null);
  assert.equal(readGuestMsg({ ns: SIDE_NS, type: 'nope' }), null);
  assert.equal(readGuestMsg(null), null);
  assert.equal(readGuestMsg('x'), null);
  // Only web links are handed to the system browser.
  assert.equal(readGuestMsg({ ns: SIDE_NS, type: 'open-external', url: 'file:///etc/passwd' }), null);
  assert.equal(readGuestMsg({ ns: SIDE_NS, type: 'open-external', url: 'javascript:alert(1)' }), null);
  assert.deepEqual(readGuestMsg({ ns: SIDE_NS, type: 'open-external', url: 'https://x.dev' }), {
    ns: SIDE_NS,
    type: 'open-external',
    url: 'https://x.dev',
  });
  // Commands: malformed rows are dropped, the rest kept.
  const cmds = readGuestMsg({
    ns: SIDE_NS,
    type: 'commands',
    list: [{ id: 'a', title: 'A', group: 'G' }, { id: 1 }, null, { id: 'b', title: 'B', shortcut: '⌘B', extra: 1 }],
  });
  assert.deepEqual(cmds, {
    ns: SIDE_NS,
    type: 'commands',
    list: [
      { id: 'a', title: 'A', group: 'G', detail: undefined, keywords: undefined, shortcut: undefined },
      { id: 'b', title: 'B', group: undefined, detail: undefined, keywords: undefined, shortcut: '⌘B' },
    ],
  });
});

test('readHostMsg validates what the pane acts on', () => {
  assert.deepEqual(readHostMsg({ ns: SIDE_NS, type: 'navigate', route: '/vault' }), { ns: SIDE_NS, type: 'navigate', route: 'vault' });
  assert.deepEqual(readHostMsg({ ns: SIDE_NS, type: 'menu', id: 'select-all' }), { ns: SIDE_NS, type: 'menu', id: 'select-all' });
  assert.deepEqual(readHostMsg({ ns: SIDE_NS, type: 'host', primary: 'agents', padTraffic: true }), {
    ns: SIDE_NS,
    type: 'host',
    primary: 'agents',
    padTraffic: true,
  });
  assert.equal((readHostMsg({ ns: SIDE_NS, type: 'host', primary: 'git', padTraffic: 'yes' }) as { padTraffic: boolean }).padTraffic, false);
  assert.equal(readHostMsg({ ns: SIDE_NS, type: 'navigate', route: 5 }), null);
  assert.equal(readHostMsg({ ns: SIDE_NS, type: 'ready', route: 'x' }), null);
});

test('targetOrigin: the real origin, or * for an opaque one', () => {
  assert.equal(targetOrigin('tauri://localhost'), 'tauri://localhost');
  assert.equal(targetOrigin('http://127.0.0.1:7700'), 'http://127.0.0.1:7700');
  assert.equal(targetOrigin('null'), '*');
  assert.equal(targetOrigin(undefined), '*');
});
