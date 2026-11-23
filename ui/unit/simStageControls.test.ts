import test from 'node:test';
import assert from 'node:assert/strict';
import type { SimDevice } from '../src/lib/api/types.ts';
import {
  buildSimMoreMenuItems,
  mapStageKeyToInput,
} from '../src/lib/simulators/simStageControls.ts';

function makeDevice(platform: 'ios' | 'android', overrides: Partial<SimDevice> = {}): SimDevice {
  return {
    id: platform === 'android' ? 'android:avd:Pixel_Phone' : 'ios:AAA-111',
    platform,
    name: platform === 'android' ? 'Pixel Phone' : 'iPhone 18 Pro',
    state: 'booted',
    os_version: platform === 'android' ? '15' : '27.2',
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
      tap: true,
      swipe: true,
      text: true,
      key_event: true,
      rotate: true,
      appearance: true,
      status_bar: platform === 'ios',
      clipboard: platform === 'ios',
      port_forward: platform === 'android',
      app_install: true,
      logs: true,
    },
    ...overrides,
  };
}

const key = (k: string, mods: { metaKey?: boolean; ctrlKey?: boolean } = {}) => ({
  key: k,
  metaKey: false,
  ctrlKey: false,
  ...mods,
});

test('mapStageKeyToInput maps Android keys and printable text, ignoring Tab and Cmd/Ctrl chords', () => {
  const dev = makeDevice('android');
  assert.deepEqual(mapStageKeyToInput(dev, key('Enter')), { type: 'key_event', key: 'ENTER' });
  assert.deepEqual(mapStageKeyToInput(dev, key('Backspace')), { type: 'key_event', key: 'DEL' });
  assert.deepEqual(mapStageKeyToInput(dev, key('Escape')), { type: 'key_event', key: 'BACK' });
  assert.deepEqual(mapStageKeyToInput(dev, key('a')), { type: 'text', text: 'a' });
  // Tab and Shift+Tab (both report key 'Tab') stay with the browser so focus can leave the stage.
  assert.equal(mapStageKeyToInput(dev, key('Tab')), null);
  assert.equal(mapStageKeyToInput(dev, key('c', { metaKey: true })), null);
  assert.equal(mapStageKeyToInput(dev, key('a', { ctrlKey: true })), null);
  // Option-typed characters (e.g. "@" on a German layout) are text, not a chord.
  const optionAt = { ...key('@'), altKey: true };
  assert.deepEqual(mapStageKeyToInput(dev, optionAt), { type: 'text', text: '@' });
});

test('mapStageKeyToInput follows capabilities and has no key map on iOS', () => {
  const caps = makeDevice('android').capabilities;
  const noKeys = makeDevice('android', { capabilities: { ...caps, key_event: false } });
  assert.equal(mapStageKeyToInput(noKeys, key('Enter')), null);
  assert.deepEqual(mapStageKeyToInput(noKeys, key('a')), { type: 'text', text: 'a' });
  const noText = makeDevice('android', { capabilities: { ...caps, text: false } });
  assert.equal(mapStageKeyToInput(noText, key('a')), null);
  assert.deepEqual(mapStageKeyToInput(noText, key('Enter')), { type: 'key_event', key: 'ENTER' });

  const ios = makeDevice('ios');
  assert.equal(mapStageKeyToInput(ios, key('Enter')), null);
  assert.equal(mapStageKeyToInput(ios, key('Escape')), null);
  assert.deepEqual(mapStageKeyToInput(ios, key('a')), { type: 'text', text: 'a' });
});

const noop = () => {};
function menu(over: Partial<Parameters<typeof buildSimMoreMenuItems>[0]> = {}) {
  return buildSimMoreMenuItems({
    dev: makeDevice('android'),
    booted: true,
    busy: false,
    fps: 5,
    split: true,
    showTextBar: false,
    showLogs: false,
    hasSplitToggle: true,
    onToggleTextBar: noop,
    onScreenshot: noop,
    onToggleLogs: noop,
    onToggleSplit: noop,
    onSetFps: noop,
    onRestart: noop,
    onPopOutNative: noop,
    onShutdown: noop,
    ...over,
  });
}
const labels = (items: ReturnType<typeof menu>) => items.filter((i) => !i.separator).map((i) => i.label);

test('buildSimMoreMenuItems: booted Android lists every action and names the native-window restart', () => {
  let popOutCalls = 0;
  let pickedFps = 0;
  const items = menu({ onSetFps: (f) => (pickedFps = f), onPopOutNative: () => popOutCalls++ });
  assert.deepEqual(labels(items), [
    'Type text…',
    'Save screenshot (PNG)',
    'Show device logs',
    'Expand to full view',
    '3 fps',
    '5 fps',
    '10 fps',
    'Restart embedded',
    'Restart in native window',
    'Shut down simulator',
  ]);
  assert.equal(items.find((i) => i.label === '5 fps')?.checked, true);
  items.find((i) => i.label === '10 fps')?.action?.();
  assert.equal(pickedFps, 10);
  items.find((i) => i.label === 'Restart in native window')?.action?.();
  assert.equal(popOutCalls, 1);
});

test('buildSimMoreMenuItems: iOS pop-out label, busy disables lifecycle items, not booted shows only split', () => {
  assert.deepEqual(labels(menu({ dev: makeDevice('ios') })).slice(-3), [
    'Restart embedded',
    'Pop out in Simulator.app',
    'Shut down simulator',
  ]);
  const busy = menu({ busy: true }).filter((i) => i.disabled).map((i) => i.label);
  assert.deepEqual(busy, ['Restart embedded', 'Restart in native window', 'Shut down simulator']);
  assert.deepEqual(labels(menu({ booted: false, split: false })), ['Split view with browser']);
});
