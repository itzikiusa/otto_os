// The saved default a new thread's access starts from: Settings › Providers ›
// "Skip permission prompts" (`agent_skip_permissions`, on by default) is the
// daemon's default too — on means Full access, off means Supervised. `GET
// /settings` needs Settings admin; anyone else (and any failure) gets null,
// which the stage shows as Supervised and sends explicitly, so what the chip
// says is what the first turn runs with. Read once per app session.
import { api } from '../../lib/api/client';
import { auth } from '../../lib/stores/auth.svelte';

class ThreadAccessDefault {
  /** True = the saved default is Full access; null = unknown (not readable). */
  skipPermissions = $state<boolean | null>(null);
  private requested = false;

  ensure(): void {
    if (this.requested || !auth.isRoot) return;
    this.requested = true;
    api.bg
      .get<Record<string, unknown>>('/settings')
      .then((all) => {
        const v = all['agent_skip_permissions'];
        this.skipPermissions = typeof v === 'boolean' ? v : true;
      })
      .catch(() => {
        /* unknown: the stage starts Supervised */
      });
  }
}

export const threadAccessDefault = new ThreadAccessDefault();
