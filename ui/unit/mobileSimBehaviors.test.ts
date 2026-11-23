// Behavioral unit tests for mobile/tablet shell controls, Browser deep-links &
// bookmarks, binary screencast framing, shared simulator installer store, and
// IDE Devices state (TEST-4 / FE-8).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import { loadSource, deferred } from './sourceHarness.ts';
import {
  BROWSER_DEVICE_PRESET_IDS,
  buildBrowserDeepLink,
  isBrowserDevicePresetId,
  isHttpUrl,
  isSimulatorDeepLinkUrl,
  parseBrowserDeepLink,
} from '../src/lib/browserDeepLink.ts';
import { BINARY_FRAME_VERSION, decodeBinaryFrame } from '../src/lib/api/binaryFrame.ts';

function makeDevice(overrides: Record<string, unknown> = {}) {
  return {
    id: 'ios:UDID-1',
    platform: 'ios',
    name: 'iPhone 16 Pro',
    os_version: '18.2',
    state: 'shutdown',
    form_factor: 'phone',
    orientation: 'portrait',
    appearance: 'light',
    capabilities: {
      boot: true,
      shutdown: true,
      open_url: true,
      screenshot: true,
      stream: true,
      rotate: true,
      appearance: true,
      port_forward: false,
      logs: true,
      create_delete: true,
    },
    ...overrides,
  };
}

function makeBinaryFrame(headerObj: unknown, payload: Uint8Array, version = BINARY_FRAME_VERSION): ArrayBuffer {
  const headerBytes = new TextEncoder().encode(JSON.stringify(headerObj));
  const buf = new ArrayBuffer(5 + headerBytes.byteLength + payload.byteLength);
  const view = new DataView(buf);
  view.setUint8(0, version);
  view.setUint32(1, headerBytes.byteLength, false);
  const out = new Uint8Array(buf);
  out.set(headerBytes, 5);
  out.set(payload, 5 + headerBytes.byteLength);
  return buf;
}

// ── 1. Shared browserDeepLink contract ────────────────────────────────────

test('browserDeepLink: builds and parses preset, mode, http(s), and simulator exp:// links', () => {
  assert.ok(BROWSER_DEVICE_PRESET_IDS.includes('iphone-16-pro'));
  assert.equal(isBrowserDevicePresetId('iphone-16-pro'), true);
  assert.equal(isBrowserDevicePresetId('not-a-preset'), false);
  assert.equal(isHttpUrl('https://example.com'), true);
  assert.equal(isHttpUrl('javascript:alert(1)'), false);
  assert.equal(isSimulatorDeepLinkUrl('exp://127.0.0.1:8081'), true);

  const hash = buildBrowserDeepLink({
    url: 'http://localhost:5173/app',
    mode: 'live',
    presetId: 'iphone-16-pro',
  });
  assert.deepEqual(parseBrowserDeepLink(hash), {
    url: 'http://localhost:5173/app',
    mode: 'live',
    presetId: 'iphone-16-pro',
  });

  // exp:// is accepted only when targeting a simulator, rejected for web tabs.
  const simHash = buildBrowserDeepLink({
    url: 'exp://127.0.0.1:8081',
    simulatorId: 'ios:ABC',
  });
  assert.deepEqual(parseBrowserDeepLink(simHash), {
    url: 'exp://127.0.0.1:8081',
    simulatorId: 'ios:ABC',
  });
  assert.equal(parseBrowserDeepLink('#/browser?url=exp%3A%2F%2F127.0.0.1%3A8081'), null);

  // Unsafe schemes, unknown presets, and non-browser routes are rejected.
  assert.equal(parseBrowserDeepLink('#/browser?url=javascript%3Aalert(1)&device=bogus'), null);
  assert.equal(parseBrowserDeepLink('#/ide?url=https%3A%2F%2Fexample.com'), null);
  assert.equal(parseBrowserDeepLink('#/browser'), null);
});

// ── 2. Shared binaryFrame decoder ─────────────────────────────────────────

