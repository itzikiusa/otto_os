// `GET /machine/status`, shared by the status-bar machine chip and Settings ▸
// Daemon ▸ Machine resources. Polled only while one of them is mounted
// (`watch()` returns the release), on a calm cadence through the shared
// poller: paused while the window is hidden, no overlapping requests, backed
// off after failures. The load average is a one-minute average, so a faster
// cadence would only re-read the same number.
import { untrack } from 'svelte';
import { api } from '../../lib/api/client';
import { loadErrorText } from '../../lib/loadError';
import { pollWhileVisible, type Poller } from '../../lib/poll';
import type { MachineStatus } from '../../lib/api/types/machine';

export const MACHINE_POLL_MS = 10_000;

class MachineStatusStore {
  status: MachineStatus | null = $state(null);
  /** The last read failed (an older daemon without the route, or offline). */
  error = $state('');
  private poller: Poller | null = null;
  private watchers = 0;

  /** Keep the status fresh while the caller is mounted. Use as
   *  `$effect(() => machineStatus.watch())` — the release is the cleanup. */
  watch(): () => void {
    // Started from effects: nothing read here may become their dependency.
    return untrack(() => {
      this.watchers += 1;
      this.poller ??= pollWhileVisible(
        async (signal) => {
          try {
            this.status = await api.get<MachineStatus>('/machine/status', signal);
            this.error = '';
          } catch (e) {
            if (signal.aborted) return;
            this.error = loadErrorText(e);
            return false;
          }
        },
        { ms: MACHINE_POLL_MS },
      );
      let released = false;
      return () => {
        if (released) return;
        released = true;
        this.watchers -= 1;
        if (this.watchers > 0) return;
        this.poller?.stop();
        this.poller = null;
      };
    });
  }

  /** Read again now (a settings save changed the thresholds). */
  refresh(): void {
    this.poller?.now({ background: true });
  }
}

export const machineStatus = new MachineStatusStore();
