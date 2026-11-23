import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  browserPresetForDevice,
  buildBrowserPreviewHash,
  buildDevBrowserRoute,
  detectDevServerUrls,
  nativeDeviceRunId,
} from '../src/modules/ide/devices/ideDevServerDetect.ts';
import { parseBrowserDeepLink } from '../src/lib/browserDeepLink.ts';
import type { SimDevice } from '../src/lib/api/types/simulators.ts';

function makeDevice(overrides: Partial<SimDevice>): SimDevice {
  return {
    id: 'ios:ABC-123',
    platform: 'ios',
    name: 'iPhone 16 Pro',
    udid: 'ABC-123',
    state: 'booted',
    os_version: '18.2',
    form_factor: 'phone',
    orientation: 'portrait',
    is_physical: false,
    capabilities: {
      boot: true,
      shutdown: true,
      delete: true,
      open_url: true,
      screenshot: true,
      stream: true,
      tap: false,
      swipe: false,
      text: false,
      key_event: false,
      rotate: true,
      appearance: true,
      status_bar: true,
      clipboard: true,
      port_forward: false,
      app_install: true,
      logs: true,
    },
    ...overrides,
  };
}

describe('ideDevServerDetect', () => {
  it('extracts and deduplicates local dev server URLs with ANSI stripping', () => {
    const lines = [
      '  \x1b[32m➜\x1b[0m  Local:   \x1b[36mhttp://localhost:5173/\x1b[0m',
      '  Expo Metro waiting on exp://127.0.0.1:8081',
      '  Server ready at http://0.0.0.0:3000/api.',
      '  Duplicate http://localhost:5173/',
    ];
    const found = detectDevServerUrls(lines);
    assert.deepEqual(
      found.map((s) => s.url),
      ['http://localhost:5173/', 'http://127.0.0.1:3000/api', 'exp://127.0.0.1:8081'],
    );
    assert.deepEqual(
      found.map((s) => s.port),
      [5173, 3000, 8081],
    );
  });

  it('caps scanning to the most recent 200 output lines', () => {
    const lines: string[] = ['http://127.0.0.1:4000/stale'];
    for (let i = 0; i < 220; i++) {
      lines.push(`build log line ${i}`);
    }
    lines.push('ready at http://127.0.0.1:5173/');
    const found = detectDevServerUrls(lines);
    assert.deepEqual(
      found.map((s) => s.url),
      ['http://127.0.0.1:5173/'],
    );
  });

  it('builds normalized browser routes for HTTP(S) dev servers and rejects non-HTTP schemes', () => {
    assert.equal(
      buildDevBrowserRoute('http://0.0.0.0:8080/api'),
      'browser?url=http%3A%2F%2F127.0.0.1%3A8080%2Fapi&mode=live',
    );
    assert.equal(buildDevBrowserRoute('exp://127.0.0.1:8081'), null);
    assert.equal(buildDevBrowserRoute('javascript:alert(1)'), null);
  });

  it('builds locked IDE -> Browser live emulation deep links that round-trip with parseBrowserDeepLink', () => {
    const presetHash = buildBrowserPreviewHash({ url: 'http://127.0.0.1:5173', device: 'iphone-16-pro' });
    assert.equal(
      presetHash,
      '#/browser?url=http%3A%2F%2F127.0.0.1%3A5173&mode=live&device=iphone-16-pro',
    );
    assert.deepEqual(parseBrowserDeepLink(presetHash), {
      url: 'http://127.0.0.1:5173',
      mode: 'live',
      presetId: 'iphone-16-pro',
    });

    const simHash = buildBrowserPreviewHash({ url: 'exp://127.0.0.1:8081', device: 'simulator:ios:ABC-123' });
    assert.deepEqual(parseBrowserDeepLink(simHash), {
      url: 'exp://127.0.0.1:8081',
      mode: 'live',
      simulatorId: 'ios:ABC-123',
    });
  });

  it('maps simulator platform and form factor to Browser presets and native CLI IDs', () => {
    assert.equal(browserPresetForDevice({ platform: 'ios', form_factor: 'phone' }), 'iphone-16-pro');
    assert.equal(browserPresetForDevice({ platform: 'ios', form_factor: 'tablet' }), 'ipad-pro-11');
    assert.equal(browserPresetForDevice({ platform: 'android', form_factor: 'phone' }), 'pixel-9');
    assert.equal(browserPresetForDevice({ platform: 'android', form_factor: 'tablet' }), 'android-tablet');

    assert.equal(nativeDeviceRunId(makeDevice({ id: 'ios:UDID-1', udid: 'UDID-1' })), 'UDID-1');
    assert.equal(
      nativeDeviceRunId(
        makeDevice({
          id: 'android:serial:emulator-5554',
          platform: 'android',
          udid: null,
          serial: 'emulator-5554',
        }),
      ),
      'emulator-5554',
    );
  });
});
