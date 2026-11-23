// Pure helpers for detecting local dev-server URLs from IDE Run output
// and building IDE -> Otto Browser mobile preview deep links.

import type { SimDevice, SimFormFactor, SimPlatform } from '../../../lib/api/types';
import {
  buildBrowserDeepLink,
  type BrowserDevicePresetId,
} from '../../../lib/browserDeepLink.ts';

export type { BrowserDevicePresetId };

export interface DetectedDevServer {
  url: string;
  port: number;
  scheme: 'http' | 'https' | 'exp';
}

// eslint-disable-next-line no-control-regex
const ANSI_RE = /\x1B\[[0-9;?]*[ -/]*[@-~]/g;
const DEV_URL_RE =
  /\b(https?|exp):\/\/(localhost|127\.0\.0\.1|\[::1\]|0\.0\.0\.0):(\d{2,5})(\/[^\s'"`<>)]*)?/gi;
/** Bound per-update regex scan on large rolling IDE run logs. */
const MAX_SCAN_LINES = 200;

/** Scan recent run output lines for local dev-server URLs (Vite, Next, Expo, Flutter web, etc.). */
export function detectDevServerUrls(lines: readonly string[]): DetectedDevServer[] {
  const seen = new Map<string, DetectedDevServer>();
  const floor = Math.max(0, lines.length - MAX_SCAN_LINES);
  // Scan newest lines first so the latest restarted server port appears first.
  for (let i = lines.length - 1; i >= floor; i--) {
    const clean = lines[i].replace(ANSI_RE, '');
    DEV_URL_RE.lastIndex = 0;
    let m: RegExpExecArray | null;
    while ((m = DEV_URL_RE.exec(clean)) !== null) {
      const scheme = m[1].toLowerCase() as 'http' | 'https' | 'exp';
      const rawHost = m[2].toLowerCase();
      const port = Number(m[3]);
      if (!Number.isInteger(port) || port < 80 || port > 65535) continue;
      const host = rawHost === '0.0.0.0' || rawHost === '[::1]' ? '127.0.0.1' : rawHost;
      const path = (m[4] ?? '').replace(/[.,;:!?]+$/, '');
      const normalized = `${scheme}://${host}:${port}${path}`;
      const key = `${scheme}:${port}${path}`;
      if (!seen.has(key)) {
        seen.set(key, { url: normalized, port, scheme });
      }
      if (seen.size >= 6) return [...seen.values()];
    }
  }
  return [...seen.values()];
}

/** Pick the best Browser live-emulation preset for a simulator/emulator device. */
export function browserPresetForDevice(
  device: Pick<SimDevice, 'platform' | 'form_factor'> | null | undefined,
): BrowserDevicePresetId {
  if (!device) return 'iphone-16-pro';
  const plat: SimPlatform = device.platform;
  const form: SimFormFactor = device.form_factor;
  if (plat === 'ios') {
    return form === 'tablet' ? 'ipad-pro-11' : 'iphone-16-pro';
  }
  return form === 'tablet' ? 'android-tablet' : 'pixel-9';
}

/**
 * Build the locked IDE -> Otto Browser deep-link hash:
 * `#/browser?url=<encoded>&mode=live&device=<preset_id|simulator:<device_id>>`.
 */
export function buildBrowserPreviewHash(opts: {
  url: string;
  device?: BrowserDevicePresetId | `simulator:${string}`;
}): string {
  const device = opts.device;
  if (device?.startsWith('simulator:')) {
    return buildBrowserDeepLink({
      url: opts.url,
      mode: 'live',
      simulatorId: device.slice('simulator:'.length),
    });
  }
  return buildBrowserDeepLink({
    url: opts.url,
    mode: 'live',
    presetId: device as BrowserDevicePresetId | undefined,
  });
}

/** Build a `browser?...` router path for a local HTTP(S) dev-server URL. */
export function buildDevBrowserRoute(rawUrl: string): string | null {
  const trimmed = rawUrl.trim().replace(/[.,;:!?]+$/, '');
  try {
    const u = new URL(trimmed);
    if (u.protocol !== 'http:' && u.protocol !== 'https:') return null;
    if (!u.port) return null;
    const port = Number(u.port);
    if (!Number.isInteger(port) || port < 80 || port > 65535) return null;
    if (u.hostname === '0.0.0.0' || u.hostname === '[::1]') u.hostname = '127.0.0.1';
    return buildBrowserDeepLink({ url: u.href, mode: 'live' }).replace(/^#\//, '');
  } catch {
    return null;
  }
}

/** Resolve the native CLI device identifier (`ANDROID_SERIAL` or iOS UDID) for a SimDevice. */
export function nativeDeviceRunId(device: SimDevice): string {
  if (device.platform === 'ios') {
    return device.udid?.trim() || device.id.replace(/^ios:/, '');
  }
  return (
    device.serial?.trim() ||
    device.avd_name?.trim() ||
    device.id.replace(/^android:(?:serial|avd):/, '')
  );
}
