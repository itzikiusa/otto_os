// Whether a trigger works, in one line under it in the Triggers panel: "Last
// fired 2h ago · started" / "Never fired" / "Last attempt rejected: <reason>".
// Read from the trigger's last_fired_at / last_outcome / last_detail, which the
// daemon stamps on every fire or refusal. Pure — the panel passes its relative
// time formatter (`rel` from stores/now.svelte.ts) so the label ticks.

import type { WorkflowTrigger } from '../../lib/api/types';

export interface TriggerStatusLine {
  text: string;
  /** `warn` for a refused attempt, `dim` for never fired, `ok` otherwise. */
  tone: 'ok' | 'warn' | 'dim';
}

type Stamp = Pick<WorkflowTrigger, 'last_fired_at' | 'last_outcome' | 'last_detail'>;

export function triggerStatusLine(t: Stamp, ago: (iso: string) => string): TriggerStatusLine {
  if (!t.last_fired_at || !t.last_outcome) return { text: 'Never fired', tone: 'dim' };
  if (t.last_outcome === 'rejected') {
    const why = (t.last_detail ?? '').trim();
    return { text: why ? `Last attempt rejected: ${why}` : 'Last attempt rejected', tone: 'warn' };
  }
  const when = ago(t.last_fired_at);
  return { text: `Last fired ${when || 'just now'} · ${t.last_outcome}`, tone: 'ok' };
}
