// Shared built-in mobile SDK / runtime installer store.
// Used by Browser SimulatorPanel, IDE DevicesPanel, and IDE ToolchainsModal.
// Polls via `pollWhileVisible` (never raw setInterval/setTimeout) so hidden
// tabs pause cleanly and catch up immediately on return.

import { simulatorsApi } from '../api/simulators';
import { pollWhileVisible, type Poller } from '../poll';
import { toasts } from '../toast.svelte';
import { toastError } from '../toastError';
import type {
  SimInstallStatus,
  SimInstallTarget,
  SimPlatformReadiness,
} from '../api/types';

/** Codes where `xcodebuild -downloadPlatform iOS` cannot run and the CTA must stay hidden. */
const IOS_INSTALL_BLOCKED_CODES = new Set([
  'xcode_not_installed',
  'xcode_downloading',
  'unsupported_os',
]);

/** True when Otto's built-in "Download iOS Runtime" CTA can succeed for this readiness state. */
export function canOfferIosRuntimeDownload(
  readiness: SimPlatformReadiness | null | undefined,
): boolean {
  if (!readiness || readiness.state === 'ready') return false;
  return !IOS_INSTALL_BLOCKED_CODES.has(readiness.code);
}

/** True when Otto's built-in "Install Android Emulator" CTA should be offered. */
export function canOfferAndroidInstall(
  readiness: SimPlatformReadiness | null | undefined,
): boolean {
  return !!readiness && readiness.state !== 'ready';
}

type CompleteListener = (status: SimInstallStatus) => void;

class SimInstallerStore {
  status: SimInstallStatus | null = $state(null);
  busy = $state(false);
  startingTarget: SimInstallTarget | null = $state(null);
  dismissedAt = $state('');

  private poller: Poller | null = null;
  private wasRunning = false;
  private listeners = new Set<CompleteListener>();

  get running(): boolean {
    return this.status?.running ?? false;
  }

  get visible(): boolean {
    const s = this.status;
    if (!s) return false;
    return s.running || (s.stage !== 'idle' && s.updated_at !== this.dismissedAt);
  }

  /** Subscribe to installation completion (`running` true -> false). Returns unsubscribe fn. */
  onComplete(cb: CompleteListener): () => void {
    this.listeners.add(cb);
    return () => this.listeners.delete(cb);
  }

  private ensurePolling(): void {
    if (this.poller) return;
    this.poller = pollWhileVisible(() => this.pollOnce(), { ms: 1500, maxBackoff: 4 });
  }

  private stopPolling(): void {
    this.poller?.stop();
    this.poller = null;
  }

  private applyStatus(next: SimInstallStatus): void {
    this.status = next;
    if (next.running) {
      this.wasRunning = true;
      this.ensurePolling();
      return;
    }
    this.stopPolling();
    if (this.wasRunning) {
      this.wasRunning = false;
      if (next.stage === 'done') {
        toasts.success('Simulator setup complete', next.message || 'Refreshing available devices.');
      } else if (next.stage === 'cancelled') {
        toasts.info('Installation canceled', next.message || 'Stopped before finishing.');
      }
      for (const cb of this.listeners) {
        try {
          cb(next);
        } catch {
          /* isolate subscriber errors */
        }
      }
    }
  }

  private async pollOnce(): Promise<void> {
    try {
      const next = await simulatorsApi.installStatus();
      this.applyStatus(next);
    } catch {
      /* transient poll error — pollWhileVisible backs off automatically */
    }
  }

  async load(): Promise<SimInstallStatus | null> {
    try {
      const next = await simulatorsApi.installStatus();
      this.applyStatus(next);
      return next;
    } catch {
      return null;
    }
  }

  async start(
    target: SimInstallTarget,
    opts?: { create_default_avds?: boolean },
  ): Promise<SimInstallStatus | null> {
    if (this.busy || this.running) return this.status;
    this.busy = true;
    this.startingTarget = target;
    try {
      const next = await simulatorsApi.startInstall({
        target,
        ...(target === 'android_emulator'
          ? { create_default_avds: opts?.create_default_avds ?? true }
          : {}),
      });
      this.dismissedAt = '';
      this.wasRunning = next.running;
      this.applyStatus(next);
      if (!next.running && next.stage === 'done') {
        for (const cb of this.listeners) {
          try {
            cb(next);
          } catch {
            /* isolate subscriber errors */
          }
        }
      }
      return next;
    } catch (e) {
      toastError('Couldn’t start installation', e);
      return null;
    } finally {
      this.busy = false;
      this.startingTarget = null;
    }
  }

  async cancel(): Promise<SimInstallStatus | null> {
    if (this.busy) return this.status;
    this.busy = true;
    try {
      const next = await simulatorsApi.cancelInstall();
      this.applyStatus(next);
      return next;
    } catch (e) {
      toastError('Couldn’t cancel installation', e);
      return null;
    } finally {
      this.busy = false;
    }
  }

  dismiss(): void {
    if (this.status) {
      this.dismissedAt = this.status.updated_at;
    }
  }
}

export const simInstaller = new SimInstallerStore();
