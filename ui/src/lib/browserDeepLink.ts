// Shared IDE <-> Browser preview deep-link contract & device preset IDs.
// Format: `#/browser?url=<encoded>&mode=live&device=<preset_id|simulator:<id>>`
// Pure, zero runtime imports — unit-testable directly in plain Node.

export type BrowserDevicePresetId =
  | 'responsive'
  | 'iphone-16-pro'
  | 'iphone-se'
  | 'ipad-pro-11'
  | 'pixel-9'
  | 'android-tablet'
  | 'custom';

export const BROWSER_DEVICE_PRESET_IDS: readonly BrowserDevicePresetId[] = [
  'responsive',
  'iphone-16-pro',
  'iphone-se',
  'ipad-pro-11',
  'pixel-9',
  'android-tablet',
  'custom',
];

const PRESET_SET = new Set<string>(BROWSER_DEVICE_PRESET_IDS);

export function isBrowserDevicePresetId(v: string | null | undefined): v is BrowserDevicePresetId {
  return !!v && PRESET_SET.has(v);
}

export interface BrowserDeepLink {
  url?: string;
  mode?: 'reader' | 'live';
  presetId?: BrowserDevicePresetId;
  simulatorId?: string;
}

export function isHttpUrl(url: string): boolean {
  return /^https?:\/\//i.test(url);
}

export function isSimulatorDeepLinkUrl(url: string): boolean {
  return /^(?:https?|exp):\/\//i.test(url);
}

/** Build a `#/browser?...` deep-link hash for IDE / Run Panel / Devices handoffs. */
export function buildBrowserDeepLink(opts: BrowserDeepLink): string {
  const params = new URLSearchParams();
  if (opts.url) params.set('url', opts.url);
  if (opts.mode) params.set('mode', opts.mode);
  if (opts.simulatorId) params.set('device', `simulator:${opts.simulatorId}`);
  else if (opts.presetId) params.set('device', opts.presetId);
  const qs = params.toString();
  return qs ? `#/browser?${qs}` : '#/browser';
}

/** Parse a `#/browser?...` hash into typed deep-link instructions.
 *  - Web URLs must be `http://` or `https://`.
 *  - When targeting a simulator (`device=simulator:<id>`), `exp://` URLs are
 *    also preserved so Expo dev servers open directly on the simulator while
 *    non-http(s) URLs are skipped for web tabs. */
export function parseBrowserDeepLink(hash: string): BrowserDeepLink | null {
  const cleaned = hash.replace(/^#/, '');
  const qIdx = cleaned.indexOf('?');
  if (qIdx < 0) return null;
  const path = cleaned.slice(0, qIdx).replace(/^\/+/, '');
  if (path !== 'browser') return null;
  const params = new URLSearchParams(cleaned.slice(qIdx + 1));
  const out: BrowserDeepLink = {};
  let any = false;

  const device = params.get('device')?.trim();
  if (device) {
    if (device.startsWith('simulator:')) {
      const simId = device.slice('simulator:'.length).trim();
      if (simId) {
        out.simulatorId = simId;
        any = true;
      }
    } else if (isBrowserDevicePresetId(device)) {
      out.presetId = device;
      any = true;
    }
  }

  const rawUrl = params.get('url')?.trim();
  if (rawUrl) {
    const allowed = out.simulatorId ? isSimulatorDeepLinkUrl(rawUrl) : isHttpUrl(rawUrl);
    if (allowed) {
      out.url = rawUrl;
      any = true;
    }
  }

  const mode = params.get('mode')?.trim().toLowerCase();
  if (mode === 'live' || mode === 'reader') {
    out.mode = mode;
    any = true;
  }

  return any ? out : null;
}
