// Runs the open-tab safety net (lib/stores/tabSweep.ts) against the daemon:
// on window return (installShell, throttled) and after a bulk action (forced).

import { api } from '../lib/api/client';
import type { Session } from '../lib/api/types';
import { ws } from '../lib/stores/workspace.svelte';
import { sweepDue, sweepTabs } from '../lib/stores/tabSweep';

let last = 0;
let running = false;

/** Check the open tabs; `force` skips the once-a-minute throttle (bulk actions). */
export function checkOpenTabs(force = false): void {
  const now = Date.now();
  if (running || !ws.layoutReady || !sweepDue(now, last, force)) return;
  last = now;
  running = true;
  void sweepTabs(ws, (ids) => api.get<Session[]>(`/sessions?ids=${ids.map(encodeURIComponent).join(',')}`))
    .catch(() => [])
    .finally(() => (running = false));
}
