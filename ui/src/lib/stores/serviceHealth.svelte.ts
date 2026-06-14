// Tracks upstream/provider availability inferred from API responses.
//
// The daemon maps git-provider failures (Bitbucket/GitHub/GitLab) to a 502
// (Error::Upstream). 503/504 are treated the same. When we see one we show a
// banner; it auto-hides ~90s after the LAST gateway error (we can't clear on
// an unrelated 200, since most calls are local git, not provider calls).

const HIDE_AFTER_MS = 90_000;

class ServiceHealth {
  /** True while a recent gateway error indicates the provider is unavailable. */
  down = $state(false);
  /** Epoch ms of the most recent gateway error (for "last seen" display). */
  lastDownAt: number | null = $state(null);
  /** User dismissed the current outage banner. */
  private dismissed = $state(false);
  private timer: ReturnType<typeof setTimeout> | null = null;

  /** Called by the API client for every response status. */
  report(status: number): void {
    if (status === 502 || status === 503 || status === 504) {
      if (!this.down) this.dismissed = false; // a fresh outage → show again
      this.down = true;
      this.lastDownAt = Date.now();
      if (this.timer) clearTimeout(this.timer);
      this.timer = setTimeout(() => {
        this.down = false;
      }, HIDE_AFTER_MS);
    }
  }

  dismiss(): void {
    this.dismissed = true;
  }

  /** Whether the outage banner should be shown. */
  get visible(): boolean {
    return this.down && !this.dismissed;
  }
}

export const serviceHealth = new ServiceHealth();
