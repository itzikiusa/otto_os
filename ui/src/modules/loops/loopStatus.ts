import type { StatusInfo } from '../../lib/status.ts';

// A goal loop's lifecycle status in the shared status vocabulary (lib/status.ts,
// patterns.md §1): same tones as a workflow/swarm run (running → info + live,
// failed → danger, stopped → neutral like a cancelled run), but the loop keeps
// its own words — "Blocked" and "Exhausted" tell the user what to do next,
// which the generic "Waiting" would hide.
const LOOP_STATUS: Record<string, Omit<StatusInfo, 'key'>> = {
  draft: { label: 'Draft', tone: 'neutral', hint: 'Not started yet' },
  running: { label: 'Running', tone: 'info', live: true, hint: 'Agents are iterating toward the goal' },
  paused: { label: 'Paused', tone: 'neutral', hint: 'Paused — Resume to continue' },
  blocked: { label: 'Blocked', tone: 'warning', hint: 'Needs you — answer the open decision or retry an agent, then Resume' },
  succeeded: { label: 'Succeeded', tone: 'success', hint: 'Every acceptance criterion is met' },
  exhausted: { label: 'Exhausted', tone: 'warning', hint: 'Hit its iteration or runtime budget before the goal was met' },
  failed: { label: 'Failed', tone: 'danger' },
  stopped: { label: 'Stopped', tone: 'neutral', hint: 'Stopped by a person' },
};

export function loopStatus(raw: string | null | undefined): StatusInfo {
  const k = (raw ?? '').trim().toLowerCase();
  const info = LOOP_STATUS[k];
  if (info) return { key: k, ...info };
  return { key: k || 'unknown', label: k ? k[0].toUpperCase() + k.slice(1) : 'Unknown', tone: 'neutral' };
}
