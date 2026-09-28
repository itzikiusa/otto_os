// The daemon's idle-suspend settings for the Agents pane hint (lib/idleSuspend.ts).
// `GET /settings` is root-only; anyone else (and any failure) keeps the
// daemon's compiled-in defaults, which is what an unconfigured daemon uses.
// Loaded once per app session, on the first pane that shows a hint.

import { api } from '../api/client';
import { auth } from './auth.svelte';
import { DEFAULT_IDLE_SUSPEND_POLICY, policyFromSettings, type IdleSuspendPolicy } from '../idleSuspend';

class IdleSuspendStore {
  policy: IdleSuspendPolicy = $state(DEFAULT_IDLE_SUSPEND_POLICY);
  private requested = false;

  /** Kick off the one settings read (no-op after the first call). */
  ensure(): void {
    if (this.requested || !auth.isRoot) return;
    this.requested = true;
    api.bg
      .get<Record<string, unknown>>('/settings')
      .then((all) => {
        this.policy = policyFromSettings(all);
      })
      .catch(() => {
        /* informational only — keep the defaults */
      });
  }
}

export const idleSuspend = new IdleSuspendStore();
