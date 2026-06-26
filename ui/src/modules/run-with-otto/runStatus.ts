// Pure presentation helpers for a run's stage status — shared by the list rows
// and the detail panel. Kept rune-free (a plain .ts) so it's trivially testable
// and importable from any .svelte file.

import type { RunStatus } from '../../lib/api/types';

/** Tone buckets that map onto the page's status-pill CSS classes. */
export type StatusTone = 'ok' | 'bad' | 'warn' | 'active' | 'dim';

const TERMINAL: ReadonlySet<string> = new Set([
  'completed',
  'failed',
  'rejected',
  'cancelled',
]);

/** True for statuses past which no further action runs. */
export function isTerminal(status: RunStatus | string): boolean {
  return TERMINAL.has(status);
}

/** Color bucket: green completed, red failed/rejected, amber awaiting_approval,
 *  blue for any active mid-stage, dim for queued/cancelled. */
export function statusTone(status: RunStatus | string): StatusTone {
  switch (status) {
    case 'completed':
      return 'ok';
    case 'failed':
    case 'rejected':
      return 'bad';
    case 'awaiting_approval':
      return 'warn';
    case 'queued':
    case 'cancelled':
      return 'dim';
    default:
      // resolving_source / building_context / provisioning / executing /
      // proving / reviewing / drafting_pr — all live work.
      return 'active';
  }
}

/** Humanize a snake_case status/kind into a Title-case label. */
export function humanize(s: string): string {
  if (!s) return '';
  return s.replace(/_/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase());
}
