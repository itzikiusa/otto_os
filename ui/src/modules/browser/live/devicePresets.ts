// Device emulation presets for Otto's Live Browser.
// Pure helpers — unit-tested directly under `ui/unit/`.

import type {
  BrowserColorScheme,
  BrowserLiveEmulation,
  BrowserOrientation,
  BrowserViewport,
} from '../../../lib/api/types';
import {
  buildBrowserDeepLink,
  parseBrowserDeepLink,
  type BrowserDeepLink,
  type BrowserDevicePresetId,
} from '../../../lib/browserDeepLink.ts';

export type DevicePresetId = BrowserDevicePresetId;
export { buildBrowserDeepLink, parseBrowserDeepLink, type BrowserDeepLink };

export interface DevicePreset {
  id: DevicePresetId;
  label: string;
  shortLabel: string;
  platform: 'desktop' | 'ios' | 'android';
  formFactor: 'desktop' | 'phone' | 'tablet';
  /** Portrait CSS px (width <= height for mobile presets; 0 for responsive). */
  width: number;
  height: number;
  dpr: number;
  mobile: boolean;
  hasTouch: boolean;
  userAgent: string;
}

const UA_IPHONE =
  'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1';
const UA_IPAD =
  'Mozilla/5.0 (iPad; CPU OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1';
const UA_ANDROID_PHONE =
  'Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Mobile Safari/537.36';
const UA_ANDROID_TABLET =
  'Mozilla/5.0 (Linux; Android 15; Pixel Tablet) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36';

export const DEVICE_PRESETS: readonly DevicePreset[] = [
  {
    id: 'responsive',
    label: 'Responsive (Fit panel)',
    shortLabel: 'Responsive',
    platform: 'desktop',
    formFactor: 'desktop',
    width: 0,
    height: 0,
    dpr: 1,
    mobile: false,
    hasTouch: false,
    userAgent: '',
  },
  {
    id: 'iphone-16-pro',
    label: 'iPhone 16 Pro',
    shortLabel: 'iPhone 16 Pro',
    platform: 'ios',
    formFactor: 'phone',
    width: 393,
    height: 852,
    dpr: 3,
    mobile: true,
    hasTouch: true,
    userAgent: UA_IPHONE,
  },
  {
    id: 'iphone-se',
    label: 'iPhone SE',
    shortLabel: 'iPhone SE',
    platform: 'ios',
    formFactor: 'phone',
    width: 375,
    height: 667,
    dpr: 2,
    mobile: true,
    hasTouch: true,
    userAgent: UA_IPHONE,
  },
  {
    id: 'ipad-pro-11',
    label: 'iPad Pro 11″',
    shortLabel: 'iPad Pro 11″',
    platform: 'ios',
    formFactor: 'tablet',
    width: 834,
    height: 1194,
    dpr: 2,
    mobile: true,
    hasTouch: true,
    userAgent: UA_IPAD,
  },
  {
    id: 'pixel-9',
    label: 'Pixel 9',
    shortLabel: 'Pixel 9',
    platform: 'android',
    formFactor: 'phone',
    width: 412,
    height: 915,
    dpr: 2.75,
    mobile: true,
    hasTouch: true,
    userAgent: UA_ANDROID_PHONE,
  },
  {
    id: 'android-tablet',
    label: 'Android Tablet',
    shortLabel: 'Android Tablet',
    platform: 'android',
    formFactor: 'tablet',
    width: 800,
    height: 1280,
    dpr: 2,
    mobile: true,
    hasTouch: true,
    userAgent: UA_ANDROID_TABLET,
  },
  {
    id: 'custom',
    label: 'Custom size',
    shortLabel: 'Custom',
    platform: 'desktop',
    formFactor: 'phone',
    width: 430,
    height: 932,
    dpr: 2,
    mobile: true,
    hasTouch: true,
    userAgent: '',
  },
];

export interface DeviceEmulationState {
  enabled: boolean;
  presetId: DevicePresetId;
  /** Effective CSS viewport width (after orientation). */
  width: number;
  /** Effective CSS viewport height (after orientation). */
  height: number;
  dpr: number;
  orientation: BrowserOrientation;
  mobile: boolean;
  hasTouch: boolean;
  colorScheme: 'system' | BrowserColorScheme;
  userAgent: string;
}