test('decodeBinaryFrame: decodes valid frames and rejects malformed headers/versions', () => {
  const payload = new Uint8Array([0x89, 0x50, 0x4e, 0x47]);
  const valid = makeBinaryFrame({ seq: 7, mime: 'image/png', width: 393, height: 852 }, payload);
  const decoded = decodeBinaryFrame<{ seq: number; mime: string }>(valid, (h) => typeof h.seq === 'number');
  assert.ok(decoded);
  assert.equal(decoded.header.seq, 7);
  assert.equal(decoded.header.mime, 'image/png');
  assert.deepEqual([...decoded.bytes], [0x89, 0x50, 0x4e, 0x47]);

  // Validator rejection
  assert.equal(decodeBinaryFrame<{ seq: number }>(valid, (h) => h.seq === 99), null);
  // Wrong version byte
  assert.equal(decodeBinaryFrame(makeBinaryFrame({ seq: 1 }, payload, 2)), null);
  // Too short
  assert.equal(decodeBinaryFrame(new ArrayBuffer(4)), null);
  // Declared length exceeds buffer
  const truncated = new ArrayBuffer(8);
  const tv = new DataView(truncated);
  tv.setUint8(0, BINARY_FRAME_VERSION);
  tv.setUint32(1, 50, false);
  assert.equal(decodeBinaryFrame(truncated), null);
  // Invalid JSON
  const badJson = new ArrayBuffer(7);
  const bv = new DataView(badJson);
  bv.setUint8(0, BINARY_FRAME_VERSION);
  bv.setUint32(1, 2, false);
  new Uint8Array(badJson, 5).set([0x7b, 0x21]); // "{!"
  assert.equal(decodeBinaryFrame(badJson), null);
});

// ── 3. Viewport tablet rail toggle (FE-8) ─────────────────────────────────

test('viewport.toggleTabletRail folds/unfolds deterministically on tablets', () => {
  const mql = (matches: boolean) => ({
    matches,
    addEventListener: () => {},
    removeEventListener: () => {},
  });
  const mod = loadSource(
    new URL('../src/lib/stores/viewport.svelte.ts', import.meta.url),
    {},
    {
      window: {
        matchMedia: (q: string) => {
          if (q.includes('max-width: 640px')) return mql(false);
          if (q.includes('max-width: 1024px')) return mql(true);
          return mql(false);
        },
      },
    },
  );
  const vp = mod.viewport;
  assert.equal(vp.isTablet, true);
  assert.equal(vp.navDrawer, false);
  assert.equal(vp.tabletNavFolded, null);

  vp.toggleTabletRail();
  assert.equal(vp.tabletNavFolded, true, 'first toggle folds from null');
  vp.toggleTabletRail();
  assert.equal(vp.tabletNavFolded, false, 'second toggle unfolds');
  vp.toggleTabletRail(false);
  assert.equal(vp.tabletNavFolded, true, 'explicit expanded=false folds');
  vp.toggleTabletRail(true);
  assert.equal(vp.tabletNavFolded, false, 'explicit expanded=true unfolds');
});

// ── 4. Browser bookmarks store ────────────────────────────────────────────

