// Live-browser ENGINE state: which renderer a live tab uses on this device,
// and whether the daemon's Chromium is installed. Contract: api.md "Browser —
// remote live view".
//
//   native — the desktop app's per-tab child WKWebView (lib/nativeBrowser.ts).
//            Fast and local, but only exists inside Otto.app.
//   remote — the daemon-owned Chromium, streamed to the Browser page as a
//            screencast (modules/browser/live/RemoteLiveView.svelte). Works in
//            remote/web/PWA sessions too, and is the engine an agent drives
//            visibly.
//
// Off the desktop app the renderer is always `remote`. In the app the user
// picks (per device, remembered); `native` is the default there.
//
// The Chromium download is one-time and never silent: `install()` runs only
// from an explicit click (the "Enable live browsing" step or Settings →
// Browser). Progress arrives as `browser_engine_install_updated` events, with
// a slow poll as a fallback while a job runs.

import * as liveApi from '../api/browserLive';
import { ApiError } from '../api/client';
import type {
  BrowserChromeBuild,
  BrowserEngineBuildStatus,
  BrowserEngineInstallJob,
  BrowserLiveSettings,
  BrowserLiveStatus,
  OttoEvent,
} from '../api/types';
import { nativeBrowserAvailable } from '../nativeBrowser';

export type LiveRenderer = 'native' | 'remote';

/** The engine as the UI presents it. */
export type LiveEngineState = 'unknown' | 'unsupported' | 'missing' | 'installing' | 'failed' | 'ready';

const PREF_KEY = 'otto_browser_live_engine';
const RUNNING: ReadonlyArray<BrowserEngineInstallJob['state']> = ['downloading', 'verifying', 'extracting'];
const POLL_MS = 2000;

/** Fallback sizes for the enable step when the daemon doesn't list a build
 *  (matches the contract's pinned Chrome for Testing downloads). */
export const DEFAULT_DOWNLOAD_BYTES: Record<BrowserChromeBuild, number> = {
  chrome: 180 * 1024 * 1024,
  'chrome-headless-shell': 98 * 1024 * 1024,
};

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

class BrowserLiveStore {
  /** Last `GET /browser/live/status`; null until the first load. */
  status: BrowserLiveStatus | null = $state(null);
  /** False when the daemon predates the remote live view (the status route
   *  404s); null while unknown. The page then keeps the old honest fallback. */
  supported: boolean | null = $state(null);
  loading = $state(false);
  /** Human-readable load failure (daemon unreachable…), shown inline. */
  loadError = $state('');
  /** A POST /install is in flight (the button's own busy state). */
  starting = $state(false);
  /** Per-device renderer preference (only meaningful in the desktop app). */
  pref: LiveRenderer = $state(lsGet(PREF_KEY) === 'remote' ? 'remote' : 'native');
  private pollTimer: ReturnType<typeof setTimeout> | null = null;

  /** The renderer a live tab uses right now on this device. */
  get renderer(): LiveRenderer {
    return nativeBrowserAvailable && this.pref === 'native' ? 'native' : 'remote';
  }

  get engineState(): LiveEngineState {
    const s = this.status;
    if (!s) return 'unknown';
    if (!s.platform_supported) return 'unsupported';
    if (s.install && RUNNING.includes(s.install.state)) return 'installing';
    if (s.builds.some((b) => b.installed)) return 'ready';
    if (s.install?.state === 'failed') return 'failed';
    return 'missing';
  }

  get ready(): boolean {
    return this.engineState === 'ready';
  }

  build(b: BrowserChromeBuild): BrowserEngineBuildStatus | null {
    return this.status?.builds.find((x) => x.build === b) ?? null;
  }

  downloadBytes(b: BrowserChromeBuild): number {
    return this.build(b)?.download_bytes || DEFAULT_DOWNLOAD_BYTES[b];
  }

  setPref(p: LiveRenderer): void {
    this.pref = p;
    lsSet(PREF_KEY, p);
  }

  async load(): Promise<void> {
    this.loading = true;
    try {
      this.status = await liveApi.liveStatus();
      this.supported = true;
      this.loadError = '';
      if (this.engineState === 'installing') this.schedulePoll();
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
  async install(build: BrowserChromeBuild): Promise<void> {
    if (this.starting) return;
    this.starting = true;
    try {
      const job = await liveApi.installEngine(build);
      if (this.status) this.status = { ...this.status, install: job };
      if (job.state === 'installed') await this.load();
      else this.schedulePoll();
    } finally {
      this.starting = false;
    }
  }

  async updateSettings(patch: Partial<BrowserLiveSettings>): Promise<void> {
    const settings = await liveApi.updateLiveSettings(patch);
    if (this.status) this.status = { ...this.status, settings };
  }

  /** `browser_engine_install_updated` from the event stream. */
  applyEvent(ev: Extract<OttoEvent, { type: 'browser_engine_install_updated' }>): void {
    if (!this.status) return;
    const prev = this.status.install;
    const job: BrowserEngineInstallJob = {
      build: ev.build,
      version: ev.version,
      state: ev.state,
      received_bytes: ev.received_bytes,
      total_bytes: ev.total_bytes,
      error: ev.error,
      started_at: prev?.started_at ?? new Date().toISOString(),
      finished_at: RUNNING.includes(ev.state) ? null : new Date().toISOString(),
    };
    this.status = { ...this.status, install: job };
    // The build list (installed flags, paths) only changes on completion.
    if (ev.state === 'installed') void this.load();
  }

  private schedulePoll(): void {
    if (this.pollTimer) return;
    this.pollTimer = setTimeout(async () => {
      this.pollTimer = null;
      try {
        this.status = await liveApi.liveStatus();
      } catch {
        /* transient: keep polling while the last known state says installing */
      }
      if (this.engineState === 'installing') this.schedulePoll();
    }, POLL_MS);
  }
}

export const browserLive = new BrowserLiveStore();