export const DEFAULT_EMULATION_STATE: DeviceEmulationState = {
  enabled: false,
  presetId: 'responsive',
  width: 0,
  height: 0,
  dpr: 1,
  orientation: 'portrait',
  mobile: false,
  hasTouch: false,
  colorScheme: 'system',
  userAgent: '',
};

export function presetById(id: string | null | undefined): DevicePreset | null {
  if (!id) return null;
  return DEVICE_PRESETS.find((p) => p.id === id) ?? null;
}

export function clampDim(v: number, lo: number, hi: number): number {
  if (!Number.isFinite(v)) return lo;
  return Math.round(Math.min(hi, Math.max(lo, v)));
}

export function clampDpr(v: number): number {
  if (!Number.isFinite(v)) return 1;
  return Math.round(Math.min(3, Math.max(1, v)) * 4) / 4;
}

/** Build a new emulation state from a preset, preserving colorScheme if provided. */
export function stateForPreset(
  presetId: DevicePresetId,
  prev?: Partial<DeviceEmulationState>,
): DeviceEmulationState {
  const preset = presetById(presetId) ?? DEVICE_PRESETS[0];
  const colorScheme = prev?.colorScheme ?? 'system';
  if (preset.id === 'responsive') {
    return {
      enabled: prev?.enabled ?? true,
      presetId: 'responsive',
      width: 0,
      height: 0,
      dpr: 1,
      orientation: 'portrait',
      mobile: false,
      hasTouch: false,
      colorScheme,
      userAgent: '',
    };
  }
  const orientation: BrowserOrientation =
    prev?.orientation === 'landscape' && preset.id !== 'custom' ? 'landscape' : 'portrait';
  const w = orientation === 'landscape' ? Math.max(preset.width, preset.height) : Math.min(preset.width, preset.height);
  const h = orientation === 'landscape' ? Math.min(preset.width, preset.height) : Math.max(preset.width, preset.height);
  return {
    enabled: true,
    presetId: preset.id,
    width: w,
    height: h,
    dpr: clampDpr(preset.dpr),
    orientation,
    mobile: preset.mobile,
    hasTouch: preset.hasTouch,
    colorScheme,
    userAgent: preset.userAgent,
  };
}

/** Swap width/height and flip orientation. No-op dimensions when `responsive`. */
export function rotateEmulation(s: DeviceEmulationState): DeviceEmulationState {
  const nextOrientation: BrowserOrientation = s.orientation === 'portrait' ? 'landscape' : 'portrait';
  if (s.presetId === 'responsive' || s.width <= 0 || s.height <= 0) {
    return { ...s, orientation: nextOrientation };
  }
  return {
    ...s,
    width: s.height,
    height: s.width,
    orientation: nextOrientation,
  };
}

/** Fixed viewport to send to CDP when a device preset or custom size is active;
 *  returns `null` when emulation is off or in `responsive` (panel-fit) mode. */
export function emulatedViewport(s: DeviceEmulationState): Required<BrowserViewport> | null {
  if (!s.enabled || s.presetId === 'responsive') return null;
  return {
    width: clampDim(s.width, 200, 3840),
    height: clampDim(s.height, 200, 2160),
    device_scale_factor: clampDpr(s.dpr),
  };
}

/** Convert UI emulation state into the wire `BrowserLiveEmulation` shape. */
export function toWireEmulation(s: DeviceEmulationState): BrowserLiveEmulation {
  if (!s.enabled) {
    return {
      preset_id: null,
      mobile: false,
      has_touch: false,
      user_agent: null,
      color_scheme: null,
      orientation: null,
    };
  }
  const fixed = s.presetId !== 'responsive';
  return {
    preset_id: fixed ? s.presetId : null,
    mobile: fixed ? s.mobile : false,
    has_touch: s.hasTouch,
    user_agent: fixed && s.userAgent ? s.userAgent : null,
    color_scheme: s.colorScheme === 'system' ? null : s.colorScheme,
    orientation: fixed ? s.orientation : null,
  };
}