test('browserBookmarks: normalizes URLs, dedupes, caps at 40, and isolates workspaces', () => {
  const mod = loadSource(
    new URL('../src/modules/browser/bookmarks.svelte.ts', import.meta.url),
    {},
  );
  assert.equal(mod.normalizeBookmarkUrl('example.com/docs'), 'https://example.com/docs');
  assert.equal(mod.normalizeBookmarkUrl('http://localhost:3000'), 'http://localhost:3000/');
  assert.equal(mod.normalizeBookmarkUrl('javascript:alert(1)'), null);
  assert.equal(mod.normalizeBookmarkUrl('   '), null);
  assert.equal(mod.shortHostLabel('https://localhost:5173/preview'), 'localhost:5173/preview');

  const bm = mod.browserBookmarks;
  // Without a workspace loaded, add returns false.
  assert.equal(bm.add('https://example.com'), false);

  bm.load('ws-alpha');
  assert.equal(bm.add('example.com', 'Example'), true);
  assert.equal(bm.has('https://example.com'), true);
  assert.equal(bm.items.length, 1);
  assert.equal(bm.items[0].title, 'Example');

  // Re-adding the same URL moves it to the front and updates its title without duplicating.
  assert.equal(bm.add('http://localhost:3000', 'Local'), true);
  assert.equal(bm.add('https://example.com/', 'Updated Example'), true);
  assert.equal(bm.items.length, 2);
  assert.equal(bm.items[0].title, 'Updated Example');

  // Toggle removes an existing bookmark and adds a missing one.
  assert.equal(bm.toggle('https://example.com'), false);
  assert.equal(bm.has('https://example.com'), false);
  assert.equal(bm.toggle('https://example.com', 'Back'), true);
  assert.equal(bm.has('https://example.com'), true);

  // Cap at 40 items.
  for (let i = 0; i < 45; i++) {
    bm.add(`https://site-${i}.example.test`, `Site ${i}`);
  }
  assert.equal(bm.items.length, 40);
  assert.equal(bm.items[0].title, 'Site 44');

  // Workspace isolation: switching workspace loads an empty list, returning restores ws-alpha.
  bm.load('ws-beta');
  assert.equal(bm.items.length, 0);
  bm.load('ws-alpha');
  assert.equal(bm.items.length, 40);
});

// ── 5. Browser deep-link consumer ─────────────────────────────────────────

