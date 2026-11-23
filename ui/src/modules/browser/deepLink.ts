// Consumes Git PR preview and IDE → Browser hash deep-links:
// `#/browser?url=<encoded>&mode=live|reader&device=<preset_id|simulator:<id>>`
import { simulatorsApi } from '../../lib/api/simulators';
import { isHttpUrl, parseBrowserDeepLink } from '../../lib/browserDeepLink';
import { nativeBrowserAvailable } from '../../lib/nativeBrowser';
import { router } from '../../lib/router.svelte';
import { browser } from '../../lib/stores/browser.svelte';
import { browserLive } from '../../lib/stores/browserLive.svelte';
import { toastError } from '../../lib/toastError';
import type { SimDevice } from '../../lib/api/types';
import { stateForPreset, type DeviceEmulationState } from './live/devicePresets';

export interface DeepLinkCallbacks {
  setUrlInput: (url: string) => void;
  setEmulation: (emu: DeviceEmulationState) => void;
  onSimulator: (dev: SimDevice, split: boolean) => void;
  onSimulatorMissing: () => void;
  /** Whether this Otto client can render a live browser tab (defaults to true). */
  liveCapable?: boolean;
}

function sameHttpUrl(a: string, b: string): boolean {
  if (a === b) return true;
  try {
    return new URL(a).href === new URL(b).href;
  } catch {
    return false;
  }
}

export async function consumeBrowserDeepLink(cb: DeepLinkCallbacks): Promise<void> {
  if (typeof window === 'undefined') return;
  const link = parseBrowserDeepLink(window.location.hash);
  if (!link) return;
  try {
    router.replace('browser');
  } catch {
    /* ignore restricted history */
  }
  const canLive = cb.liveCapable ?? true;
  if (link.presetId) {
    if (nativeBrowserAvailable && browserLive.renderer === 'native') {
      browserLive.setPref('remote');
    }
    cb.setEmulation(stateForPreset(link.presetId));
  }
  const hasWebUrl = !!link.url && isHttpUrl(link.url);
  if (hasWebUrl && link.url) {
    const targetUrl = link.url;
    const wantLive = (link.mode === 'live' || !!link.presetId) && canLive;
    cb.setUrlInput(targetUrl);
    try {
      const existing = Array.isArray(browser.tabs)
        ? browser.tabs.find((t) => sameHttpUrl(t.url, targetUrl))
        : undefined;
      if (existing) {
        browser.select(existing.id);
        if (wantLive && existing.mode !== 'live') {
          await browser.setMode(existing.id, 'live');
        } else if (link.mode === 'reader' && existing.mode !== 'reader') {
          await browser.setMode(existing.id, 'reader');
        }
      } else if (wantLive) {
        await browser.openLiveTab(targetUrl);
      } else if (typeof browser.openTab === 'function') {
        await browser.openTab(targetUrl);
      } else {
        await browser.navigate(targetUrl);
      }
    } catch (e) {
      toastError('Couldn’t open linked URL', e);
    }
  } else if (link.mode && browser.activeTab) {
    const nextMode = link.mode === 'live' && !canLive ? 'reader' : link.mode;
    void browser.setMode(browser.activeTab.id, nextMode);
  }
  if (link.simulatorId) {
    try {
      const dev = await simulatorsApi.getDevice(link.simulatorId);
      cb.onSimulator(dev, hasWebUrl || !!browser.activeTab);
      if (link.url && dev.state === 'booted' && dev.capabilities.open_url) {
        void simulatorsApi
          .openUrl(dev.id, { url: link.url, auto_forward_localhost: true })
          .catch((e) => toastError('Couldn’t open URL on simulator', e));
      }
    } catch (e) {
      toastError('Couldn’t attach simulator', e);
      cb.onSimulatorMissing();
    }
  }
}
