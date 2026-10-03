// Incremental Activity feed (perf W4). The tab used to re-GET the whole
// `/personal-agents/{id}/activity` payload (20 full run rows + the ring + one
// approval read per waiting item) on EVERY agent tool call and on every
// approval change anywhere in the workspace. Now it asks only for ring
// entries after its cursor (`after_seq`) and for runs only when a run
// changed, and merges the answer here.
import type { PersonalAgentActivity } from '../../lib/api/types';

/** Ring entries the client keeps (the daemon's per-agent cap). */
export const ACTIVITY_ITEM_CAP = 200;

/** Merge an incremental answer into the current state. `items` are prepended
 *  (newest first, deduped by `seq`, capped); `now`/`approvals`/`seq` are
 *  replaced; `runs` is replaced only when the answer carries them. */
export function mergeActivity(
  prev: PersonalAgentActivity | null,
  next: PersonalAgentActivity,
): PersonalAgentActivity {
  if (!prev) return { ...next, runs: next.runs ?? [] };
  const seen = new Set(prev.items.map((i) => i.seq));
  const fresh = next.items.filter((i) => !seen.has(i.seq));
  const items = [...fresh, ...prev.items].sort((a, b) => b.seq - a.seq).slice(0, ACTIVITY_ITEM_CAP);
  return {
    now: next.now,
    items,
    approvals: next.approvals,
    runs: next.runs ?? prev.runs,
    seq: Math.max(prev.seq ?? 0, next.seq ?? 0),
  };
}

/** Does an `mcp_approval_changed` event concern this agent's feed? Only when
 *  it names an approval the feed shows (a new waiting approval arrives as a
 *  `personal_agent_activity` event instead). */
export function approvalConcernsFeed(
  data: PersonalAgentActivity | null,
  approvalId: unknown,
): boolean {
  if (!data || typeof approvalId !== 'string') return false;
  return data.approvals.some((a) => a.approval_id === approvalId);
}