test('consumeBrowserDeepLink: strips hash, activates preset/live tab, and handles simulator handoff', async () => {
  const replaced: string[] = [];
  const liveUrls: string[] = [];
  const readerUrls: string[] = [];
  const modeCalls: Array<[string, string]> = [];
  const openedOnSim: Array<{ id: string; url: string }> = [];
  const prefCalls: string[] = [];
  const errors: string[] = [];
  let simResult: Record<string, unknown> | Error = makeDevice({ id: 'ios:BOOTED', state: 'booted' });

  const fakeWindow = {
    location: { hash: '#/browser?url=https%3A%2F%2Fexample.com&mode=live&device=iphone-16-pro' },
    history: {
      replaceState: (_s: unknown, _t: string, url: string) => {
        replaced.push(url);
        fakeWindow.location.hash = url;
      },
    },
  };

  const browserStub = {
    activeTab: { id: 'tab-1', url: 'https://initial.test', mode: 'reader' },
    openLiveTab: async (url: string) => { liveUrls.push(url); },
    navigate: async (url: string) => { readerUrls.push(url); },
    setMode: async (id: string, mode: string) => { modeCalls.push([id, mode]); },
  };
  const browserLiveStub = {
    renderer: 'native',
    setPref: (p: string) => { prefCalls.push(p); },
  };

  const mod = loadSource(
    new URL('../src/modules/browser/deepLink.ts', import.meta.url),
    {
      'lib/api/simulators': {
        simulatorsApi: {
          getDevice: async () => {
            if (simResult instanceof Error) throw simResult;
            return simResult;
          },
          openUrl: async (id: string, req: { url: string }) => {
            openedOnSim.push({ id, url: req.url });
            return { effective_url: req.url, forwarded_ports: [] };
          },
        },
      },
      'lib/nativeBrowser': { nativeBrowserAvailable: true },
      'lib/router.svelte': {
        router: {
          replace: (path: string) => {
            const hash = `#/${path.replace(/^\/+/, '')}`;
            replaced.push(hash);
            fakeWindow.location.hash = hash;
          },
        },
      },
      'lib/stores/browser.svelte': { browser: browserStub },
      'lib/stores/browserLive.svelte': { browserLive: browserLiveStub },
      'lib/toastError': { toastError: (title: string) => { errors.push(title); } },
    },
    { window: fakeWindow },
    { relative: true },
  );

  let urlInput = '';
  let emulation: Record<string, unknown> | null = null;
  let attachedSim: { id: string; split: boolean } | null = null;
  let missingCount = 0;

  const callbacks = {
    setUrlInput: (u: string) => { urlInput = u; },
    setEmulation: (e: Record<string, unknown>) => { emulation = e; },
    onSimulator: (d: { id: string }, split: boolean) => { attachedSim = { id: d.id, split }; },
    onSimulatorMissing: () => { missingCount += 1; },
  };

  // Case 1: preset + mode=live + URL
  await mod.consumeBrowserDeepLink(callbacks);
  assert.deepEqual(replaced, ['#/browser']);
  assert.deepEqual(prefCalls, ['remote'], 'native renderer switches to remote for preset emulation');
  assert.equal(urlInput, 'https://example.com');
  assert.deepEqual(liveUrls, ['https://example.com']);
  assert.equal(emulation?.['presetId'], 'iphone-16-pro');
  assert.equal(emulation?.['width'], 393);
  assert.equal(emulation?.['height'], 852);
  assert.equal(emulation?.['hasTouch'], true);

  // Case 2: reader navigation (no mode=live, no preset)
  fakeWindow.location.hash = '#/browser?url=https%3A%2F%2Fdocs.example.com';
  await mod.consumeBrowserDeepLink(callbacks);
  assert.deepEqual(readerUrls, ['https://docs.example.com']);

  // Case 3: mode-only link updates active tab mode
  fakeWindow.location.hash = '#/browser?mode=live';
  await mod.consumeBrowserDeepLink(callbacks);
  assert.deepEqual(modeCalls, [['tab-1', 'live']]);

  // Case 4: booted simulator target attaches split view and opens URL
  fakeWindow.location.hash = '#/browser?url=http%3A%2F%2Flocalhost%3A3000&device=simulator%3Aios%3ABOOTED';
  await mod.consumeBrowserDeepLink(callbacks);
  assert.deepEqual(attachedSim, { id: 'ios:BOOTED', split: true });
  assert.deepEqual(openedOnSim, [{ id: 'ios:BOOTED', url: 'http://localhost:3000' }]);

  // Case 4b: Expo exp:// URL forwards to simulator without navigating a web tab
  const readerCountBeforeExp = readerUrls.length;
  const liveCountBeforeExp = liveUrls.length;
  fakeWindow.location.hash = '#/browser?url=exp%3A%2F%2F127.0.0.1%3A8081&device=simulator%3Aios%3ABOOTED';
  await mod.consumeBrowserDeepLink(callbacks);
  assert.equal(readerUrls.length, readerCountBeforeExp, 'exp:// must not open a web reader tab');
  assert.equal(liveUrls.length, liveCountBeforeExp, 'exp:// must not open a web live tab');
  assert.deepEqual(openedOnSim.at(-1), { id: 'ios:BOOTED', url: 'exp://127.0.0.1:8081' });

  // Case 5: missing simulator calls onSimulatorMissing and surfaces toastError
  simResult = new Error('not found');
  fakeWindow.location.hash = '#/browser?device=simulator%3Aios%3AMISSING';
  await mod.consumeBrowserDeepLink(callbacks);
  assert.equal(missingCount, 1);
  assert.ok(errors.length >= 1);
});

// ── 6. Shared simInstaller store ──────────────────────────────────────────

