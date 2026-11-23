// Feature Edge tests for Git → IDE / Browser quick-jump, IDE Dev-Server → Browser
// deep-links, and the unified BrowserView deep-link consumer (`consumeBrowserDeepLink`).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildBrowserRoute, buildIdeRoute, detectPreviewUrls } from '../src/modules/git/gitQuickJump.ts';
import {
  buildBrowserPreviewHash,
  buildDevBrowserRoute,
  detectDevServerUrls,
} from '../src/modules/ide/devices/ideDevServerDetect.ts';
import { parseBrowserDeepLink } from '../src/lib/browserDeepLink.ts';
import { loadSource } from './sourceHarness.ts';
import type { PrChecksResp } from '../src/lib/api/types/git-merge.ts';

test('buildIdeRoute builds normalized root/file/line query and blocks traversal', () => {
  const route = buildIdeRoute('/work/app/', 'src/main.ts', 42);
  assert.ok(route);
  const q = new URLSearchParams(route.slice('ide?'.length));
  assert.equal(q.get('root'), '/work/app');
  assert.equal(q.get('file'), '/work/app/src/main.ts');
  assert.equal(q.get('line'), '42');

  const noLine = buildIdeRoute('/work/app', './pkg/util.go', null);
  assert.ok(noLine);
  const q2 = new URLSearchParams(noLine.slice('ide?'.length));
  assert.equal(q2.get('file'), '/work/app/pkg/util.go');
  assert.equal(q2.get('line'), null);

  assert.equal(buildIdeRoute('/work/app', '../../etc/passwd', 1), null);
  assert.equal(buildIdeRoute('', 'src/a.ts', 1), null);
  assert.equal(buildIdeRoute('/work/app', '', 1), null);
});

test('detectPreviewUrls extracts deployment previews, ranks success first, and ignores CI log links', () => {
  const checks: PrChecksResp = {
    ci: { state: 'success', total: 5, passed: 4, failed: 0, url: 'https://github.com/org/repo/actions/runs/1' },
    checks: [
      { name: 'CI / test', state: 'success', url: 'https://github.com/org/repo/actions/runs/12345' },
      { name: 'Deploy Preview (staging)', state: 'pending', url: 'https://staging.myapp.dev/pr-9' },
      { name: 'Vercel', state: 'success', url: 'https://myapp-pr-9.vercel.app/' },
      { name: 'Vercel duplicate', state: 'neutral', url: 'https://myapp-pr-9.vercel.app' },
      { name: 'Chromatic Storybook', state: 'success', url: 'https://abc123.chromatic.com' },
      { name: 'Evil preview', state: 'success', url: 'javascript:alert(1)' },
    ],
  };
  const previews = detectPreviewUrls(checks);
  assert.deepEqual(
    previews.map((p) => ({ name: p.name, url: p.url })),
    [
      { name: 'Vercel', url: 'https://myapp-pr-9.vercel.app/' },
      { name: 'Chromatic Storybook', url: 'https://abc123.chromatic.com/' },
      { name: 'Deploy Preview (staging)', url: 'https://staging.myapp.dev/pr-9' },
    ],
  );
  assert.deepEqual(detectPreviewUrls(null), []);
});

test('buildBrowserRoute and parseBrowserDeepLink round-trip safe http(s) URLs and reject unsafe schemes', () => {
  const route = buildBrowserRoute('https://myapp-pr-9.vercel.app/dashboard?tab=1', 'live');
  assert.ok(route && route.startsWith('browser?'));
  assert.deepEqual(parseBrowserDeepLink(`#/${route}`), {
    url: 'https://myapp-pr-9.vercel.app/dashboard?tab=1',
    mode: 'live',
  });

  const reader = buildBrowserRoute('https://docs.example.com', 'reader');
  assert.ok(reader);
  assert.deepEqual(parseBrowserDeepLink(`#/${reader}`), {
    url: 'https://docs.example.com/',
    mode: 'reader',
  });

  assert.equal(buildBrowserRoute('javascript:alert(1)'), null);
  assert.equal(parseBrowserDeepLink('#/browser?url=file:///etc/passwd&mode=live')?.url, undefined);
  assert.equal(parseBrowserDeepLink('#/browser?url=file:///etc/passwd'), null);
  assert.equal(parseBrowserDeepLink('#/browser?url=javascript:alert(1)'), null);
  assert.equal(parseBrowserDeepLink('#/browser'), null);
});

test('detectDevServerUrls parses ANSI Vite/Go/Next output, normalizes 0.0.0.0, and builds deep-link hashes', () => {
  const output = [
    '\x1b[32m  VITE v6.0.0\x1b[0m  ready in 210 ms',
    '  ➜  Local:   \x1b[36mhttp://localhost:5173/\x1b[0m',
    '  ➜  Network: http://192.168.1.40:5173/',
    'Listening on http://0.0.0.0:8080/api.',
    'Docs available at http://127.0.0.1:3000/docs,',
  ];
  const detected = detectDevServerUrls(output);
  assert.deepEqual(
    detected.map((d) => d.url),
    [
      'http://127.0.0.1:3000/docs',
      'http://127.0.0.1:8080/api',
      'http://localhost:5173/',
    ],
  );
  assert.equal(
    buildDevBrowserRoute('http://0.0.0.0:8080/api'),
    'browser?url=http%3A%2F%2F127.0.0.1%3A8080%2Fapi&mode=live',
  );
  assert.equal(
    buildBrowserPreviewHash({ url: 'http://127.0.0.1:8080/api' }),
    '#/browser?url=http%3A%2F%2F127.0.0.1%3A8080%2Fapi&mode=live',
  );
  assert.deepEqual(detectDevServerUrls(['GET https://api.github.com/repos/a/b 200']), []);
});

