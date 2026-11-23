// Unit tests for Browser device emulation presets, IDE→Browser deep-links,
// and Simulator screen coordinate mapping.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  DEFAULT_EMULATION_STATE,
  DEVICE_PRESETS,
  buildBrowserDeepLink,
  emulatedViewport,
  parseBrowserDeepLink,
  presetById,
  rotateEmulation,
  stateForPreset,
  toWireEmulation,
} from '../src/modules/browser/live/devicePresets.ts';
import { clientToSimNormalized, simDeviceSubtitle } from '../src/modules/browser/simulators/simGeometry.ts';
import { isSwipeGesture } from '../src/lib/simulators/simStageControls.ts';
import { BINARY_FRAME_VERSION, decodeBinaryFrame } from '../src/lib/api/binaryFrame.ts';
import { isHttpUrl, isSimulatorDeepLinkUrl } from '../src/lib/browserDeepLink.ts';
import type { SimDevice } from '../src/lib/api/types';

test('all contract device presets exist and produce valid mobile/tablet viewports', () => {
  for (const id of ['responsive', 'iphone-16-pro', 'iphone-se', 'ipad-pro-11', 'pixel-9', 'android-tablet'] as const) {
    const p = presetById(id);
    assert.ok(p, `missing preset ${id}`);
  }
  assert.ok(DEVICE_PRESETS.length >= 7);

  const iphone = stateForPreset('iphone-16-pro');
  assert.equal(iphone.enabled, true);
  assert.equal(iphone.orientation, 'portrait');
  assert.deepEqual(emulatedViewport(iphone), { width: 393, height: 852, device_scale_factor: 3 });

  const wire = toWireEmulation(iphone);
  assert.equal(wire.preset_id, 'iphone-16-pro');
  assert.equal(wire.mobile, true);
  assert.equal(wire.has_touch, true);
  assert.equal(wire.orientation, 'portrait');
  assert.ok(wire.user_agent?.includes('iPhone'));
});

test('rotateEmulation swaps dimensions and updates wire orientation', () => {
  const portrait = stateForPreset('pixel-9');
  const landscape = rotateEmulation(portrait);
  assert.equal(landscape.orientation, 'landscape');
  assert.equal(landscape.width, 915);
  assert.equal(landscape.height, 412);
  assert.deepEqual(emulatedViewport(landscape), { width: 915, height: 412, device_scale_factor: 2.75 });
  assert.equal(toWireEmulation(landscape).orientation, 'landscape');
});

test('responsive mode returns null fixed viewport and clears mobile/UA overrides', () => {
  const resp = stateForPreset('responsive', { colorScheme: 'dark', enabled: true });
  assert.equal(emulatedViewport(resp), null);
  const wire = toWireEmulation(resp);
  assert.equal(wire.preset_id, null);
  assert.equal(wire.mobile, false);
  assert.equal(wire.user_agent, null);
  assert.equal(wire.color_scheme, 'dark');

  const off = toWireEmulation(DEFAULT_EMULATION_STATE);
  assert.equal(off.color_scheme, null);
  assert.equal(off.mobile, false);
});

test('parseBrowserDeepLink & buildBrowserDeepLink handle presets, simulator targets, exp://, and reject unsafe schemes', () => {
  assert.deepEqual(
    parseBrowserDeepLink('#/browser?url=http%3A%2F%2Flocalhost%3A5173&mode=live&device=iphone-16-pro'),
    { url: 'http://localhost:5173', mode: 'live', presetId: 'iphone-16-pro' },
  );
  assert.deepEqual(
    parseBrowserDeepLink('#/browser?url=https%3A%2F%2Fexample.com&device=simulator:ios:ABC-123'),
    { url: 'https://example.com', simulatorId: 'ios:ABC-123' },
  );
  // exp:// is permitted ONLY when targeting a simulator, never a bare web tab.
  const expoHash = buildBrowserDeepLink({ url: 'exp://127.0.0.1:8081', simulatorId: 'ios:ABC-123' });
  assert.deepEqual(parseBrowserDeepLink(expoHash), {
    url: 'exp://127.0.0.1:8081',
    simulatorId: 'ios:ABC-123',
  });
  assert.equal(parseBrowserDeepLink('#/browser?url=exp%3A%2F%2F127.0.0.1%3A8081'), null);
  assert.equal(isHttpUrl('https://example.com'), true);
  assert.equal(isHttpUrl('exp://127.0.0.1:8081'), false);
  assert.equal(isSimulatorDeepLinkUrl('exp://127.0.0.1:8081'), true);
  assert.equal(isSimulatorDeepLinkUrl('javascript:alert(1)'), false);

  assert.equal(parseBrowserDeepLink('#/ide?url=https://example.com'), null);
  assert.equal(parseBrowserDeepLink('#/browser?url=javascript:alert(1)'), null);
  assert.equal(parseBrowserDeepLink('#/browser'), null);
});