test('simInstaller: gates iOS/Android CTAs, polls until completion, fires onComplete, and handles cancel/dismiss', async () => {
  const poller: { fn: (() => Promise<void>) | null; stopped: number } = { fn: null, stopped: 0 };
  let currentStatus = {
    running: true,
    target: 'ios_runtime',
    stage: 'downloading',
    message: 'Downloading iOS runtime',
    progress_pct: 40,
    log: [],
    updated_at: '2026-11-19T19:00:00Z',
  };
  const startCalls: unknown[] = [];
  const toastCalls: string[] = [];
  const toastErrors: string[] = [];

  const mod = loadSource(
    new URL('../src/lib/simulators/simInstaller.svelte.ts', import.meta.url),
    {
      'lib/api/simulators': {
        simulatorsApi: {
          installStatus: async () => currentStatus,
          startInstall: async (req: unknown) => {
            startCalls.push(req);
            return currentStatus;
          },
          cancelInstall: async () => ({
            ...currentStatus,
            running: false,
            stage: 'cancelled',
            message: 'Canceled by user',
            updated_at: '2026-11-19T19:01:00Z',
          }),
        },
      },
      '../poll': {
        pollWhileVisible: (fn: () => Promise<void>) => {
          poller.fn = fn;
          return {
            stop: () => { poller.stopped += 1; },
          };
        },
      },
      'lib/toast.svelte': {
        toasts: {
          success: (t: string) => toastCalls.push(`ok:${t}`),
          info: (t: string) => toastCalls.push(`info:${t}`),
          error: (t: string) => toastCalls.push(`err:${t}`),
        },
      },
      'lib/toastError': {
        toastError: (t: string) => toastErrors.push(t),
      },
    },
    {},
    { relative: true },
  );

  // Readiness gating helpers
  assert.equal(mod.canOfferIosRuntimeDownload(null), false);
  assert.equal(mod.canOfferIosRuntimeDownload({ platform: 'ios', state: 'ready', code: 'ready' }), false);
  assert.equal(mod.canOfferIosRuntimeDownload({ platform: 'ios', state: 'unavailable', code: 'xcode_not_installed' }), false);
  assert.equal(mod.canOfferIosRuntimeDownload({ platform: 'ios', state: 'unavailable', code: 'xcode_downloading' }), false);
  assert.equal(mod.canOfferIosRuntimeDownload({ platform: 'ios', state: 'unavailable', code: 'unsupported_os' }), false);
  assert.equal(mod.canOfferIosRuntimeDownload({ platform: 'ios', state: 'degraded', code: 'no_ios_runtime' }), true);
  assert.equal(mod.canOfferAndroidInstall({ platform: 'android', state: 'ready', code: 'ready' }), false);
  assert.equal(mod.canOfferAndroidInstall({ platform: 'android', state: 'unavailable', code: 'sdk_missing' }), true);

  const completed: string[] = [];
  const unsub = mod.simInstaller.onComplete((st: { stage: string }) => completed.push(st.stage));

  // Start installation -> begins polling
  await mod.simInstaller.start('android_emulator');
  assert.deepEqual(JSON.parse(JSON.stringify(startCalls)), [
    { target: 'android_emulator', create_default_avds: true },
  ]);
  assert.equal(mod.simInstaller.running, true);
  assert.equal(mod.simInstaller.visible, true);
  assert.ok(poller.fn, 'pollWhileVisible should be started while running');

  // Next poll reports completion -> stops poller, fires toast and subscriber
  currentStatus = {
    ...currentStatus,
    running: false,
    stage: 'done',
    message: 'Installed',
    progress_pct: 100,
    updated_at: '2026-11-19T19:02:00Z',
  };
  await poller.fn();
  assert.equal(mod.simInstaller.running, false);
  assert.equal(poller.stopped, 1);
  assert.deepEqual(completed, ['done']);
  assert.ok(toastCalls.some((c) => c.startsWith('ok:')));

  // Dismiss hides card for current updated_at
  mod.simInstaller.dismiss();
  assert.equal(mod.simInstaller.visible, false);

  // Cancel path works cleanly and unsubscribed listener is not called again
  unsub();
  await mod.simInstaller.cancel();
  assert.equal(mod.simInstaller.status?.stage, 'cancelled');
  assert.equal(completed.length, 1);
  assert.equal(toastErrors.length, 0);
});

// ── 7. IDE Devices state store ────────────────────────────────────────────

/** Load a `.svelte.ts` class store where `$derived` / `$derived.by` class fields
 *  are turned into live getters (matching Svelte 5's compiled getter semantics). */
