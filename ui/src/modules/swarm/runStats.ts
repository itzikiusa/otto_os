// Swarm run stats in ONE pass over the runs (backlog B6 / SE-04): per-agent
// live/active/done plus swarm totals. The Org tree, Graph and page header used
// to filter the whole run list per agent per helper per render.
import type { SwarmRun } from './types';

/** One pass over the runs: per-agent live/active/done plus swarm totals. */
export interface AgentRunStats {
  /** running + waiting (the Org tree's badge). */
  live: number;
  /** Newest queued/running/waiting run, if any. */
  active: SwarmRun | null;
  /** Finished (`done`) runs. */
  done: number;
}
export interface RunStats {
  byAgent: Map<string, AgentRunStats>;
  /** Task ids that have a queued/running/waiting run. */
  liveTaskIds: Set<string>;
  queued: number;
  running: number;
}
export const EMPTY_AGENT_STATS: AgentRunStats = { live: 0, active: null, done: 0 };

export function computeRunStats(runs: readonly SwarmRun[]): RunStats {
  const byAgent = new Map<string, AgentRunStats>();
  const liveTaskIds = new Set<string>();
  let queued = 0;
  let running = 0;
  for (const r of runs) {
    let st = byAgent.get(r.agent_id);
    if (!st) {
      st = { live: 0, active: null, done: 0 };
      byAgent.set(r.agent_id, st);
    }
    const isLive = r.status === 'running' || r.status === 'waiting' || r.status === 'queued';
    if (r.status === 'queued') queued += 1;
    if (r.status === 'running' || r.status === 'waiting') {
      running += 1;
      st.live += 1;
    }
    if (r.status === 'done') st.done += 1;
    if (isLive) {
      if (r.task_id) liveTaskIds.add(r.task_id);
      if (!st.active || new Date(r.enqueued_at).getTime() > new Date(st.active.enqueued_at).getTime()) st.active = r;
    }
  }
  return { byAgent, liveTaskIds, queued, running };
}