test('decodeBinaryFrame parses valid version-1 frames and rejects malformed envelopes', () => {
  function makeFrame(version: number, headerText: string, payload: number[]): ArrayBuffer {
    const hBytes = new TextEncoder().encode(headerText);
    const buf = new ArrayBuffer(5 + hBytes.byteLength + payload.length);
    const view = new DataView(buf);
    view.setUint8(0, version);
    view.setUint32(1, hBytes.byteLength);
    new Uint8Array(buf, 5, hBytes.byteLength).set(hBytes);
    new Uint8Array(buf, 5 + hBytes.byteLength).set(payload);
    return buf;
  }

  const ok = decodeBinaryFrame<{ seq: number; width: number }>(
    makeFrame(BINARY_FRAME_VERSION, JSON.stringify({ seq: 7, width: 393 }), [0xff, 0xd8, 0xff]),
    (h) => typeof h.seq === 'number' && h.width > 0,
  );
  assert.ok(ok);
  assert.equal(ok.header.seq, 7);
  assert.deepEqual(Array.from(ok.bytes), [0xff, 0xd8, 0xff]);

  // Wrong version
  assert.equal(decodeBinaryFrame(makeFrame(99, '{"seq":1}', [1])), null);
  // Truncated buffer
  assert.equal(decodeBinaryFrame(new ArrayBuffer(3)), null);
  // Zero-length header
  assert.equal(decodeBinaryFrame(makeFrame(BINARY_FRAME_VERSION, '', [1])), null);
  // Malformed JSON
  assert.equal(decodeBinaryFrame(makeFrame(BINARY_FRAME_VERSION, '{bad', [1])), null);
  // Validator rejects
  assert.equal(
    decodeBinaryFrame<{ seq: number }>(
      makeFrame(BINARY_FRAME_VERSION, '{"seq":0}', [1]),
      (h) => h.seq > 0,
    ),
    null,
  );
});

test('clientToSimNormalized maps clicks inside pillarboxed simulator frame and ignores bars', () => {
  // 400×800 phone screenshot centered in 800×800 container → fit is x=200..600, y=0..800.
  const box = { left: 10, top: 20, width: 800, height: 800 };
  const img = { width: 400, height: 800 };
  // Left bar → null
  assert.equal(clientToSimNormalized({ x: 50, y: 420 }, box, img), null);
  // Right bar → null
  assert.equal(clientToSimNormalized({ x: 750, y: 420 }, box, img), null);
  // Exact top-left of screen (10+200=210, 20+0=20) → (0, 0)
  assert.deepEqual(clientToSimNormalized({ x: 210, y: 20 }, box, img), { x: 0, y: 0 });
  // Center of screen (10+400=410, 20+400=420) → (0.5, 0.5)
  assert.deepEqual(clientToSimNormalized({ x: 410, y: 420 }, box, img), { x: 0.5, y: 0.5 });
});

test('isSwipeGesture distinguishes taps from drags', () => {
  assert.equal(isSwipeGesture({ x: 100, y: 100 }, { x: 104, y: 103 }), false);
  assert.equal(isSwipeGesture({ x: 100, y: 100 }, { x: 100, y: 140 }), true);
});

test('simDeviceSubtitle formats OS, form factor, screen, and USB tag', () => {
  const dev: SimDevice = {
    id: 'android:avd:Pixel_9',
    platform: 'android',
    name: 'Pixel 9',
    state: 'booted',
    os_version: 'Android 15',
    form_factor: 'phone',
    screen: { width: 1080, height: 2424 },
    orientation: 'portrait',
    is_physical: false,
    capabilities: {
      boot: true,
      shutdown: true,
      delete: true,
      open_url: true,
      screenshot: true,
      stream: true,
      tap: true,
      swipe: true,
      text: true,
      key_event: true,
      rotate: true,
      appearance: true,
      status_bar: false,
      clipboard: true,
      port_forward: true,
      app_install: true,
      logs: true,
    },
  };
  assert.equal(simDeviceSubtitle(dev), 'Android 15 · Phone · 1080×2424');
});
