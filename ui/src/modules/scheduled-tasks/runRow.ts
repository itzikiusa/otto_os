// What one run row of a scheduled task says: its summary as plain text and its
// reason without the daemon's error-kind prefix ("internal: …", "conflict: …").
import { leadLine } from '../../lib/plainText.ts';
import type { ScheduledTaskRun } from '../../lib/api/types';

export function runSummary(r: Pick<ScheduledTaskRun, 'status' | 'summary'>): string {
  if (r.status === 'running') return 'Running…';
  return leadLine(r.summary) || 'No summary';
}

export function runReason(error: string | null | undefined): string {
  return (error ?? '').replace(/^\w+: /, '');
}