test('consumeBrowserDeepLink handles Git PR and IDE dev-server links with liveCapable fallback, tab reuse, mode=reader, and openUrl errors', async () => {
  const tabs: Array<{ id: string; url: string; mode: 'reader' | 'live' }> = [
    { id: 'tab-existing', url: 'https://docs.example.com/', mode: 'reader' },
  ];
  let activeId: string | null = 'tab-existing';
  const openedLive: string[] = [];
  const openedReader: string[] = [];
  const navigated: string[] = [];
  const selected: string[] = [];
  const modeUpdates: Array<[string, string]> = [];
  const errors: string[] = [];
  let failSimOpenUrl = false;

  const fakeWindow = { location: { hash: '#/browser' } };
  const browserStub = {
    get tabs() { return tabs; },
    get activeTab() { return tabs.find((t) => t.id === activeId) ?? null; },
    select: (id: string) => {
      activeId = id;
      selected.push(id);
    },
    openLiveTab: async (url: string) => {
      openedLive.push(url);
      const t = { id: `live-${openedLive.length}`, url, mode: 'live' as const };
      tabs.push(t);
      activeId = t.id;
    },
    openTab: async (url: string) => {
      openedReader.push(url);
      const t = { id: `reader-${openedReader.length}`, url, mode: 'reader' as const };
      tabs.push(t);
      activeId = t.id;
    },
    navigate: async (url: string) => { navigated.push(url); },
    setMode: async (id: string, mode: 'reader' | 'live') => {
      modeUpdates.push([id, mode]);
      const found = tabs.find((t) => t.id === id);
      if (found) found.mode = mode;
    },
  };

  const mod = loadSource(
    new URL('../src/modules/browser/deepLink.ts', import.meta.url),
    {
      'lib/api/simulators': {
        simulatorsApi: {
          getDevice: async () => ({
            id: 'ios:BOOTED',
            state: 'booted',
            capabilities: { open_url: true },
          }),
          openUrl: async () => {
            if (failSimOpenUrl) throw new Error('simctl failed');
            return { effective_url: 'http://127.0.0.1:8080/api', forwarded_ports: [] };
          },
        },
      },
      'lib/nativeBrowser': { nativeBrowserAvailable: false },
      'lib/router.svelte': {
        router: {
          replace: (path: string) => { fakeWindow.location.hash = `#/${path}`; },
        },
      },
      'lib/stores/browser.svelte': { browser: browserStub },
      'lib/stores/browserLive.svelte': { browserLive: { renderer: 'remote', setPref: () => {} } },
      'lib/toastError': { toastError: (title: string) => { errors.push(title); } },
    },
    { window: fakeWindow },
    { relative: true },
  );

  const baseCb = {
    setUrlInput: () => {},
    setEmulation: () => {},
    onSimulator: () => {},
    onSimulatorMissing: () => {},
  };

  // 1. Git PR preview with liveCapable = false falls back to openTab (Reader), never openLiveTab.
  const prLive = buildBrowserRoute('https://myapp-pr-9.vercel.app/dashboard?tab=1', 'live');
  assert.ok(prLive);
  fakeWindow.location.hash = `#/${prLive}`;
  await mod.consumeBrowserDeepLink({ ...baseCb, liveCapable: false });
  assert.deepEqual(openedLive, []);
  assert.deepEqual(openedReader, ['https://myapp-pr-9.vercel.app/dashboard?tab=1']);
  assert.deepEqual(navigated, [], 'deep-link must not clobber active tab via navigate');

  // 2. mode=reader for a new URL opens a new Reader tab via openTab (does not clobber active tab).
  const prReader = buildBrowserRoute('https://guide.example.com/intro', 'reader');
  assert.ok(prReader);
  fakeWindow.location.hash = `#/${prReader}`;
  await mod.consumeBrowserDeepLink({ ...baseCb, liveCapable: true });
  assert.deepEqual(openedReader, [
    'https://myapp-pr-9.vercel.app/dashboard?tab=1',
    'https://guide.example.com/intro',
  ]);

  // 3. Re-opening an already-open tab focuses it and upgrades mode to live instead of duplicating.
  const existingRoute = buildBrowserRoute('https://docs.example.com', 'live');
  assert.ok(existingRoute);
  fakeWindow.location.hash = `#/${existingRoute}`;
  await mod.consumeBrowserDeepLink({ ...baseCb, liveCapable: true });
  assert.deepEqual(selected, ['tab-existing']);
  assert.deepEqual(modeUpdates, [['tab-existing', 'live']]);
  assert.equal(openedLive.length, 0, 'existing tab must be reused, not duplicated');

  // 4. IDE dev-server route (via buildDevBrowserRoute) opens a live tab when liveCapable = true.
  const devRoute = buildDevBrowserRoute('http://0.0.0.0:8080/api');
  assert.ok(devRoute);
  fakeWindow.location.hash = `#/${devRoute}`;
  await mod.consumeBrowserDeepLink({ ...baseCb, liveCapable: true });
  assert.deepEqual(openedLive, ['http://127.0.0.1:8080/api']);

  // 5. Simulator openUrl failure surfaces via toastError instead of being swallowed.
  failSimOpenUrl = true;
  fakeWindow.location.hash = buildBrowserPreviewHash({
    url: 'http://127.0.0.1:8080/api',
    device: 'simulator:ios:BOOTED',
  });
  await mod.consumeBrowserDeepLink({ ...baseCb, liveCapable: true });
  await new Promise((r) => setTimeout(r, 0));
  assert.ok(errors.includes('Couldn’t open URL on simulator'));
});