function loadReactiveClassStore(
  path: URL,
  imports: Record<string, unknown>,
  globals: Record<string, unknown> = {},
): Record<string, any> {
  let source = readFileSync(path, 'utf8');
  const ast = ts.createSourceFile(path.pathname, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const edits: Array<{ start: number; end: number; text: string }> = [];
  function visit(node: ts.Node): void {
    if (ts.isPropertyDeclaration(node) && node.initializer && ts.isCallExpression(node.initializer)) {
      const expr = node.initializer.expression.getText(ast);
      if (expr === '$derived' && node.initializer.arguments.length === 1) {
        edits.push({
          start: node.getStart(ast),
          end: node.end,
          text: `get ${node.name.getText(ast)}() { return (${node.initializer.arguments[0].getText(ast)}); }`,
        });
      } else if (expr === '$derived.by' && node.initializer.arguments.length === 1) {
        edits.push({
          start: node.getStart(ast),
          end: node.end,
          text: `get ${node.name.getText(ast)}() { return (${node.initializer.arguments[0].getText(ast)})(); }`,
        });
      }
    }
    ts.forEachChild(node, visit);
  }
  visit(ast);
  for (const e of edits.sort((a, b) => b.start - a.start)) {
    source = source.slice(0, e.start) + e.text + source.slice(e.end);
  }
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const exports: Record<string, any> = {};
  const state = Object.assign((v: unknown) => v, { raw: (v: unknown) => v, snapshot: (v: unknown) => v });
  runInNewContext(compiled, {
    exports,
    require: (name: string) => {
      if (name in imports) return imports[name];
      if (name.endsWith('loadError')) {
        return loadSource(new URL('../src/lib/loadError.ts', import.meta.url), {}, globals);
      }
      if (name === './ideDevServerDetect') {
        return loadSource(new URL('../src/modules/ide/devices/ideDevServerDetect.ts', import.meta.url), {}, globals, { relative: true });
      }
      throw new Error(`Missing reactive store import: ${name}`);
    },
    $state: state,
    $derived: Object.assign((v: unknown) => v, { by: (fn: () => unknown) => fn() }),
    $effect: () => {},
    console,
    setTimeout,
    clearTimeout,
    URL,
    URLSearchParams,
    ...globals,
  }, { timeout: 5000, filename: path.pathname });
  return exports;
}

test('ideDevices: refreshes, sorts, tracks run target, and clears busy state on boot/port errors', async () => {
  const sessionMap = new Map<string, string>();
  const runTargets: Array<Record<string, unknown>> = [];
  const toastErrors: string[] = [];
  const d1 = makeDevice({ id: 'android:avd:Pixel_9', platform: 'android', name: 'Pixel 9', state: 'shutdown', capabilities: { ...makeDevice().capabilities, port_forward: true } });
  const d2 = makeDevice({ id: 'ios:SIM-2', platform: 'ios', name: 'iPhone 16 Pro', state: 'booted' });
  const d3 = makeDevice({ id: 'ios:SIM-NEW', platform: 'ios', name: 'iPhone 16 Mini', state: 'shutdown' });
  let bootShouldFail = false;
  let ports: Array<{ remote_port: number; local_port: number }> = [];
  const firstRefresh = deferred<{ devices: unknown[]; status: null }>();
  const forcedRefresh = deferred<{ devices: unknown[]; status: null }>();
  let refreshCall = 0;

  const mod = loadReactiveClassStore(
    new URL('../src/modules/ide/devices/ideDevicesState.svelte.ts', import.meta.url),
    {
      '../../../lib/api/simulators': {
        simulatorsApi: {
          devices: async () => {
            refreshCall += 1;
            if (refreshCall === 1) return firstRefresh.promise;
            if (refreshCall === 2) return forcedRefresh.promise;
            return { devices: [d1, d2], status: null };
          },
          boot: async (id: string) => {
            if (bootShouldFail) throw new Error('simctl failed');
            return { ...d1, id, state: 'booted' };
          },
          shutdown: async (id: string) => ({ ...d2, id, state: 'shutdown' }),
          restart: async (id: string) => ({ ...d2, id, state: 'booted' }),
          openUrl: async (_id: string, req: { url: string }) => ({
            effective_url: req.url,
            forwarded_ports: [5173],
          }),
          listPorts: async () => ({ ports }),
          addPortForward: async (_id: string, req: { remote_port: number; local_port: number }) => {
            ports = [...ports, req];
          },
          removePortForward: async (_id: string, remotePort: number) => {
            ports = ports.filter((p) => p.remote_port !== remotePort);
          },
        },
      },
      '../../../lib/toast.svelte': {
        toasts: { success: () => {}, info: () => {}, error: () => {} },
      },
      '../../../lib/toastError': {
        toastError: (t: string) => { toastErrors.push(t); },
      },
      './ideDeviceRunTarget': {
        setIdeMobileRunTarget: (t: Record<string, unknown>) => { runTargets.push(JSON.parse(JSON.stringify(t))); },
      },
    },
    {
      sessionStorage: {
        getItem: (k: string) => sessionMap.get(k) ?? null,
        setItem: (k: string, v: string) => sessionMap.set(k, v),
        removeItem: (k: string) => sessionMap.delete(k),
      },
    },
  );

  const store = mod.ideDevices;

  // Non-forced refresh joins inflight; forced refresh supersedes stale response (#refreshSeq / CONC-5).
  const p1 = store.refresh(false);
  const p2 = store.refresh(false);
  assert.equal(refreshCall, 1);
  const pForced = store.refresh(true);
  assert.equal(refreshCall, 2);

  // Newer forced refresh finishes first with [d1, d2].
  forcedRefresh.resolve({ devices: [d1, d2], status: null });
  await pForced;
  assert.equal(store.loadedOnce, true);
  assert.equal(store.devices.length, 2);
  assert.equal(store.sortedDevices[0].id, 'ios:SIM-2', 'booted device sorts first');
  assert.equal(store.selectedDeviceId, 'ios:SIM-2', 'auto-selects booted device');
  assert.deepEqual(runTargets.at(-1), {
    device_id: 'SIM-2',
    device_platform: 'ios',
  });

  // Stale first refresh arrives late with only [d1] — must be ignored!
  firstRefresh.resolve({ devices: [d1], status: null });
  await Promise.all([p1, p2]);
  assert.equal(store.devices.length, 2, 'stale refresh must not clobber newer device list');
  assert.equal(store.loading, false);

  // Selecting a shutdown device clears the synced run target.
  store.selectDevice('android:avd:Pixel_9');
  assert.equal(sessionMap.get('otto.ide.selectedSimDeviceId'), 'android:avd:Pixel_9');
  assert.deepEqual(runTargets.at(-1), {});

  // Registering a newly created device adds and selects it.
  store.registerCreatedDevice(d3);
  assert.equal(store.selectedDeviceId, 'ios:SIM-NEW');
  assert.equal(store.devices.length, 3);

  // Boot failure clears busyById and records toastError.
  bootShouldFail = true;
  assert.equal(await store.boot(d1), false);
  assert.equal(store.busyById[d1.id], undefined);
  assert.equal(toastErrors.length, 1);

  // Boot success patches state to booted, selects device, and updates run target.
  bootShouldFail = false;
  assert.equal(await store.boot(d1), true);
  assert.equal(store.devices.find((d: { id: string }) => d.id === d1.id)?.state, 'booted');
  assert.equal(store.busyById[d1.id], undefined);
  assert.deepEqual(runTargets.at(-1), {
    device_id: 'Pixel_9',
    device_platform: 'android',
  });

  // Open URL trims blank input and reloads ports on forwarded_ports.
  assert.equal(await store.openUrl(d1, '   '), false);
  ports = [{ remote_port: 5173, local_port: 5173 }];
  assert.equal(await store.openUrl(d1, 'http://localhost:5173'), true);
  await Promise.resolve();
  assert.equal(store.portsByDevice[d1.id]?.length, 1);

  // Port forward add/remove updates portsByDevice.
  assert.equal(await store.addPortForward(d1.id, 8081, 8081), true);
  assert.equal(store.portsByDevice[d1.id]?.length, 2);
  await store.removePortForward(d1.id, 8081);
  assert.equal(store.portsByDevice[d1.id]?.length, 1);
});
