// Live-browser ENGINE state: which renderer a live tab uses on this device,
// and whether the daemon's Chromium is installed.
//
//   native — the desktop app's per-tab child WKWebView (lib/nativeBrowser.ts).
//            Fast and local, but only exists inside Otto.app.
//   otto   — the daemon-run Chromium, streamed to the Browser page as a
//            screencast (modules/browser/live/RemoteLiveView.svelte). Works in
//            remote/web/PWA sessions too, and is the engine an agent can
//            drive visibly.
//
// Off the desktop app the engine is always `otto`. In the app the user picks
// (per device, remembered); `native` is the default there.
//
// The Chromium download is one-time and never silent: `install()` runs only
// from an explicit click (the "Enable live browsing" step or Settings →
// Browser). Progress is polled while an install runs.

import * as liveApi from '../api/browserLive';
import type { LiveEngineKind, LiveEngineStatus } from '../api/browserLive';
import { ApiError } from '../api/client';
import { nativeBrowserAvailable } from '../nativeBrowser';

export type LiveRenderer = 'native' | 'otto';

const PREF_KEY = 'otto_browser_live_engine';

function lsGet(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}
function lsSet(key: string, val: string): void {
  try {
    localStorage.setItem(key, val);
  } catch {
    /* blocked storage: the choice just isn't remembered */
  }
}

const POLL_MS = 800;

class BrowserLiveStore {
  /** Last engine status from the daemon; null until the first load. */
  status: LiveEngineStatus | null = $state(null);
  /** False when the daemon predates live streaming (the status route 404s);
   *  null while unknown. The view then keeps the old honest fallback. */
  supported: boolean | null = $state(null);
  loading = $state(false);
  /** Human-readable load failure (daemon unreachable…), shown inline. */
  loadError = $state('');
  installing = $state(false);
  /** Per-device renderer preference (only meaningful in the desktop app). */
  pref: LiveRenderer = $state(lsGet(PREF_KEY) === 'otto' ? 'otto' : 'native');
  private pollTimer: ReturnType<typeof setTimeout> | null = null;

  /** The renderer a live tab uses right now on this device. */
  get renderer(): LiveRenderer {
    return nativeBrowserAvailable && this.pref === 'native' ? 'native' : 'otto';
  }

  /** The daemon engine is installed and can stream. */
  get ready(): boolean {
    return this.status?.state === 'ready';
  }

  setPref(p: LiveRenderer): void {
    this.pref = p;
    lsSet(PREF_KEY, p);
  }

  async load(): Promise<void> {
    this.loading = true;
    try {
      this.status = await liveApi.engineStatus();
      this.supported = this.status.state !== 'unsupported';
      this.loadError = '';
      if (this.status.state === 'installing') this.schedulePoll();
    } catch (e) {
      if (e instanceof ApiError && (e.status === 404 || e.status === 405)) {
        this.supported = false;
        this.loadError = '';
      } else {
        this.loadError = e instanceof Error ? e.message : String(e);
      }
    } finally {
      this.loading = false;
    }
  }

  /** Start the one-time download. Only ever called from an explicit click. */
  async install(kind: LiveEngineKind): Promise<void> {
    if (this.installing) return;
    this.installing = true;
    try {
      this.status = await liveApi.installEngine(kind);
      this.loadError = '';
      this.schedulePoll();
    } finally {
      this.installing = false;
    }
  }

  async setKind(kind: LiveEngineKind): Promise<void> {
    this.status = await liveApi.updateEngineSettings({ engine: kind });
    if (this.status.state === 'installing') this.schedulePoll();
  }

  async setHeaded(headed: boolean): Promise<void> {
    this.status = await liveApi.updateEngineSettings({ headed });
  }

  private schedulePoll(): void {
    if (this.pollTimer) return;
    this.pollTimer = setTimeout(async () => {
      this.pollTimer = null;
      try {
        this.status = await liveApi.engineStatus();
      } catch {
        /* transient: keep polling while the last known state says installing */
      }
      if (this.status?.state === 'installing') this.schedulePoll();
    }, POLL_MS);
  }
}

export const browserLive = new BrowserLiveStore();
