// GET /usage/limits, shared: the Usage page's "Remaining" and the Threads
// sidebar's meter read the same answer from ONE poller (60 s while the page is
// visible — the cadence "Remaining" always had), which runs while either is
// on screen.
import { api } from '../api/client';
import type { LimitsResp } from '../api/types/usage-limits';
import { loadErrorText } from '../loadError';
import { pollWhileVisible, type Poller } from '../poll';

class LimitsStore {
  data = $state<LimitsResp | null>(null);
  error = $state<string | null>(null);
  loading = $state(true);
  /** The clock "resets in" and the stale verdict are read against (moves with every poll). */
  now = $state(Date.now());
  private poller: Poller | null = null;
  private refs = 0;

  /** Keep polling while mounted; returns the release. */
  mount(): () => void {
    if (this.refs++ === 0) {
      this.poller = pollWhileVisible(
        async (signal) => {
          try {
            this.data = await api.get<LimitsResp>('/usage/limits', signal);
            this.error = null;
          } catch (e) {
            if (signal.aborted) return;
            this.error = loadErrorText(e);
            return false;
          } finally {
            this.loading = false;
            this.now = Date.now();
          }
        },
        { ms: 60_000 },
      );
    }
    return () => {
      if (--this.refs === 0) {
        this.poller?.stop();
        this.poller = null;
      }
    };
  }

  refresh(): void {
    this.loading = true;
    this.poller?.now();
  }

  /** A fresher answer from another call (a Claude check). */
  set(resp: LimitsResp): void {
    this.data = resp;
    this.error = null;
    this.now = Date.now();
  }
}

export const limits = new LimitsStore();
