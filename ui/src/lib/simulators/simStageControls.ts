// Pure gesture, keyboard-mapping & context-menu helpers shared by the embedded
// simulator stages (Browser SimulatorView and the IDE Devices preview).
import type { MenuItem } from '../contextmenu.svelte';
import type { SimDevice, SimInputReq } from '../api/types';

const ANDROID_KEY_MAP: Record<string, string> = {
  Enter: 'ENTER',
  Backspace: 'DEL',
  Delete: 'FORWARD_DEL',
  Escape: 'BACK',
  ArrowUp: 'DPAD_UP',
  ArrowDown: 'DPAD_DOWN',
  ArrowLeft: 'DPAD_LEFT',
  ArrowRight: 'DPAD_RIGHT',
};

/** Decide whether a pointer down→up gesture was a tap or a swipe (client px). */
export function isSwipeGesture(start: { x: number; y: number }, end: { x: number; y: number }): boolean {
  return Math.hypot(end.x - start.x, end.y - start.y) >= 10;
}

/** Map a DOM KeyboardEvent from the focused embedded stage to a SimInputReq, or null if unhandled.
 *  Tab is never mapped, so Tab / Shift+Tab always move focus out of the stage. Option is allowed:
 *  it types characters such as "@" on many non-US layouts. */
export function mapStageKeyToInput(
  dev: SimDevice,
  e: Pick<KeyboardEvent, 'key' | 'metaKey' | 'ctrlKey'>,
): SimInputReq | null {
  if (e.metaKey || e.ctrlKey) return null;
  if (dev.platform === 'android' && dev.capabilities.key_event && e.key in ANDROID_KEY_MAP) {
    return { type: 'key_event', key: ANDROID_KEY_MAP[e.key] };
  }
  if (dev.capabilities.text && e.key.length === 1) {
    return { type: 'text', text: e.key };
  }
  return null;
}

interface SimMoreMenuArgs {
  dev: SimDevice;
  booted: boolean;
  busy: boolean;
  fps: number;
  split: boolean;
  showTextBar: boolean;
  showLogs: boolean;
  hasSplitToggle: boolean;
  onToggleTextBar: () => void;
  onScreenshot: () => void;
  onToggleLogs: () => void;
  onToggleSplit: () => void;
  onSetFps: (fps: number) => void;
  onRestart: () => void;
  onPopOutNative: () => void;
  onShutdown: () => void;
}

export function buildSimMoreMenuItems(a: SimMoreMenuArgs): MenuItem[] {
  const { dev, booted, busy, fps } = a;
  const caps = dev.capabilities;
  const items: MenuItem[] = [];
  if (booted && caps.text) {
    items.push({
      label: a.showTextBar ? 'Hide text bar' : 'Type text…',
      icon: 'edit',
      action: a.onToggleTextBar,
    });
  }
  if (booted && caps.screenshot) {
    items.push({
      label: 'Save screenshot (PNG)',
      icon: 'image',
      action: a.onScreenshot,
    });
  }
  if (booted && caps.logs) {
    items.push({
      label: a.showLogs ? 'Hide device logs' : 'Show device logs',
      icon: 'terminal',
      action: a.onToggleLogs,
    });
  }
  if (a.hasSplitToggle) {
    items.push({
      label: a.split ? 'Expand to full view' : 'Split view with browser',
      icon: 'columns',
      action: a.onToggleSplit,
    });
  }
  if (booted) {
    items.push(
      { separator: true },
      { label: '3 fps', checked: fps === 3, action: () => a.onSetFps(3) },
      { label: '5 fps', checked: fps === 5, action: () => a.onSetFps(5) },
      { label: '10 fps', checked: fps === 10, action: () => a.onSetFps(10) },
      { separator: true },
    );
    if (caps.boot) {
      items.push({
        label: 'Restart embedded',
        icon: 'refresh',
        disabled: busy,
        action: a.onRestart,
      });
      items.push({
        label: dev.platform === 'ios' ? 'Pop out in Simulator.app' : 'Restart in native window',
        icon: 'external',
        disabled: busy,
        action: a.onPopOutNative,
      });
    }
    if (caps.shutdown) {
      items.push({
        label: 'Shut down simulator',
        icon: 'stop',
        disabled: busy,
        action: a.onShutdown,
      });
    }
  }
  return items;
}
